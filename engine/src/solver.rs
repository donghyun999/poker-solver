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
use crate::tree::{self, NodeKind, SuitSymmetry, Tree, TreeConfig};
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
    /// 플레이어별, 보드와 안 겹치는 콤보를 강도 오름차순으로 (연속 메모리로 훑기 위해 카드까지 같이 저장)
    sorted: [Vec<SdHand>; 2],
}

#[derive(Clone, Copy)]
struct SdHand {
    strength: u32,
    idx: u32,
    c1: u8,
    c2: u8,
}

/// 트리 + 레인지 + 쇼다운 정보. 한 번 만들면 바뀌지 않는다.
pub struct Game {
    /// 이 게임을 만든 설정 (저장/불러오기에 사용)
    pub config: SolveConfig,
    pub tree: Tree,
    /// 플레이어별 콤보 목록 (보드와 겹치거나 가중치 0인 콤보는 제외)
    pub hands: [Vec<(Card, Card)>; 2],
    pub init_weights: [Vec<f32>; 2],
    /// 상대 목록에서 같은 콤보의 인덱스 (없으면 NONE)
    same_hand: [Vec<u32>; 2],
    /// [플레이어][수트 쌍 번호] → 콤보 h의 수트를 바꾼 콤보 인덱스 (대칭인 쌍만 채움)
    swap_map: [Vec<Vec<u32>>; 2],
    showdowns: Vec<ShowdownData>,
    storage_len: usize,
    /// 노드별 하위 트리 크기 (노드 수). 큰 하위 트리만 액션 자식을 병렬로 계산
    subtree: Vec<u32>,
}

/// 이보다 큰 하위 트리를 가진 액션 노드는 자식들을 병렬로 계산
const PAR_SUBTREE: u32 = 400;

/// 양쪽 레인지가 수트 a<->b 교환에 대칭인지
fn range_symmetry(ranges: &[Range; 2]) -> SuitSymmetry {
    let mut sym = [[false; 4]; 4];
    for a in 0..4u8 {
        for b in a + 1..4u8 {
            let ok = ranges.iter().all(|r| {
                (0..NUM_COMBOS).all(|i| {
                    let (x, y) = combo_cards(i);
                    r.weights[i] == r.weights[combo_index(swap_suit(x, a, b), swap_suit(y, a, b))]
                })
            });
            sym[a as usize][b as usize] = ok;
            sym[b as usize][a as usize] = ok;
        }
    }
    sym
}

impl Game {
    /// 수트 동형성 사용 (레인지가 대칭인 경우 자동)
    pub fn new(cfg: &SolveConfig) -> Result<Game, String> {
        Self::with_isomorphism(cfg, true)
    }

    pub fn with_isomorphism(cfg: &SolveConfig, isomorphism: bool) -> Result<Game, String> {
        let sym = if isomorphism { range_symmetry(&cfg.ranges) } else { [[false; 4]; 4] };
        let mut tree = tree::build_with_symmetry(&cfg.tree, &cfg.board, &sym)?;
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

        let mut swap_map: [Vec<Vec<u32>>; 2] = Default::default();
        for p in 0..2 {
            let mut pos = vec![NONE; NUM_COMBOS];
            for (i, &(a, b)) in hands[p].iter().enumerate() {
                pos[combo_index(a, b)] = i as u32;
            }
            swap_map[p] = vec![Vec::new(); 6];
            for a in 0..4u8 {
                for b in a + 1..4u8 {
                    if sym[a as usize][b as usize] {
                        swap_map[p][suit_pair_id(a, b)] = hands[p]
                            .iter()
                            .map(|&(x, y)| pos[combo_index(swap_suit(x, a, b), swap_suit(y, a, b))])
                            .collect();
                    }
                }
            }
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
                let mut data = ShowdownData { sorted: Default::default() };
                let mut cards = board.clone();
                cards.extend([0, 0]);
                for p in 0..2 {
                    let mut v: Vec<SdHand> = hands[p]
                        .iter()
                        .enumerate()
                        .filter(|&(_, &(a, b))| mask & (card_bit(a) | card_bit(b)) == 0)
                        .map(|(i, &(a, b))| {
                            cards[5] = a;
                            cards[6] = b;
                            SdHand { strength: evaluate(&cards), idx: i as u32, c1: a, c2: b }
                        })
                        .collect();
                    v.sort_by_key(|x| x.strength);
                    data.sorted[p] = v;
                }
                data
            })
            .collect();

