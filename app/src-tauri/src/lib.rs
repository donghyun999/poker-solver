//! 데스크탑 앱 백엔드: 엔진을 Tauri 명령으로 감싼다.
//!
//! - 무거운 작업(트리 생성, 솔브, 조회)은 spawn_blocking으로 워커 스레드에서 실행
//! - 솔브 진행률은 "solve-progress" 이벤트로 프론트엔드에 push
//! - 결과(Solver)는 앱 상태에 보관하고, 화면에서 보는 노드만 요약해서 보냄

use engine::card::{card_to_string, parse_card, parse_cards};
use engine::{Game, NodeKind, Range, SolveConfig, Solver, Step, TreeConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDto {
    board: String,
    oop_range: String,
    ip_range: String,
    pot: f64,
    stack: f64,
    /// 팟 대비 %, 쉼표 구분. 예: "33,75"
    flop_bet: String,
    turn_bet: String,
    river_bet: String,
    raise: String,
    max_raises: u32,
    allow_allin: bool,
    compress: bool,
}

fn parse_sizes(s: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| x.trim_end_matches('%').parse::<f64>().map(|v| v / 100.0).map_err(|_| format!("사이즈 오류: {x}")))
        .collect()
}

fn sizes_to_string(v: &[f64]) -> String {
    v.iter().map(|x| format!("{}", (x * 1000.0).round() / 10.0)).collect::<Vec<_>>().join(",")
}

impl ConfigDto {
    fn to_config(&self) -> Result<SolveConfig, String> {
        let board = parse_cards(&self.board)?;
        let oop = Range::parse(&self.oop_range).map_err(|e| format!("OOP 레인지: {e}"))?;
        let ip = Range::parse(&self.ip_range).map_err(|e| format!("IP 레인지: {e}"))?;
        let raise = parse_sizes(&self.raise)?;
        let mut tree = TreeConfig::uniform(self.pot, self.stack, &[], &raise);
        for (i, s) in [&self.flop_bet, &self.turn_bet, &self.river_bet].iter().enumerate() {
            let b = parse_sizes(s)?;
            for p in 0..2 {
                tree.sizes[i][p].bet = b.clone();
            }
        }
        tree.max_raises = self.max_raises;
        tree.allow_allin = self.allow_allin;
        Ok(SolveConfig { board, ranges: [oop, ip], tree })
    }

