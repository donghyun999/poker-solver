//! 엔진을 터미널에서 돌려보는 실행파일.
//!
//! 예:
//!   cargo run --release -p cli -- --board "Kh Qd 7c 3s 2h" --oop "77,54s" --ip "88" \
//!       --pot 10 --stack 100 --bet 100 --raise 100
//!
//! 도움말: cargo run --release -p cli -- --help

use engine::card::{card_to_string, cards_to_string, parse_card, parse_cards};
use engine::{Game, NodeKind, Range, SolveConfig, Solver, Step, StreetSizes, TreeConfig};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

const HELP: &str = "\
솔버 CLI

필수
  --board <카드>      보드 3~5장. 예: \"Kh Qd 7c\"
  --oop <레인지>      OOP 레인지. 예: \"AA,KK,AKs,T9s+\"
  --ip <레인지>       IP 레인지

게임 설정
  --pot <칩>          시작 팟 (기본 10)
  --stack <칩>        유효 스택 (기본 100)
  --bet <%목록>       모든 스트리트 벳 사이즈, 팟 대비 %. 예: 33,75 (기본 50,100)
  --raise <%목록>     모든 스트리트 레이즈 사이즈. 예: 100 (기본 100)
  --flop-bet, --turn-bet, --river-bet, --flop-raise, --turn-raise, --river-raise
                      스트리트별로 덮어쓰기 (빈 값 \"\" = 그 스트리트 벳 없음)
  --max-raises <n>    스트리트당 최대 레이즈 (기본 2)
  --no-allin          올인 선택지 추가 안 함 (사이즈가 스택을 넘으면 여전히 올인)

솔브 설정
  --iters <n>         최대 반복 (기본 1000)
  --target <%>        목표 익스플로이터빌리티, 팟 대비 % (기본 0.5)
  --check-every <n>   익스플로이터빌리티 계산 간격 (기본 10)
  --threads <n>       스레드 수 (기본: 전체 코어)
  --max-mem <GB>      메모리 상한 (기본 8)
  --compress          16비트 압축 저장 (메모리 약 절반, 플랍 추천)

저장/불러오기
  --save <파일>       솔브 결과를 파일로 저장
  --load <파일>       저장한 결과를 불러와서 바로 조회 (솔브 안 함, 보드·레인지 불필요)

결과 보기
  --line <경로>       루트에서 내려갈 경로. 액션 번호와 카드를 쉼표로.
                      예: \"0,1\" = OOP 0번 액션 → IP 1번 액션
                          \"1,1,Ts,0\" = 벳 → 콜 → 턴 Ts → OOP 0번 액션
";

struct Args(HashMap<String, String>);

impl Args {
    fn parse() -> Result<Args, String> {
        let mut map = HashMap::new();
        let mut it = std::env::args().skip(1);
        while let Some(key) = it.next() {
            let Some(name) = key.strip_prefix("--") else {
                return Err(format!("알 수 없는 인자: {key}"));
            };
            let flag = matches!(name, "no-allin" | "help" | "compress");
            let val = if flag { String::new() } else { it.next().ok_or(format!("{key} 값이 없음"))? };
            map.insert(name.to_string(), val);
        }
        Ok(Args(map))
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(|s| s.as_str())
    }

    fn require(&self, k: &str) -> Result<&str, String> {
        self.get(k).ok_or(format!("--{k} 필요 (--help 참고)"))
    }

    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        match self.get(k) {
            Some(v) => v.parse().map_err(|_| format!("--{k} 숫자 오류: {v}")),
            None => Ok(default),
        }
    }
}

fn parse_sizes(s: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| x.trim_end_matches('%').parse::<f64>().map(|v| v / 100.0).map_err(|_| format!("사이즈 오류: {x}")))
        .collect()
}

fn build_config(args: &Args) -> Result<SolveConfig, String> {
    let board = parse_cards(args.require("board")?)?;
    let oop = Range::parse(args.require("oop")?).map_err(|e| format!("OOP 레인지: {e}"))?;
    let ip = Range::parse(args.require("ip")?).map_err(|e| format!("IP 레인지: {e}"))?;

    let bet = parse_sizes(args.get("bet").unwrap_or("50,100"))?;
    let raise = parse_sizes(args.get("raise").unwrap_or("100"))?;
    let mut tree = TreeConfig::uniform(args.num("pot", 10.0)?, args.num("stack", 100.0)?, &bet, &raise);
    for (i, street) in ["flop", "turn", "river"].iter().enumerate() {
        let b = args.get(&format!("{street}-bet")).map(parse_sizes).transpose()?;
        let r = args.get(&format!("{street}-raise")).map(parse_sizes).transpose()?;
        for p in 0..2 {
            let s: &mut StreetSizes = &mut tree.sizes[i][p];
            if let Some(b) = &b {
                s.bet = b.clone();
            }
            if let Some(r) = &r {
                s.raise = r.clone();
            }
        }
    }
    tree.max_raises = args.num("max-raises", 2)?;
    tree.allow_allin = args.get("no-allin").is_none();
    Ok(SolveConfig { board, ranges: [oop, ip], tree })
}

fn parse_line(s: &str) -> Result<Vec<Step>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| match x.parse::<usize>() {
            Ok(n) => Ok(Step::Action(n)),
            Err(_) => parse_card(x).map(Step::Card),
        })
        .collect()
}

fn pct(x: f64) -> String {
    format!("{:5.1}%", x * 100.0)
}

