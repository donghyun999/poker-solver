//! 솔버 본체: 벡터 방식 DCFR + 익스플로이터빌리티.
//!
//! ## Kuhn 코드와 다른 점
//! 트리를 한 번 내려갈 때 **한 플레이어의 레인지 전체(콤보 벡터)**를 들고 간다.
//! - 내려갈 때: 상대 콤보별 도달 확률(reach) 벡터
//! - 올라올 때: 내 콤보별 가치(counterfactual value) 벡터
//!
//! ## 손익 기준 (EV 단위: 칩)
//! - 폴드: 이긴 쪽 = 시작팟 + 진 쪽이 넣은 칩, 진 쪽 = −자기가 넣은 칩
//! - 쇼다운: 이긴 쪽 = 시작팟 + 상대가 넣은 칩, 진 쪽 = −자기가 넣은 칩, 무승부 = 시작팟/2
//! - 그래서 두 사람 EV 합은 항상 시작팟
//!
//! ## 데이터 배치
//! regret과 누적 전략은 큰 `Vec<f32>` 두 개. 액션 노드마다
//! `[액션0: 콤보0..n, 액션1: 콤보0..n, …]` 순서로 `data_offset`부터 저장.

use crate::card::*;
use crate::eval::evaluate;
use crate::range::Range;
use crate::tree::{self, NodeKind, Tree, TreeConfig};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const NONE: u32 = u32::MAX;

/// 솔브 입력
#[derive(Clone, Debug)]
pub struct SolveConfig {
    pub board: Vec<Card>,
    /// [OOP 레인지, IP 레인지]
    pub ranges: [Range; 2],
    pub tree: TreeConfig,
}

/// 리버 보드 하나에 대한 콤보별 핸드 강도
struct ShowdownData {
    strength: [Vec<u32>; 2],
    /// 보드와 안 겹치는 콤보 인덱스를 강도 오름차순으로
    order: [Vec<u32>; 2],
}

/// 트리 + 레인지 + 쇼다운 정보. 한 번 만들면 바뀌지 않는다.
pub struct Game {
    pub tree: Tree,
    /// 플레이어별 콤보 목록 (보드와 겹치거나 가중치 0인 콤보는 제외)
    pub hands: [Vec<(Card, Card)>; 2],
    pub init_weights: [Vec<f32>; 2],
    /// 상대 목록에서 같은 콤보의 인덱스 (없으면 NONE)
    same_hand: [Vec<u32>; 2],
    showdowns: Vec<ShowdownData>,
    storage_len: usize,
}