    /// 불러온 결과에서 폼을 다시 채우기 위한 역변환 (레인지는 콤보 목록으로)
    fn from_config(c: &SolveConfig, compress: bool) -> ConfigDto {
        let range_str = |r: &Range| {
            let mut parts = Vec::new();
            for idx in 0..engine::card::NUM_COMBOS {
                let w = r.weights[idx];
                if w > 0.0 {
                    let (a, b) = engine::card::combo_cards(idx);
                    let s = format!("{}{}", card_to_string(b), card_to_string(a));
                    parts.push(if w < 1.0 { format!("{s}:{w}") } else { s });
                }
            }
            parts.join(",")
        };
        ConfigDto {
            board: c.board.iter().map(|&x| card_to_string(x)).collect::<Vec<_>>().join(" "),
            oop_range: range_str(&c.ranges[0]),
            ip_range: range_str(&c.ranges[1]),
            pot: c.tree.starting_pot,
            stack: c.tree.effective_stack,
            flop_bet: sizes_to_string(&c.tree.sizes[0][0].bet),
            turn_bet: sizes_to_string(&c.tree.sizes[1][0].bet),
            river_bet: sizes_to_string(&c.tree.sizes[2][0].bet),
            raise: sizes_to_string(&c.tree.sizes[0][0].raise),
            max_raises: c.tree.max_raises,
            allow_allin: c.tree.allow_allin,
            compress,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateDto {
    oop_combos: usize,
    ip_combos: usize,
    action_nodes: usize,
    chance_nodes: usize,
    terminal_nodes: usize,
    memory_mb: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    iteration: u32,
    exploitability: f64,
    elapsed_secs: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveDoneDto {
    iteration: u32,
    exploitability: f64,
    elapsed_secs: f64,
    ev: [f64; 2],
    cancelled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassDto {
    class: String,
    combos: f64,
    freqs: Vec<f64>,
    ev: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDto {
    kind: &'static str,
    player: u8,
    board: Vec<String>,
    pot: f64,
    actions: Vec<String>,
    overall: Vec<f64>,
    total_combos: f64,
    classes: Vec<ClassDto>,
    dealable: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedDto {
    config: ConfigDto,
    iteration: u32,
    exploitability: f64,
    ev: [f64; 2],
}

/// 경로 한 걸음: 숫자 = 액션 번호, 문자열 = 카드 ("Ts")
#[derive(Deserialize)]
#[serde(untagged)]
pub enum StepDto {
    Action(usize),
    Card(String),
}

#[derive(Default)]
pub struct AppState {
    solver: Mutex<Option<Arc<Solver>>>,
    cancel: Arc<AtomicBool>,
    busy: AtomicBool,
}

fn current(state: &AppState) -> Result<Arc<Solver>, String> {
    state.solver.lock().unwrap().clone().ok_or_else(|| "아직 솔브 결과가 없음".to_string())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn estimate(config: ConfigDto) -> Result<EstimateDto, String> {
    blocking(move || {
        let game = Game::new(&config.to_config()?)?;
        let t = &game.tree;
        Ok(EstimateDto {
            oop_combos: game.hands[0].len(),
            ip_combos: game.hands[1].len(),
            action_nodes: t.count(NodeKind::Action),
            chance_nodes: t.count(NodeKind::Chance),
            terminal_nodes: t.count(NodeKind::Fold) + t.count(NodeKind::Showdown),
            memory_mb: game.memory_bytes(config.compress) as f64 / 1e6,
        })
    })
    .await
}

#[tauri::command]
async fn solve(
    app: AppHandle,
    state: State<'_, AppState>,
    config: ConfigDto,
    max_iterations: u32,
    target_exploitability: f64,
    max_memory_gb: f64,
) -> Result<SolveDoneDto, String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("이미 솔브 중".into());
    }
    // 이전 결과의 메모리를 먼저 해제
    *state.solver.lock().unwrap() = None;
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let result = blocking(move || {
        let game = Game::new(&config.to_config()?)?;
        let mem = game.memory_bytes(config.compress) as f64;
        if mem > max_memory_gb * 1e9 {
            return Err(format!(
                "필요 메모리 {:.1} GB가 상한 {max_memory_gb} GB를 넘습니다. 벳 사이즈를 줄이거나 압축을 켜세요",
                mem / 1e9
            ));
        }
        let mut solver = Solver::with_compression(game, config.compress);
        let check_every = if solver.game.tree.count(NodeKind::Chance) > 1000 { 10 } else { 25 };
        let fin = solver.run(max_iterations, target_exploitability, check_every, &cancel, |p| {
            let _ = app.emit(
                "solve-progress",
                ProgressDto { iteration: p.iteration, exploitability: p.exploitability, elapsed_secs: p.elapsed_secs },
            );
        });
        let ev = solver.expected_values();
        let done = SolveDoneDto {
            iteration: fin.iteration,
            exploitability: fin.exploitability,
            elapsed_secs: fin.elapsed_secs,
            ev,
            cancelled: cancel.load(Ordering::SeqCst),
        };
        Ok((solver, done))
    })
    .await;

    state.busy.store(false, Ordering::SeqCst);
    let (solver, done) = result?;
    *state.solver.lock().unwrap() = Some(Arc::new(solver));
    Ok(done)
}

#[tauri::command]
fn cancel(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn node(state: State<'_, AppState>, path: Vec<StepDto>) -> Result<NodeDto, String> {
    let solver = current(&state)?;
    blocking(move || {
        let steps = path
            .into_iter()
            .map(|s| match s {
                StepDto::Action(a) => Ok(Step::Action(a)),
                StepDto::Card(c) => parse_card(&c).map(Step::Card),
            })
            .collect::<Result<Vec<_>, String>>()?;
        let s = solver.summarize(&steps)?;
        let cards = |v: &[u8]| v.iter().map(|&c| card_to_string(c)).collect();
        Ok(NodeDto {
            kind: match s.kind {
                NodeKind::Action => "action",
                NodeKind::Chance => "chance",
                NodeKind::Fold => "fold",
                NodeKind::Showdown => "showdown",
            },
            player: s.player,
            board: cards(&s.board),
            pot: s.pot,
            actions: s.actions,
            overall: s.overall,
            total_combos: s.total_combos,
            classes: s
                .classes
                .into_iter()
                .map(|c| ClassDto { class: c.class, combos: c.combos, freqs: c.freqs, ev: c.ev })
                .collect(),
            dealable: cards(&s.dealable),
        })
    })
    .await
}

#[tauri::command]
async fn save(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let solver = current(&state)?;
    blocking(move || solver.save(&path)).await
}

#[tauri::command]
async fn load(state: State<'_, AppState>, path: String) -> Result<LoadedDto, String> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("솔브 중에는 불러올 수 없음".into());
    }
    *state.solver.lock().unwrap() = None;
    let solver = blocking(move || Solver::load(&path)).await?;
    let loaded = LoadedDto {
        config: ConfigDto::from_config(&solver.game.config, solver.is_compressed()),
        iteration: solver.iteration,
        exploitability: solver.exploitability(),
        ev: solver.expected_values(),
    };
    *state.solver.lock().unwrap() = Some(Arc::new(solver));
    Ok(loaded)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![estimate, solve, cancel, node, save, load])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