fn print_node(s: &Solver, path: &[Step]) -> Result<(), String> {
    let sum = s.summarize(path)?;
    let who = ["OOP", "IP"];

    println!();
    println!("── 노드 ─────────────────────────────────");
    let path_str: Vec<String> = path
        .iter()
        .map(|st| match st {
            Step::Action(a) => a.to_string(),
            Step::Card(c) => card_to_string(*c),
        })
        .collect();
    println!("경로 : {}", if path.is_empty() { "(루트)".into() } else { path_str.join(",") });
    println!("보드 : {}   팟 {:.2}", cards_to_string(&sum.board), sum.pot);

    match sum.kind {
        NodeKind::Fold => {
            println!("종료 : {} 폴드", who[sum.player as usize]);
            return Ok(());
        }
        NodeKind::Showdown => {
            println!("종료 : 쇼다운");
            return Ok(());
        }
        NodeKind::Chance => {
            println!("카드 딜 차례: --line 끝에 카드를 붙이세요 (예: \"{},Ts\")", path_str.join(","));
            return Ok(());
        }
        NodeKind::Action => {}
    }

    println!("차례 : {}", who[sum.player as usize]);
    println!(
        "액션 : {}",
        sum.actions.iter().enumerate().map(|(i, l)| format!("{i}) {l}")).collect::<Vec<_>>().join("   ")
    );
    if sum.total_combos <= 0.0 {
        println!("이 노드에 도달하는 콤보가 없음");
        return Ok(());
    }
    print!("전체 :");
    for (l, f) in sum.actions.iter().zip(&sum.overall) {
        print!("  {} {}", l, pct(*f));
    }
    println!("   (콤보 {:.1})", sum.total_combos);

    let has_ev = sum.classes.iter().any(|c| c.ev.is_some());
    println!();
    print!("{:<6}{:>7}", "Hand", "Combos");
    for l in &sum.actions {
        print!("{:>12}", l);
    }
    if has_ev {
        print!("{:>9}", "EV");
    }
    println!();
    for row in &sum.classes {
        print!("{:<6}{:>7.2}", row.class, row.combos);
        for f in &row.freqs {
            print!("{:>12}", pct(*f));
        }
        if let Some(ev) = row.ev {
            print!("{:>9.2}", ev);
        }
        println!();
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args = Args::parse()?;
    if args.get("help").is_some() || std::env::args().len() == 1 {
        print!("{HELP}");
        return Ok(());
    }
    if let Some(t) = args.get("threads") {
        let t: usize = t.parse().map_err(|_| "--threads 숫자 오류")?;
        rayon::ThreadPoolBuilder::new().num_threads(t).build_global().map_err(|e| e.to_string())?;
    }

    let path = parse_line(args.get("line").unwrap_or(""))?;
    println!("solver engine v{}", engine::VERSION);

    if let Some(file) = args.get("load") {
        let t0 = std::time::Instant::now();
        let solver = Solver::load(file)?;
        let c = &solver.game.config;
        println!(
            "불러옴: {file}  ({:.1}초)\n보드 {}   시작팟 {}   스택 {}   반복 {}",
            t0.elapsed().as_secs_f64(),
            cards_to_string(&c.board),
            c.tree.starting_pot,
            c.tree.effective_stack,
            solver.iteration
        );
        return print_node(&solver, &path);
    }

    let cfg = build_config(&args)?;
    println!(
        "보드 {}   시작팟 {}   스택 {}",
        cards_to_string(&cfg.board),
        cfg.tree.starting_pot,
        cfg.tree.effective_stack
    );

    let compress = args.get("compress").is_some();
    let t0 = std::time::Instant::now();
    let game = Game::new(&cfg)?;
    let mem = game.memory_bytes(compress) as f64;
    let t = &game.tree;
    println!(
        "콤보 OOP {} / IP {}   트리: 액션 {} · 찬스 {} · 종료 {} 노드   메모리 {:.1} MB{}   (생성 {:.2}초)",
        game.hands[0].len(),
        game.hands[1].len(),
        t.count(NodeKind::Action),
        t.count(NodeKind::Chance),
        t.count(NodeKind::Fold) + t.count(NodeKind::Showdown),
        mem / 1e6,
        if compress { " (압축)" } else { "" },
        t0.elapsed().as_secs_f64()
    );
    let max_mem: f64 = args.num("max-mem", 8.0)?;
    if mem > max_mem * 1e9 {
        let hint = if compress { "" } else { " 또는 --compress" };
        return Err(format!(
            "메모리 {:.1} GB가 상한 {max_mem} GB를 넘음. 벳 사이즈를 줄이거나{hint} --max-mem을 올리세요",
            mem / 1e9
        ));
    }

    let mut solver = Solver::with_compression(game, compress);
    let cancel = AtomicBool::new(false);
    let fin = solver.run(
        args.num("iters", 1000)?,
        args.num("target", 0.5)?,
        args.num("check-every", 10)?,
        &cancel,
        |p| println!("반복 {:>5}   익스플로이터빌리티 {:>7.3}%   {:>7.1}초", p.iteration, p.exploitability, p.elapsed_secs),
    );
    let ev = solver.expected_values();
    println!(
        "완료: 반복 {}, 익스플로이터빌리티 {:.3}%, {:.1}초   EV OOP {:.3} / IP {:.3}",
        fin.iteration, fin.exploitability, fin.elapsed_secs, ev[0], ev[1]
    );
    if let Some(file) = args.get("save") {
        solver.save(file)?;
        println!("저장함: {file}");
    }

    print_node(&solver, &path)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("오류: {e}");
        std::process::exit(1);
    }
}