impl Game {
    pub fn new(cfg: &SolveConfig) -> Result<Game, String> {
        let mut tree = tree::build(&cfg.tree, &cfg.board)?;
        let board_mask = mask_of(&cfg.board);

        let mut hands: [Vec<(Card, Card)>; 2] = Default::default();
        let mut init_weights: [Vec<f32>; 2] = Default::default();
        for p in 0..2 {
            for idx in 0..NUM_COMBOS {
                let w = cfg.ranges[p].weights[idx];
                let (a, b) = combo_cards(idx);
                if w > 0.0 && board_mask & (card_bit(a) | card_bit(b)) == 0 {
                    hands[p].push((a, b));
                    init_weights[p].push(w);
                }
            }
            if hands[p].is_empty() {
                let who = if p == 0 { "OOP" } else { "IP" };
                return Err(format!("{who} 레인지에 보드와 안 겹치는 콤보가 없음"));
            }
        }

        let mut same_hand: [Vec<u32>; 2] = Default::default();
        for p in 0..2 {
            let mut pos = vec![NONE; NUM_COMBOS];
            for (i, &(a, b)) in hands[1 - p].iter().enumerate() {
                pos[combo_index(a, b)] = i as u32;
            }
            same_hand[p] = hands[p].iter().map(|&(a, b)| pos[combo_index(a, b)]).collect();
        }

        // 액션 노드 데이터 위치, 쇼다운 보드 번호 배정
        let mut offset = 0;
        let mut board_ids: HashMap<u64, u32> = HashMap::new();
        let mut showdown_boards: Vec<u64> = Vec::new();
        for node in &mut tree.nodes {
            match node.kind {
                NodeKind::Action => {
                    node.data_offset = offset;
                    offset += node.actions.len() * hands[node.player as usize].len();
                }
                NodeKind::Showdown => {
                    let next = board_ids.len() as u32;
                    let id = *board_ids.entry(node.board).or_insert_with(|| {
                        showdown_boards.push(node.board);
                        next
                    });
                    node.showdown_id = id;
                }
                _ => {}
            }
        }

        let showdowns = showdown_boards
            .par_iter()
            .map(|&mask| {
                let board: Vec<Card> = (0..52u8).filter(|&c| mask & card_bit(c) != 0).collect();
                let mut data = ShowdownData { strength: Default::default(), order: Default::default() };
                for p in 0..2 {
                    let mut cards = board.clone();
                    cards.extend([0, 0]);
                    let strength: Vec<u32> = hands[p]
                        .iter()
                        .map(|&(a, b)| {
                            if mask & (card_bit(a) | card_bit(b)) != 0 {
                                0
                            } else {
                                cards[5] = a;
                                cards[6] = b;
                                evaluate(&cards)
                            }
                        })
                        .collect();
                    let mut order: Vec<u32> = (0..hands[p].len() as u32)
                        .filter(|&i| {
                            let (a, b) = hands[p][i as usize];
                            mask & (card_bit(a) | card_bit(b)) == 0
                        })
                        .collect();
                    order.sort_by_key(|&i| strength[i as usize]);
                    data.strength[p] = strength;
                    data.order[p] = order;
                }
                data
            })
            .collect();

        Ok(Game { tree, hands, init_weights, same_hand, showdowns, storage_len: offset })
    }

    pub fn starting_pot(&self) -> f64 {
        self.tree.config.starting_pot
    }

    /// regret + 누적 전략 저장에 필요한 메모리 (바이트)
    pub fn memory_bytes(&self) -> usize {
        self.storage_len * 2 * std::mem::size_of::<f32>()
    }

    /// 상대 reach의 합과 카드별 합 (카드 겹침 계산용)
    fn reach_sums(&self, o: usize, reach: &[f32]) -> (f64, [f64; 52]) {
        let mut total = 0.0;
        let mut per_card = [0.0f64; 52];
        for (i, &(a, b)) in self.hands[o].iter().enumerate() {
            let r = reach[i] as f64;
            total += r;
            per_card[a as usize] += r;
            per_card[b as usize] += r;
        }
        (total, per_card)
    }

    /// 내 콤보 h와 카드가 안 겹치는 상대 reach 합
    #[inline]
    fn compatible(&self, p: usize, h: usize, reach: &[f32], total: f64, per_card: &[f64; 52]) -> f64 {
        let (a, b) = self.hands[p][h];
        let same = match self.same_hand[p][h] {
            NONE => 0.0,
            i => reach[i as usize] as f64,
        };
        total - per_card[a as usize] - per_card[b as usize] + same
    }

    /// 양쪽 초기 레인지에서 카드가 안 겹치는 조합의 가중치 합
    pub fn pair_weight(&self) -> f64 {
        let (total, per_card) = self.reach_sums(1, &self.init_weights[1]);
        (0..self.hands[0].len())
            .map(|h| self.init_weights[0][h] as f64 * self.compatible(0, h, &self.init_weights[1], total, &per_card))
            .sum()
    }

    fn fold_value(&self, node: &tree::Node, p: usize, reach: &[f32]) -> Vec<f32> {
        let folder = node.player as usize;
        let payoff = if folder == p {
            -node.commits[p]
        } else {
            self.starting_pot() + node.commits[folder]
        };
        let (total, per_card) = self.reach_sums(1 - p, reach);
        (0..self.hands[p].len())
            .map(|h| (payoff * self.compatible(p, h, reach, total, &per_card)) as f32)
            .collect()
    }