        // 트리는 전위 순서로 저장되어 있어서, 하위 트리 = [id, 마지막 자식의 끝)
        let mut end = vec![0u32; tree.nodes.len()];
        for id in (0..tree.nodes.len()).rev() {
            end[id] = tree.nodes[id].children.last().map_or(id as u32 + 1, |&c| end[c as usize]);
        }
        let subtree = end.iter().enumerate().map(|(i, &e)| e - i as u32).collect();

        Ok(Game {
            config: cfg.clone(),
            tree,
            hands,
            init_weights,
            same_hand,
            swap_map,
            showdowns,
            storage_len: offset,
            subtree,
        })
    }

    pub fn starting_pot(&self) -> f64 {
        self.tree.config.starting_pot
    }

    /// regret + 누적 전략 저장에 필요한 메모리 (바이트).
    /// compressed = 16비트 압축 (절반 크기 + 노드별 스케일)
    pub fn memory_bytes(&self, compressed: bool) -> usize {
        if compressed {
            self.storage_len * 2 * 2 + self.tree.nodes.len() * 2 * 4
        } else {
            self.storage_len * 2 * 4
        }
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
        if total <= 0.0 {
            return vec![0.0; self.hands[p].len()];
        }
        (0..self.hands[p].len())
            .map(|h| (payoff * self.compatible(p, h, reach, total, &per_card)) as f32)
            .collect()
    }

    /// 쇼다운 가치. 강도순 정렬 + 누적합으로 O(n).
    /// 가치 = 무승부 지급 × 겹치지 않는 상대 + (이김 − 무승부) × 약한 상대 + (짐 − 무승부) × 강한 상대
    fn showdown_value(&self, node: &tree::Node, p: usize, reach: &[f32]) -> Vec<f32> {
        let o = 1 - p;
        let n = self.hands[p].len();
        let (total, per_card) = self.reach_sums(o, reach);
        if total <= 0.0 {
            return vec![0.0f32; n];
        }
        // f64로 모아서 마지막에 한 번만 f32로 (정밀도)
        let mut acc = vec![0.0f64; n];
        let sd = &self.showdowns[node.showdown_id as usize];
        let tie_pay = self.starting_pot() / 2.0;
        let win_extra = self.starting_pot() + node.commits[o] - tie_pay;
        let lose_extra = -node.commits[p] - tie_pay;
        let (mine, opp) = (&sd.sorted[p], &sd.sorted[o]);

        // 약한 상대 reach 누적 (오름차순)
        let mut cum = 0.0f64;
        let mut cum_card = [0.0f64; 52];
        let mut j = 0;
        for m in mine {
            while j < opp.len() && opp[j].strength < m.strength {
                let x = &opp[j];
                let r = reach[x.idx as usize] as f64;
                cum += r;
                cum_card[x.c1 as usize] += r;
                cum_card[x.c2 as usize] += r;
                j += 1;
            }
            let h = m.idx as usize;
            let win = cum - cum_card[m.c1 as usize] - cum_card[m.c2 as usize];
            let compat = self.compatible(p, h, reach, total, &per_card);
            acc[h] = compat * tie_pay + win * win_extra;
        }

        // 강한 상대 reach 누적 (내림차순)
        let mut cum = 0.0f64;
        let mut cum_card = [0.0f64; 52];
        let mut j = opp.len();
        for m in mine.iter().rev() {
            while j > 0 && opp[j - 1].strength > m.strength {
                let x = &opp[j - 1];
                let r = reach[x.idx as usize] as f64;
                cum += r;
                cum_card[x.c1 as usize] += r;
                cum_card[x.c2 as usize] += r;
                j -= 1;
            }
            let lose = cum - cum_card[m.c1 as usize] - cum_card[m.c2 as usize];
            acc[m.idx as usize] += lose * lose_extra;
        }
        acc.into_iter().map(|x| x as f32).collect()
    }

    /// 찬스 노드에서 카드 한 장이 나올 확률 (양쪽 핸드 4장을 뺀 남은 덱 기준)
    fn chance_factor(&self, node: &tree::Node) -> f32 {
        1.0 / (52 - node.board.count_ones() as i32 - 4) as f32
    }

    /// 찬스 노드: 대표 카드별 가치(results, 카드 겹침 0 처리됨) + 수트 동형 카드 -> 확률 가중 평균
    fn chance_sum(&self, node: &tree::Node, p: usize, results: Vec<Vec<f32>>) -> Vec<f32> {
        let n = self.hands[p].len();
        let mut out = vec![0.0f32; n];
        for v in &results {
            for (x, y) in out.iter_mut().zip(v) {
                *x += y;
            }
        }
        for iso in &node.iso {
            // 수트를 바꾼 상황: 콤보 h의 가치 = 대표 카드 상황에서 h의 수트를 바꾼 콤보의 가치
            let v = &results[iso.canon as usize];
            let map = &self.swap_map[p][suit_pair_id(iso.swap.0, iso.swap.1)];
            for (x, &m) in out.iter_mut().zip(map) {
                if m != NONE {
                    *x += v[m as usize];
                }
            }
        }
        let f = self.chance_factor(node);
        out.iter_mut().for_each(|x| *x *= f);
        out
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

/// [액션 × 콤보] 배열에서 콤보별로 액션 합이 1이 되게 (합이 0이면 균등).
/// 액션 행 단위로 연속 메모리를 훑어서 SIMD로 컴파일되게 한다.
fn normalize_columns(s: &mut [f32], k: usize, n: usize) {
    let mut sum = vec![0.0f32; n];
    for row in s.chunks_exact(n).take(k) {
        for (t, &x) in sum.iter_mut().zip(row) {
            *t += x;
        }
    }
    let uniform = 1.0 / k as f32;
    for row in s.chunks_exact_mut(n).take(k) {
        for (x, &t) in row.iter_mut().zip(&sum) {
            *x = if t > 0.0 { *x / t } else { uniform };
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
struct Ptr<T>(*mut T);
impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Ptr<T> {}
// SAFETY: 트리의 각 액션 노드는 저장소에서 겹치지 않는 구간 [offset, offset+len)과
// 자기 노드 번호의 스케일 칸을 갖고, 한 번의 순회에서 각 노드는 정확히 한 번만 방문된다.
// 병렬화는 서로 다른 하위 트리(찬스·액션 노드의 자식들)로만 나뉘므로 같은 곳을 동시에 쓰는 일이 없다.
unsafe impl<T: Send> Send for Ptr<T> {}
unsafe impl<T: Send> Sync for Ptr<T> {}

impl<T> Ptr<T> {
    /// SAFETY: 호출자는 이 구간을 동시에 다른 곳에서 쓰지 않음을 보장해야 한다.
    #[allow(clippy::mut_from_ref)]
    unsafe fn slice<'a>(self, off: usize, len: usize) -> &'a mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.0.add(off), len) }
    }
}

/// regret / 누적 전략 저장소.
/// - F32: 그대로 저장
/// - C16: 16비트 정수 + 노드별 스케일(최댓값). 메모리 절반, 정밀도 약간 손해
pub(crate) enum Storage {
    F32 { regrets: Vec<f32>, strat: Vec<f32> },
    C16 { regrets: Vec<i16>, strat: Vec<u16>, rscale: Vec<f32>, sscale: Vec<f32> },
}

impl Storage {
    pub(crate) fn new(len: usize, num_nodes: usize, compressed: bool) -> Storage {
        if compressed {
            Storage::C16 {
                regrets: vec![0; len],
                strat: vec![0; len],
                rscale: vec![0.0; num_nodes],
                sscale: vec![0.0; num_nodes],
            }
        } else {
            Storage::F32 { regrets: vec![0.0; len], strat: vec![0.0; len] }
        }
    }

    pub(crate) fn is_compressed(&self) -> bool {
        matches!(self, Storage::C16 { .. })
    }

    fn view(&mut self) -> View {
        match self {
            Storage::F32 { regrets, strat } => View::F32 { regrets: Ptr(regrets.as_mut_ptr()), strat: Ptr(strat.as_mut_ptr()) },
            Storage::C16 { regrets, strat, rscale, sscale } => View::C16 {
                regrets: Ptr(regrets.as_mut_ptr()),
                strat: Ptr(strat.as_mut_ptr()),
                rscale: Ptr(rscale.as_mut_ptr()),
                sscale: Ptr(sscale.as_mut_ptr()),
            },
        }
    }

    /// 읽기 전용 뷰. 반환된 뷰로는 load만 호출해야 한다.
    fn view_ro(&self) -> View {
        match self {
            Storage::F32 { regrets, strat } => View::F32 {
                regrets: Ptr(regrets.as_ptr() as *mut f32),
                strat: Ptr(strat.as_ptr() as *mut f32),
            },
            Storage::C16 { regrets, strat, rscale, sscale } => View::C16 {
                regrets: Ptr(regrets.as_ptr() as *mut i16),
                strat: Ptr(strat.as_ptr() as *mut u16),
                rscale: Ptr(rscale.as_ptr() as *mut f32),
                sscale: Ptr(sscale.as_ptr() as *mut f32),
            },
        }
    }
}

#[derive(Clone, Copy)]
enum View {
    F32 { regrets: Ptr<f32>, strat: Ptr<f32> },
    C16 { regrets: Ptr<i16>, strat: Ptr<u16>, rscale: Ptr<f32>, sscale: Ptr<f32> },
}

impl View {
    /// SAFETY: 노드 id의 구간을 동시에 쓰는 곳이 없어야 한다 (Ptr 참고)
    unsafe fn load_regrets(self, id: u32, off: usize, len: usize) -> Vec<f32> {
        unsafe {
            match self {
                View::F32 { regrets, .. } => regrets.slice(off, len).to_vec(),
                View::C16 { regrets, rscale, .. } => {
                    let sc = rscale.slice(id as usize, 1)[0] / i16::MAX as f32;
                    regrets.slice(off, len).iter().map(|&q| q as f32 * sc).collect()
                }
            }
        }
    }

    unsafe fn save_regrets(self, id: u32, off: usize, v: &[f32]) {
        unsafe {
            match self {
                View::F32 { regrets, .. } => regrets.slice(off, v.len()).copy_from_slice(v),
                View::C16 { regrets, rscale, .. } => {
                    let m = v.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
                    rscale.slice(id as usize, 1)[0] = m;
                    let inv = if m > 0.0 { i16::MAX as f32 / m } else { 0.0 };
                    for (q, &x) in regrets.slice(off, v.len()).iter_mut().zip(v) {
                        *q = (x * inv).round() as i16;
                    }
                }
            }
        }
    }

    unsafe fn load_strat(self, id: u32, off: usize, len: usize) -> Vec<f32> {
        unsafe {
            match self {
                View::F32 { strat, .. } => strat.slice(off, len).to_vec(),
                View::C16 { strat, sscale, .. } => {
                    let sc = sscale.slice(id as usize, 1)[0] / u16::MAX as f32;
                    strat.slice(off, len).iter().map(|&q| q as f32 * sc).collect()
                }
            }
        }
    }

    unsafe fn save_strat(self, id: u32, off: usize, v: &[f32]) {
        unsafe {
            match self {
                View::F32 { strat, .. } => strat.slice(off, v.len()).copy_from_slice(v),
                View::C16 { strat, sscale, .. } => {
                    let m = v.iter().fold(0.0f32, |m, &x| m.max(x));
                    sscale.slice(id as usize, 1)[0] = m;
                    let inv = if m > 0.0 { u16::MAX as f32 / m } else { 0.0 };
                    for (q, &x) in strat.slice(off, v.len()).iter_mut().zip(v) {
                        *q = (x * inv).round() as u16;
                    }
                }
            }
        }
    }
}

struct Ctx<'a> {
    game: &'a Game,
    store: View,
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
            g.chance_sum(node, p, results)
        }
        NodeKind::Action if node.player as usize == p => {
            let n = g.hands[p].len();
            let k = node.actions.len();
            // SAFETY: 이 노드의 구간은 이 순회에서 여기서만 접근
            let mut regrets = unsafe { ctx.store.load_regrets(id, node.data_offset, k * n) };
            let strategy = regret_matching(&regrets, k, n);
            let child_vals: Vec<Vec<f32>> = if g.subtree[id as usize] > PAR_SUBTREE {
                node.children.par_iter().map(|&c| cfr(ctx, c, reach)).collect()
            } else {
                node.children.iter().map(|&c| cfr(ctx, c, reach)).collect()
            };

            let mut ev = vec![0.0f32; n];
            for (srow, cv) in strategy.chunks_exact(n).zip(&child_vals) {
                for ((e, &sv), &v) in ev.iter_mut().zip(srow).zip(cv) {
                    *e += sv * v;
                }
            }
            let (pos, neg) = (ctx.d.pos, ctx.d.neg);
            for (rrow, cv) in regrets.chunks_exact_mut(n).zip(&child_vals) {
                for ((r, &v), &e) in rrow.iter_mut().zip(cv).zip(&ev) {
                    let disc = if *r >= 0.0 { pos } else { neg };
                    *r = *r * disc + (v - e);
                }
            }
            unsafe { ctx.store.save_regrets(id, node.data_offset, &regrets) };
            ev
        }
        NodeKind::Action => {
            // 상대 노드: 상대 전략으로 reach를 나눠 내려보내고, 상대의 평균 전략을 누적
            let n = g.hands[o].len();
            let k = node.actions.len();
            let regrets = unsafe { ctx.store.load_regrets(id, node.data_offset, k * n) };
            let strategy = regret_matching(&regrets, k, n);
            let mut ss = unsafe { ctx.store.load_strat(id, node.data_offset, k * n) };
            let gamma = ctx.d.strat;
            for (ssrow, srow) in ss.chunks_exact_mut(n).zip(strategy.chunks_exact(n)) {
                for ((x, &sv), &r) in ssrow.iter_mut().zip(srow).zip(reach) {
                    *x = *x * gamma + sv * r;
                }
            }
            unsafe { ctx.store.save_strat(id, node.data_offset, &ss) };
            // 액션별 reach는 ss 버퍼를 재사용 (저장 끝났으므로)
            let child = |((&c, srow), r): ((&u32, &[f32]), &mut [f32])| {
                for ((x, &sv), &rh) in r.iter_mut().zip(srow).zip(reach) {
                    *x = rh * sv;
                }
                cfr(ctx, c, r)
            };
            let vals: Vec<Vec<f32>> = if g.subtree[id as usize] > PAR_SUBTREE {
                node.children.par_iter().zip(strategy.par_chunks_exact(n)).zip(ss.par_chunks_exact_mut(n)).map(child).collect()
            } else {
                node.children.iter().zip(strategy.chunks_exact(n)).zip(ss.chunks_exact_mut(n)).map(child).collect()
            };
            let mut out = vec![0.0f32; g.hands[p].len()];
            for v in vals {
                for (x, y) in out.iter_mut().zip(v) {
                    *x += y;
                }
            }
            out
        }
    }
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
    pub(crate) storage: Storage,
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
        Self::with_compression(game, false)
    }

    /// compressed = true면 16비트 압축 저장 (메모리 약 절반)
    pub fn with_compression(game: Game, compressed: bool) -> Solver {
        let storage = Storage::new(game.storage_len, game.tree.nodes.len(), compressed);
        Solver { game, storage, iteration: 0 }
    }

    pub fn is_compressed(&self) -> bool {
        self.storage.is_compressed()
    }

    /// DCFR 1회 반복 (양 플레이어 번갈아 업데이트)
    pub fn iterate(&mut self) {
        let t = self.iteration + 1;
        let store = self.storage.view();
        for p in 0..2 {
            let ctx = Ctx { game: &self.game, store, d: discount(t), player: p };
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
        // SAFETY: &self라 동시에 쓰는 곳이 없음
        let mut s = unsafe { self.storage.view_ro().load_strat(id, node.data_offset, k * n) };
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
                g.chance_sum(node, p, results)
            }
            NodeKind::Action if node.player as usize == p => {
                let n = g.hands[p].len();
                let vals: Vec<Vec<f32>> = if g.subtree[id as usize] > PAR_SUBTREE {
                    node.children.par_iter().map(|&c| self.traverse(c, p, reach, mode)).collect()
                } else {
                    node.children.iter().map(|&c| self.traverse(c, p, reach, mode)).collect()
                };
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
                let child = |(&c, srow): (&u32, &[f32])| {
                    let r: Vec<f32> = reach.iter().zip(srow).map(|(&rh, &sv)| rh * sv).collect();
                    self.traverse(c, p, &r, mode)
                };
                let vals: Vec<Vec<f32>> = if g.subtree[id as usize] > PAR_SUBTREE {
                    node.children.par_iter().zip(s.par_chunks_exact(n)).map(child).collect()
                } else {
                    node.children.iter().zip(s.chunks_exact(n)).map(child).collect()
                };
                let mut out = vec![0.0f32; g.hands[p].len()];
                for v in vals {
                    for (x, y) in out.iter_mut().zip(v) {
                        *x += y;
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
        self.walk_full(path).map(|w| (w.node, w.reach))
    }

    /// `walk` + 실제 보드. 수트 동형 카드를 고르면 트리에서는 대표 카드의 하위 트리로 가고,
    /// reach는 그 트리 기준(수트를 바꾼 콤보 순서)으로 바뀐다.
    pub fn walk_full(&self, path: &[Step]) -> Result<Walk, String> {
        let g = &self.game;
        let mut id = g.tree.root();
        let mut reach = g.init_weights.clone();
        let mut board = g.config.board.clone();
        // 실제 수트 -> 트리 수트
        let mut perm = [0u8, 1, 2, 3];
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
                    if board.contains(&c) {
                        return Err(format!("{}는 이미 보드에 있음", card_to_string(c)));
                    }
                    let t = make_card(rank(c), perm[suit(c) as usize]);
                    let (pos, tree_card) = match node.cards.iter().position(|&x| x == t) {
                        Some(pos) => (pos, t),
                        None => {
                            let iso = node.iso.iter().find(|i| i.card == t).ok_or("카드를 찾을 수 없음")?;
                            let (a, b) = iso.swap;
                            for s in perm.iter_mut() {
                                *s = if *s == a { b } else if *s == b { a } else { *s };
                            }
                            for p in 0..2 {
                                let map = &g.swap_map[p][suit_pair_id(a, b)];
                                reach[p] = map.iter().map(|&m| if m == NONE { 0.0 } else { reach[p][m as usize] }).collect();
                            }
                            let pos = iso.canon as usize;
                            (pos, node.cards[pos])
                        }
                    };
                    for p in 0..2 {
                        g.zero_conflicts(p, tree_card, &mut reach[p]);
                    }
                    board.push(c);
                    id = node.children[pos];
                }
                (NodeKind::Action, Step::Card(_)) => return Err("여기는 액션 노드: 액션 번호를 입력".into()),
                (NodeKind::Chance, Step::Action(_)) => return Err("여기는 카드 딜: 카드를 입력 (예: Ts)".into()),
                _ => return Err("게임이 이미 끝난 노드".into()),
            }
        }
        Ok(Walk { node: id, reach, board })
    }
}