    /// 쇼다운 가치. 강도순 정렬 + 누적합으로 O(n)
    fn showdown_value(&self, node: &tree::Node, p: usize, reach: &[f32]) -> Vec<f32> {
        let o = 1 - p;
        let sd = &self.showdowns[node.showdown_id as usize];
        let win_pay = self.starting_pot() + node.commits[o];
        let lose_pay = -node.commits[p];
        let tie_pay = self.starting_pot() / 2.0;

        let n = self.hands[p].len();
        let (my_order, opp_order) = (&sd.order[p], &sd.order[o]);
        let (my_str, opp_str) = (&sd.strength[p], &sd.strength[o]);
        let mut win = vec![0.0f64; n];
        let mut lose = vec![0.0f64; n];

        // 이기는 쪽: 나보다 약한 상대 reach 누적 (오름차순)
        let mut cum = 0.0;
        let mut cum_card = [0.0f64; 52];
        let mut j = 0;
        for &h in my_order {
            let s = my_str[h as usize];
            while j < opp_order.len() && opp_str[opp_order[j] as usize] < s {
                let oi = opp_order[j] as usize;
                let (a, b) = self.hands[o][oi];
                let r = reach[oi] as f64;
                cum += r;
                cum_card[a as usize] += r;
                cum_card[b as usize] += r;
                j += 1;
            }
            let (a, b) = self.hands[p][h as usize];
            win[h as usize] = cum - cum_card[a as usize] - cum_card[b as usize];
        }

        // 지는 쪽: 나보다 강한 상대 reach 누적 (내림차순)
        let mut cum = 0.0;
        let mut cum_card = [0.0f64; 52];
        let mut j = opp_order.len();
        for &h in my_order.iter().rev() {
            let s = my_str[h as usize];
            while j > 0 && opp_str[opp_order[j - 1] as usize] > s {
                let oi = opp_order[j - 1] as usize;
                let (a, b) = self.hands[o][oi];
                let r = reach[oi] as f64;
                cum += r;
                cum_card[a as usize] += r;
                cum_card[b as usize] += r;
                j -= 1;
            }
            let (a, b) = self.hands[p][h as usize];
            lose[h as usize] = cum - cum_card[a as usize] - cum_card[b as usize];
        }

        let (total, per_card) = self.reach_sums(o, reach);
        let mut out = vec![0.0f32; n];
        for &h in my_order {
            let h = h as usize;
            let tie = self.compatible(p, h, reach, total, &per_card) - win[h] - lose[h];
            out[h] = (win[h] * win_pay + lose[h] * lose_pay + tie * tie_pay) as f32;
        }
        out
    }

    /// 찬스 노드에서 카드 한 장이 나올 확률 (양쪽 핸드 4장을 뺀 남은 덱 기준)
    fn chance_factor(&self, node: &tree::Node) -> f32 {
        1.0 / (52 - node.board.count_ones() as i32 - 4) as f32
    }

    fn zero_conflicts(&self, p: usize, card: Card, v: &mut [f32]) {
        for (i, &(a, b)) in self.hands[p].iter().enumerate() {
            if a == card || b == card {
                v[i] = 0.0;
            }
        }
    }
}

/// Regret Matching: 양수 regret에 비례. 전부 0 이하면 균등
fn regret_matching(regrets: &[f32], k: usize, n: usize) -> Vec<f32> {
    let mut s: Vec<f32> = regrets.iter().map(|&r| r.max(0.0)).collect();
    normalize_columns(&mut s, k, n);
    s
}

/// [액션 × 콤보] 배열에서 콤보별로 액션 합이 1이 되게
fn normalize_columns(s: &mut [f32], k: usize, n: usize) {
    for h in 0..n {
        let sum: f32 = (0..k).map(|a| s[a * n + h]).sum();
        for a in 0..k {
            s[a * n + h] = if sum > 0.0 { s[a * n + h] / sum } else { 1.0 / k as f32 };
        }
    }
}

/// DCFR 할인 계수 (α=1.5, β=0, γ=2). t = 이번 반복 번호(1부터)
#[derive(Clone, Copy)]
struct Discount {
    pos: f32,
    neg: f32,
    strat: f32,
}

fn discount(t: u32) -> Discount {
    let t1 = (t - 1) as f64;
    let pa = t1.powf(1.5);
    Discount {
        pos: (pa / (pa + 1.0)) as f32,
        neg: 0.5,
        strat: (t1 / t as f64).powi(2) as f32,
    }
}

/// 병렬 순회 중 노드별로 겹치지 않는 구간에 쓰기 위한 포인터
#[derive(Clone, Copy)]
struct Ptr(*mut f32);
// SAFETY: 트리의 각 액션 노드는 저장소에서 겹치지 않는 구간 [offset, offset+len)을 갖고,
// 한 번의 순회에서 각 노드는 정확히 한 번만 방문된다. 병렬화는 서로 다른 하위 트리
// (찬스 노드의 자식들)로만 나뉘므로 같은 구간을 동시에 쓰는 일이 없다.
unsafe impl Send for Ptr {}
unsafe impl Sync for Ptr {}

impl Ptr {
    /// SAFETY: 호출자는 이 구간을 동시에 다른 곳에서 쓰지 않음을 보장해야 한다.
    #[allow(clippy::mut_from_ref)]
    unsafe fn slice<'a>(self, off: usize, len: usize) -> &'a mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.0.add(off), len) }
    }
}

struct Ctx<'a> {
    game: &'a Game,
    regrets: Ptr,
    strat_sum: Ptr,
    d: Discount,
    /// 이번 순회에서 regret을 업데이트하는 플레이어
    player: usize,
}

/// CFR 순회. reach = 상대 콤보별 도달 확률. 반환 = 내 콤보별 가치
fn cfr(ctx: &Ctx, id: u32, reach: &[f32]) -> Vec<f32> {
    let g = ctx.game;
    let node = &g.tree.nodes[id as usize];
    let p = ctx.player;
    let o = 1 - p;
    match node.kind {
        NodeKind::Fold => g.fold_value(node, p, reach),
        NodeKind::Showdown => g.showdown_value(node, p, reach),
        NodeKind::Chance => {
            let results: Vec<Vec<f32>> = node
                .children
                .par_iter()
                .zip(node.cards.par_iter())
                .map(|(&child, &card)| {
                    let mut r = reach.to_vec();
                    g.zero_conflicts(o, card, &mut r);
                    let mut v = cfr(ctx, child, &r);
                    g.zero_conflicts(p, card, &mut v);
                    v
                })
                .collect();
            sum_scaled(results, g.hands[p].len(), g.chance_factor(node))
        }
        NodeKind::Action if node.player as usize == p => {
            let n = g.hands[p].len();
            let k = node.actions.len();
            // SAFETY: 이 노드의 구간은 이 순회에서 여기서만 접근
            let strategy = regret_matching(unsafe { ctx.regrets.slice(node.data_offset, k * n) }, k, n);
            let child_vals: Vec<Vec<f32>> = node.children.iter().map(|&c| cfr(ctx, c, reach)).collect();

            let mut ev = vec![0.0f32; n];
            for a in 0..k {
                for h in 0..n {
                    ev[h] += strategy[a * n + h] * child_vals[a][h];
                }
            }
            let regrets = unsafe { ctx.regrets.slice(node.data_offset, k * n) };
            for a in 0..k {
                for h in 0..n {
                    let i = a * n + h;
                    let r = regrets[i];
                    let disc = if r >= 0.0 { ctx.d.pos } else { ctx.d.neg };
                    regrets[i] = r * disc + (child_vals[a][h] - ev[h]);
                }
            }
            ev
        }
        NodeKind::Action => {
            // 상대 노드: 상대 전략으로 reach를 나눠 내려보내고, 상대의 평균 전략을 누적
            let n = g.hands[o].len();
            let k = node.actions.len();
            let strategy = regret_matching(unsafe { ctx.regrets.slice(node.data_offset, k * n) }, k, n);
            let ss = unsafe { ctx.strat_sum.slice(node.data_offset, k * n) };
            for a in 0..k {
                for h in 0..n {
                    let i = a * n + h;
                    ss[i] = ss[i] * ctx.d.strat + strategy[i] * reach[h];
                }
            }
            let mut out = vec![0.0f32; g.hands[p].len()];
            for (a, &child) in node.children.iter().enumerate() {
                let r: Vec<f32> = (0..n).map(|h| reach[h] * strategy[a * n + h]).collect();
                for (x, v) in out.iter_mut().zip(cfr(ctx, child, &r)) {
                    *x += v;
                }
            }
            out
        }
    }
}