/// `walk_full` 결과
pub struct Walk {
    pub node: u32,
    /// 트리 기준 콤보 순서 (`game.hands`)
    pub reach: [Vec<f32>; 2],
    /// 실제로 고른 카드 순서의 보드
    pub board: Vec<Card>,
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

    /// 16비트 압축: 결과가 압축 안 한 것과 거의 같고, 수렴도 하는지
    #[test]
    fn compressed_storage_matches_f32() {
        let tree = TreeConfig::uniform(10.0, 50.0, &[0.5, 1.0], &[1.0]);
        let cfg = config(
            "Ks 9d 5c 3h",
            "AA,KK,99,55,AK,KQ,KJs,QJs,JTs,T9s,98s,87s,76s,A5s,A4s",
            "QQ-TT,88-66,AK,AQ,KQ,KJ,K9s,Q9s,J9s,T9s,98s,A5s,65s,54s",
            tree,
        );
        let mut f = Solver::with_compression(Game::new(&cfg).unwrap(), false);
        let mut c = Solver::with_compression(Game::new(&cfg).unwrap(), true);
        for _ in 0..300 {
            f.iterate();
            c.iterate();
        }
        let (ef, ec) = (f.exploitability(), c.exploitability());
        assert!(ec < 0.5, "압축 익스플로이터빌리티 {ec}% (f32 {ef}%)");
        let (sf, sc) = (f.strategy(0), c.strategy(0));
        let w = &f.game.init_weights[0];
        let n = w.len();
        let total: f32 = w.iter().sum();
        for a in 0..sf.len() / n {
            let freq = |s: &[f32]| (0..n).map(|h| w[h] * s[a * n + h]).sum::<f32>() / total;
            assert!((freq(&sf) - freq(&sc)).abs() < 0.03, "액션 {a}: {} vs {}", freq(&sf), freq(&sc));
        }
        assert!(c.game.memory_bytes(true) < f.game.memory_bytes(false) * 6 / 10);
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

    /// 투톤 이하 보드: 수트 동형성으로 트리가 줄어도 결과는 전부 펼친 트리와 같아야 한다
    #[test]
    fn isomorphism_matches_full_tree() {
        let mut tree = TreeConfig::uniform(20.0, 60.0, &[0.5, 1.0], &[1.0]);
        tree.max_raises = 1;
        // 턴 보드에 d, s가 없음 → 리버 s 카드는 d 카드와 동형
        let cfg = config("Kh 9h 4h 2c", "22+,A2s+,K9s+,QTs+,JTs,ATo+,KQo", "55+,A8s+,KTs+,QJs,AJo+", tree);
        let run = |iso: bool, iters: u32| {
            let mut s = Solver::new(Game::with_isomorphism(&cfg, iso).unwrap());
            for _ in 0..iters {
                s.iterate();
            }
            s
        };

        // 반복 전(균등 전략): 같은 전략이라 콤보별 EV·최적 대응까지 정확히 같아야 함.
        // (반복을 돌리면 regret이 0 근처인 콤보에서 미세한 오차가 전략을 크게 바꿔 궤적이 갈라짐)
        let (full, small) = (run(false, 0), run(true, 0));
        assert!(small.game.tree.nodes.len() < full.game.tree.nodes.len() * 4 / 5);
        for p in 0..2 {
            for (x, y) in full.root_hand_evs(p).iter().zip(small.root_hand_evs(p)) {
                assert!((x - y).abs() < 1e-3, "{x} vs {y}");
            }
        }
        let (xf, xs) = (full.exploitability(), small.exploitability());
        assert!((xf - xs).abs() < 1e-4 * xf, "exploitability {xf} vs {xs}");
        for path in [
            vec![Step::Action(0), Step::Action(0), Step::Card(parse_card("5s").unwrap())],
            vec![Step::Action(0), Step::Action(0), Step::Card(parse_card("5s").unwrap()), Step::Action(1)],
            vec![Step::Action(2), Step::Action(1), Step::Card(parse_card("As").unwrap())],
        ] {
            let (a, b) = (full.summarize(&path).unwrap(), small.summarize(&path).unwrap());
            assert_eq!(a.board, b.board);
            assert_eq!(a.actions, b.actions);
            assert_eq!(a.dealable, b.dealable);
            assert!((a.total_combos - b.total_combos).abs() < 1e-3);
        }

        // 수렴 후: 균형이 유일하지 않아 핸드별 빈도는 갈릴 수 있지만 EV·익스플로이터빌리티는 같다
        let (full, small) = (run(false, 300), run(true, 300));
        let (ef, es) = (full.expected_values(), small.expected_values());
        assert!((ef[0] - es[0]).abs() < 1e-2, "{ef:?} vs {es:?}");
        assert!(full.exploitability() < 0.2 && small.exploitability() < 0.2);

        // 동형 트리 안에서: 5s로 가든 5d로 가든 같은 하위 트리 → 결과가 정확히 같아야 함
        let via = |c: &str| {
            small.summarize(&[Step::Action(0), Step::Action(0), Step::Card(parse_card(c).unwrap()), Step::Action(1)]).unwrap()
        };
        let (d, s) = (via("5d"), via("5s"));
        assert_eq!(d.board.last(), Some(&parse_card("5d").unwrap()));
        assert_eq!(s.board.last(), Some(&parse_card("5s").unwrap()));
        assert!((d.total_combos - s.total_combos).abs() < 1e-6);
        for (x, y) in d.overall.iter().zip(&s.overall) {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    fn asymmetric_range_disables_isomorphism() {
        let tree = TreeConfig::uniform(20.0, 60.0, &[1.0], &[]);
        let sym = config("Kh 9h 4h 2c", "AA,KK", "QQ", tree.clone());
        let asym = config("Kh 9h 4h 2c", "AA,KK,AdKd", "QQ", tree);
        let a = Game::new(&sym).unwrap().tree.nodes.len();
        let b = Game::new(&asym).unwrap().tree.nodes.len();
        let full = Game::with_isomorphism(&sym, false).unwrap().tree.nodes.len();
        assert!(a < full);
        assert_eq!(b, full);
    }

    /// O(n) 쇼다운 계산 = 모든 콤보 쌍을 직접 비교한 결과
    #[test]
    fn showdown_matches_brute_force() {
        let tree = TreeConfig::uniform(10.0, 50.0, &[0.5], &[]);
        let cfg = config("Kh 9d 4c 2s 2h", "22+,A2s+,K5s+,QTs+,A9o+,KJo", "33+,A4s+,K9s+,QJs,ATo+", tree);
        let g = Game::new(&cfg).unwrap();
        let board = parse_cards("Kh 9d 4c 2s 2h").unwrap();
        let strength = |a: Card, b: Card| {
            let mut c = board.clone();
            c.extend([a, b]);
            evaluate(&c)
        };
        let mut seed = 7u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f32 / 1000.0
        };
        for node in g.tree.nodes.iter().filter(|n| n.kind == NodeKind::Showdown) {
            for p in 0..2 {
                let o = 1 - p;
                let reach: Vec<f32> = (0..g.hands[o].len()).map(|_| rnd()).collect();
                let fast = g.showdown_value(node, p, &reach);
                for (h, &(a, b)) in g.hands[p].iter().enumerate() {
                    let my = strength(a, b);
                    let mut want = 0.0f64;
                    for (j, &(c, d)) in g.hands[o].iter().enumerate() {
                        if [c, d].iter().any(|x| *x == a || *x == b) {
                            continue;
                        }
                        let opp = strength(c, d);
                        let pay = if my > opp {
                            g.starting_pot() + node.commits[o]
                        } else if my < opp {
                            -node.commits[p]
                        } else {
                            g.starting_pot() / 2.0
                        };
                        want += pay * reach[j] as f64;
                    }
                    assert!((fast[h] as f64 - want).abs() < 1e-3 * want.abs().max(1.0), "{} {} vs {want}", h, fast[h]);
                }
            }
        }
    }
}