fn sum_scaled(vs: Vec<Vec<f32>>, n: usize, factor: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; n];
    for v in vs {
        for (x, y) in out.iter_mut().zip(v) {
            *x += y;
        }
    }
    out.iter_mut().for_each(|x| *x *= factor);
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// 내 노드에서 평균 전략 사용 → 현재 전략의 EV
    Average,
    /// 내 노드에서 가장 좋은 액션 선택 → 최적 대응 EV
    BestResponse,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub iteration: u32,
    /// 팟 대비 % (시작팟 기준)
    pub exploitability: f64,
    pub elapsed_secs: f64,
}

pub struct Solver {
    pub game: Game,
    regrets: Vec<f32>,
    strat_sum: Vec<f32>,
    pub iteration: u32,
}

/// 평균 전략 노드 탐색 경로의 한 걸음
#[derive(Clone, Copy, Debug)]
pub enum Step {
    /// 액션 노드에서 n번째 액션
    Action(usize),
    /// 찬스 노드에서 이 카드
    Card(Card),
}

impl Solver {
    pub fn new(game: Game) -> Solver {
        let len = game.storage_len;
        Solver { game, regrets: vec![0.0; len], strat_sum: vec![0.0; len], iteration: 0 }
    }

    /// DCFR 1회 반복 (양 플레이어 번갈아 업데이트)
    pub fn iterate(&mut self) {
        let t = self.iteration + 1;
        for p in 0..2 {
            let ctx = Ctx {
                game: &self.game,
                regrets: Ptr(self.regrets.as_mut_ptr()),
                strat_sum: Ptr(self.strat_sum.as_mut_ptr()),
                d: discount(t),
                player: p,
            };
            cfr(&ctx, self.game.tree.root(), &self.game.init_weights[1 - p]);
        }
        self.iteration = t;
    }

    /// 목표 익스플로이터빌리티(팟 대비 %)에 도달하거나, 최대 반복, 또는 취소될 때까지 반복
    pub fn run(
        &mut self,
        max_iterations: u32,
        target_exploitability: f64,
        check_every: u32,
        cancel: &AtomicBool,
        mut on_progress: impl FnMut(&Progress),
    ) -> Progress {
        let start = Instant::now();
        let check_every = check_every.max(1);
        loop {
            self.iterate();
            let done = self.iteration >= max_iterations || cancel.load(Ordering::Relaxed);
            if self.iteration % check_every == 0 || done {
                let prog = Progress {
                    iteration: self.iteration,
                    exploitability: self.exploitability(),
                    elapsed_secs: start.elapsed().as_secs_f64(),
                };
                on_progress(&prog);
                if done || prog.exploitability <= target_exploitability {
                    return prog;
                }
            }
        }
    }

    /// 노드의 평균 전략. [액션 × 콤보], 콤보는 `game.hands[node.player]` 순서
    pub fn strategy(&self, id: u32) -> Vec<f32> {
        let node = &self.game.tree.nodes[id as usize];
        assert_eq!(node.kind, NodeKind::Action);
        let k = node.actions.len();
        let n = self.game.hands[node.player as usize].len();
        let mut s = self.strat_sum[node.data_offset..node.data_offset + k * n].to_vec();
        normalize_columns(&mut s, k, n);
        s
    }

    fn traverse(&self, id: u32, p: usize, reach: &[f32], mode: Mode) -> Vec<f32> {
        let g = &self.game;
        let node = &g.tree.nodes[id as usize];
        let o = 1 - p;
        match node.kind {
            NodeKind::Fold => g.fold_value(node, p, reach),
            NodeKind::Showdown => g.showdown_value(node, p, reach),
            NodeKind::Chance => {
                let results: Vec<Vec<f32>> = node
                    .children
                    .par_iter()
                    .zip(node.cards.par_iter())
                    .map(|(&child, &card)| {
                        let mut r = reach.to_vec();
                        g.zero_conflicts(o, card, &mut r);
                        let mut v = self.traverse(child, p, &r, mode);
                        g.zero_conflicts(p, card, &mut v);
                        v
                    })
                    .collect();
                sum_scaled(results, g.hands[p].len(), g.chance_factor(node))
            }
            NodeKind::Action if node.player as usize == p => {
                let n = g.hands[p].len();
                let vals: Vec<Vec<f32>> = node.children.iter().map(|&c| self.traverse(c, p, reach, mode)).collect();
                match mode {
                    Mode::BestResponse => (0..n)
                        .map(|h| vals.iter().map(|v| v[h]).fold(f32::NEG_INFINITY, f32::max))
                        .collect(),
                    Mode::Average => {
                        let s = self.strategy(id);
                        (0..n).map(|h| vals.iter().enumerate().map(|(a, v)| s[a * n + h] * v[h]).sum()).collect()
                    }
                }
            }
            NodeKind::Action => {
                let n = g.hands[o].len();
                let s = self.strategy(id);
                let mut out = vec![0.0f32; g.hands[p].len()];
                for (a, &child) in node.children.iter().enumerate() {
                    let r: Vec<f32> = (0..n).map(|h| reach[h] * s[a * n + h]).collect();
                    for (x, v) in out.iter_mut().zip(self.traverse(child, p, &r, mode)) {
                        *x += v;
                    }
                }
                out
            }
        }
    }

    fn total_ev(&self, p: usize, mode: Mode) -> f64 {
        let v = self.traverse(self.game.tree.root(), p, &self.game.init_weights[1 - p], mode);
        let sum: f64 = v.iter().zip(&self.game.init_weights[p]).map(|(&x, &w)| x as f64 * w as f64).sum();
        sum / self.game.pair_weight()
    }

    /// 평균 전략끼리 붙였을 때 [OOP EV, IP EV] (칩). 합 = 시작팟
    pub fn expected_values(&self) -> [f64; 2] {
        [self.total_ev(0, Mode::Average), self.total_ev(1, Mode::Average)]
    }

    /// 익스플로이터빌리티: 양쪽이 상대 평균 전략에 최적 대응할 때 추가로 얻는 EV의 평균.
    /// 시작팟 대비 %로 반환. 0이면 완벽한 균형.
    pub fn exploitability(&self) -> f64 {
        let br = self.total_ev(0, Mode::BestResponse) + self.total_ev(1, Mode::BestResponse);
        let pot = self.game.starting_pot();
        (br - pot) / 2.0 / pot * 100.0
    }

    /// 루트에서 콤보별 EV (칩). 카드가 겹치지 않는 상대 콤보 기준 평균
    pub fn root_hand_evs(&self, p: usize) -> Vec<f32> {
        let g = &self.game;
        let reach = &g.init_weights[1 - p];
        let v = self.traverse(g.tree.root(), p, reach, Mode::Average);
        let (total, per_card) = g.reach_sums(1 - p, reach);
        (0..g.hands[p].len())
            .map(|h| {
                let c = g.compatible(p, h, reach, total, &per_card);
                if c > 0.0 { (v[h] as f64 / c) as f32 } else { 0.0 }
            })
            .collect()
    }

    /// 루트에서 경로를 따라 내려가며 각 플레이어의 콤보별 도달 확률을 계산.
    /// 반환: (도착 노드, [OOP reach, IP reach]) — 초기 가중치 포함
    pub fn walk(&self, path: &[Step]) -> Result<(u32, [Vec<f32>; 2]), String> {
        let g = &self.game;
        let mut id = g.tree.root();
        let mut reach = g.init_weights.clone();
        for step in path {
            let node = &g.tree.nodes[id as usize];
            match (node.kind, *step) {
                (NodeKind::Action, Step::Action(a)) => {
                    if a >= node.actions.len() {
                        return Err(format!("액션 번호 {a} 없음 (0~{})", node.actions.len() - 1));
                    }
                    let p = node.player as usize;
                    let n = g.hands[p].len();
                    let s = self.strategy(id);
                    for h in 0..n {
                        reach[p][h] *= s[a * n + h];
                    }
                    id = node.children[a];
                }
                (NodeKind::Chance, Step::Card(c)) => {
                    let pos = node
                        .cards
                        .iter()
                        .position(|&x| x == c)
                        .ok_or_else(|| format!("{}는 이미 보드에 있음", card_to_string(c)))?;
                    for p in 0..2 {
                        g.zero_conflicts(p, c, &mut reach[p]);
                    }
                    id = node.children[pos];
                }
                (NodeKind::Action, Step::Card(_)) => return Err("여기는 액션 노드: 액션 번호를 입력".into()),
                (NodeKind::Chance, Step::Action(_)) => return Err("여기는 카드 딜: 카드를 입력 (예: Ts)".into()),
                _ => return Err("게임이 이미 끝난 노드".into()),
            }
        }
        Ok((id, reach))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::StreetSizes;

    fn config(board: &str, oop: &str, ip: &str, tree: TreeConfig) -> SolveConfig {
        SolveConfig {
            board: parse_cards(board).unwrap(),
            ranges: [Range::parse(oop).unwrap(), Range::parse(ip).unwrap()],
            tree,
        }
    }

    fn solve(cfg: &SolveConfig, iters: u32) -> Solver {
        let mut s = Solver::new(Game::new(cfg).unwrap());
        for _ in 0..iters {
            s.iterate();
        }
        s
    }

    /// 콤보 가중 평균 액션 빈도 (특정 핸드 클래스만)
    fn class_freq(s: &Solver, id: u32, reach: &[f32], class: &str, action: usize) -> f32 {
        let node = &s.game.tree.nodes[id as usize];
        let p = node.player as usize;
        let n = s.game.hands[p].len();
        let st = s.strategy(id);
        let (mut num, mut den) = (0.0, 0.0);
        for (h, &(a, b)) in s.game.hands[p].iter().enumerate() {
            if hand_class(a, b) == class {
                num += reach[h] * st[action * n + h];
                den += reach[h];
            }
        }
        num / den
    }

    /// 이론 토이 게임: 너트+에어 vs 블러프캐처, 리버 팟벳 하나
    /// 정답: 너트 100% 벳, 블러프는 벳 레인지의 1/3, 블러프캐처는 50% 콜
    #[test]
    fn river_toy_game_matches_theory() {
        let mut tree = TreeConfig::uniform(10.0, 100.0, &[1.0], &[]);
        tree.sizes[2][1] = StreetSizes::default();
        tree.max_raises = 0;
        tree.allow_allin = false;
        // 보드 Kh Qd 7c 3s 2h: 스트레이트·플러시 불가
        // OOP: 77(셋, 3콤보) + 54s(에어, 4콤보) / IP: 88(블러프캐처, 6콤보)
        let cfg = config("Kh Qd 7c 3s 2h", "77,54s", "88", tree);
        let s = solve(&cfg, 3000);

        assert!(s.exploitability() < 0.05, "익스플로이터빌리티 {}", s.exploitability());
        let root = s.game.tree.root();
        let w = &s.game.init_weights;
        let bet_77 = class_freq(&s, root, &w[0], "77", 1);
        let bet_54 = class_freq(&s, root, &w[0], "54s", 1);
        assert!((bet_77 - 1.0).abs() < 0.01, "77 벳 {bet_77}");
        // 밸류 3콤보 : 블러프 1.5콤보 → 에어 4콤보 중 37.5%
        assert!((bet_54 - 0.375).abs() < 0.01, "54s 벳 {bet_54}");

        let (facing, reach) = s.walk(&[Step::Action(1)]).unwrap();
        let call_88 = class_freq(&s, facing, &reach[1], "88", 1);
        assert!((call_88 - 0.5).abs() < 0.01, "88 콜 {call_88}");

        // OOP EV = (3×6×15 + 4×6×0) / (7×6) = 6.4286
        let ev = s.expected_values();
        assert!((ev[0] - 270.0 / 42.0).abs() < 0.02, "OOP EV {}", ev[0]);
        assert!((ev[0] + ev[1] - 10.0).abs() < 1e-3, "EV 합 = 시작팟");
    }

    /// 레이즈·올인이 있는 리버에서 익스플로이터빌리티가 꾸준히 줄어드는지
    #[test]
    fn river_with_raises_converges() {
        let tree = TreeConfig::uniform(10.0, 50.0, &[0.5, 1.0], &[1.0]);
        let cfg = config(
            "Ks 9d 5c 3h 2s",
            "AA,KK,99,55,AK,KQ,KJs,QJs,JTs,T9s,98s,87s,76s,A5s,A4s",
            "QQ-TT,88-66,AK,AQ,KQ,KJ,K9s,Q9s,J9s,T9s,98s,A5s,65s,54s",
            tree,
        );
        let mut s = Solver::new(Game::new(&cfg).unwrap());
        let mut prev = f64::INFINITY;
        for i in 1..=500 {
            s.iterate();
            if i % 100 == 0 {
                let e = s.exploitability();
                assert!(e < prev, "{i}회: {e} (이전 {prev})");
                prev = e;
            }
        }
        assert!(prev < 0.5, "500회 후 익스플로이터빌리티 {prev}%");
        let ev = s.expected_values();
        assert!((ev[0] + ev[1] - 10.0).abs() < 1e-2, "EV 합 {}", ev[0] + ev[1]);
    }

    /// 찬스 노드 검증: 턴에서 체크-체크만 가능하면,
    /// 턴 솔브 EV = 리버 카드별 리버 솔브 EV의 (조합 가중) 평균
    #[test]
    fn turn_equals_average_of_rivers() {
        let mut tree = TreeConfig::uniform(10.0, 50.0, &[0.5, 1.0], &[1.0]);
        tree.sizes[1] = Default::default(); // 턴: 벳 없음
        tree.allow_allin = false;
        tree.max_raises = 1;
        let oop = "77,54s,AK,QJs,T9s,A3s";
        let ip = "88,A7s,KQ,JTs,66,KJs";
        let turn_cfg = config("Kh Qd 7c 3s", oop, ip, tree.clone());
        let turn = solve(&turn_cfg, 1000);
        let turn_ev = turn.expected_values()[0];
        assert!(turn.exploitability() < 0.3);

        let mut weighted = 0.0;
        let mut total_w = 0.0;
        for river in 0..52u8 {
            if turn_cfg.board.contains(&river) {
                continue;
            }
            let mut rc = turn_cfg.clone();
            rc.board.push(river);
            let s = solve(&rc, 1000);
            let w = s.game.pair_weight();
            weighted += w * s.expected_values()[0];
            total_w += w;
        }
        let avg = weighted / total_w;
        assert!((turn_ev - avg).abs() < 0.03, "턴 {turn_ev} vs 리버 평균 {avg}");
    }
}
