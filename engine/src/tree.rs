//! 게임 트리 빌더.
//!
//! - 플레이어 0 = OOP(아웃오브포지션, 각 스트리트 먼저 행동), 1 = IP
//! - 금액은 모두 칩(f64). `commits[p]` = 포스트플랍에서 p가 지금까지 넣은 칩
//! - 팟 = 시작 팟 + commits[0] + commits[1]
//! - 트리는 Arena 방식: 모든 노드를 `Vec<Node>`에 넣고 인덱스(u32)로 연결
//! - 턴·리버 카드는 찬스 노드로 모두 펼친다 (카드마다 별도 하위 트리)

use crate::card::*;

const EPS: f64 = 1e-9;

/// 액션. 숫자 = 이 액션으로 **추가로 넣는 칩**
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Fold,
    Check,
    Call(f64),
    Bet(f64),
    Raise(f64),
    AllIn(f64),
}

fn fmt_amount(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Action {
    pub fn label(&self) -> String {
        match *self {
            Action::Fold => "Fold".into(),
            Action::Check => "Check".into(),
            Action::Call(a) => format!("Call {}", fmt_amount(a)),
            Action::Bet(a) => format!("Bet {}", fmt_amount(a)),
            Action::Raise(a) => format!("Raise {}", fmt_amount(a)),
            Action::AllIn(a) => format!("AllIn {}", fmt_amount(a)),
        }
    }
}

/// 한 스트리트·한 플레이어의 사이즈 목록 (팟 대비 비율, 예: 0.75 = 75%)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreetSizes {
    /// 벳: 금액 = 비율 × 팟
    pub bet: Vec<f64>,
    /// 레이즈: 추가 금액 = 콜 금액 + 비율 × (콜한 뒤 팟)
    pub raise: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeConfig {
    pub starting_pot: f64,
    pub effective_stack: f64,
    /// [스트리트: 0 플랍, 1 턴, 2 리버][플레이어: 0 OOP, 1 IP]
    pub sizes: [[StreetSizes; 2]; 3],
    /// 스트리트당 최대 레이즈 횟수 (첫 벳은 제외)
    pub max_raises: u32,
    /// 모든 액션 노드에 올인 선택지 추가
    pub allow_allin: bool,
}

impl TreeConfig {
    /// 모든 스트리트·양 플레이어에 같은 사이즈
    pub fn uniform(pot: f64, stack: f64, bet: &[f64], raise: &[f64]) -> Self {
        let s = StreetSizes { bet: bet.to_vec(), raise: raise.to_vec() };
        let street = [s.clone(), s];
        Self {
            starting_pot: pot,
            effective_stack: stack,
            sizes: [street.clone(), street.clone(), street],
            max_raises: 2,
            allow_allin: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// 플레이어 결정
    Action,
    /// 턴/리버 카드 딜
    Chance,
    /// 폴드로 종료 (`player` = 폴드한 사람)
    Fold,
    /// 쇼다운으로 종료
    Showdown,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    /// Action: 행동할 플레이어 / Fold: 폴드한 플레이어
    pub player: u8,
    pub street: u8,
    pub commits: [f64; 2],
    /// 이 노드 시점의 보드 카드 마스크
    pub board: u64,
    /// Action 노드: 선택지 (children과 같은 순서)
    pub actions: Vec<Action>,
    /// Chance 노드: 딜되는 카드 (children과 같은 순서)
    pub cards: Vec<Card>,
    pub children: Vec<u32>,
    /// Action 노드: regret/전략 저장소에서 이 노드 데이터 시작 위치 (솔버가 채움)
    pub data_offset: usize,
    /// Showdown 노드: 리버 보드별 강도 데이터 번호 (솔버가 채움)
    pub showdown_id: u32,
}

impl Node {
    fn new(kind: NodeKind, st: &State) -> Self {
        Self {
            kind,
            player: st.player,
            street: st.street,
            commits: st.commits,
            board: st.board,
            actions: Vec::new(),
            cards: Vec::new(),
            children: Vec::new(),
            data_offset: 0,
            showdown_id: u32::MAX,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.kind, NodeKind::Fold | NodeKind::Showdown)
    }
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub config: TreeConfig,
    pub root_board: Vec<Card>,
}

impl Tree {
    pub fn root(&self) -> u32 {
        0
    }

    pub fn count(&self, kind: NodeKind) -> usize {
        self.nodes.iter().filter(|n| n.kind == kind).count()
    }

    pub fn pot(&self, node: u32) -> f64 {
        let n = &self.nodes[node as usize];
        self.config.starting_pot + n.commits[0] + n.commits[1]
    }
}

#[derive(Clone, Copy, Debug)]
struct State {
    street: u8,
    commits: [f64; 2],
    board: u64,
    player: u8,
    raises: u32,
    /// 이번 스트리트에서 OOP가 체크했는지
    checked: bool,
    /// 직전 벳/레이즈의 증가분 (최소 레이즈 계산용)
    last_inc: f64,
}

pub fn build(config: &TreeConfig, board: &[Card]) -> Result<Tree, String> {
    if !(3..=5).contains(&board.len()) {
        return Err("보드는 3~5장이어야 함".into());
    }
    if config.starting_pot <= 0.0 || config.effective_stack < 0.0 {
        return Err("팟은 0보다 크고 스택은 0 이상이어야 함".into());
    }
    let all_sizes = config.sizes.iter().flatten().flat_map(|s| s.bet.iter().chain(&s.raise));
    if all_sizes.clone().any(|&x| !(x > 0.0)) {
        return Err("사이즈는 0보다 커야 함".into());
    }

    let mut b = Builder { cfg: config, nodes: Vec::new() };
    let st = State {
        street: (board.len() - 3) as u8,
        commits: [0.0; 2],
        board: mask_of(board),
        player: 0,
        raises: 0,
        checked: false,
        last_inc: 0.0,
    };
    b.action(st);
    Ok(Tree { nodes: b.nodes, config: config.clone(), root_board: board.to_vec() })
}

struct Builder<'a> {
    cfg: &'a TreeConfig,
    nodes: Vec<Node>,
}

impl Builder<'_> {
    fn push(&mut self, node: Node) -> u32 {
        self.nodes.push(node);
        (self.nodes.len() - 1) as u32
    }

    fn pot(&self, st: &State) -> f64 {
        self.cfg.starting_pot + st.commits[0] + st.commits[1]
    }

    fn remaining(&self, st: &State, p: usize) -> f64 {
        self.cfg.effective_stack - st.commits[p]
    }

    fn anyone_allin(&self, st: &State) -> bool {
        self.remaining(st, 0) <= EPS || self.remaining(st, 1) <= EPS
    }

    /// 사이즈 목록 → 액션 목록. 스택 이상이면 올인으로 바꾸고, 중복 제거
    fn sized_actions(&self, amounts: Vec<f64>, rem: f64, raise: bool, out: &mut Vec<Action>) {
        let mut need_allin = self.cfg.allow_allin;
        let mut seen: Vec<f64> = Vec::new();
        for a in amounts {
            if a >= rem - EPS {
                need_allin = true;
            } else if !seen.iter().any(|&s| (s - a).abs() < 1e-6) {
                seen.push(a);
            }
        }
        seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out.extend(seen.into_iter().map(|a| if raise { Action::Raise(a) } else { Action::Bet(a) }));
        if need_allin {
            out.push(Action::AllIn(rem));
        }
    }

    fn action(&mut self, st: State) -> u32 {
        let p = st.player as usize;
        let o = 1 - p;
        let to_call = st.commits[o] - st.commits[p];
        let rem = self.remaining(&st, p);
        let pot = self.pot(&st);
        let sizes = &self.cfg.sizes[st.street as usize][p];

        let mut actions = Vec::new();
        if to_call <= EPS {
            actions.push(Action::Check);
            if rem > EPS {
                let amounts = sizes.bet.iter().map(|s| s * pot).collect();
                self.sized_actions(amounts, rem, false, &mut actions);
            }
        } else {
            actions.push(Action::Fold);
            actions.push(Action::Call(to_call.min(rem)));
            let can_raise = st.raises < self.cfg.max_raises
                && rem > to_call + EPS
                && self.remaining(&st, o) > EPS;
            if can_raise {
                let pot_after_call = pot + to_call;
                let min_add = to_call + st.last_inc.max(to_call);
                let amounts = sizes
                    .raise
                    .iter()
                    .map(|s| (to_call + s * pot_after_call).max(min_add))
                    .collect();
                self.sized_actions(amounts, rem, true, &mut actions);
            }
        }

        let id = self.push(Node::new(NodeKind::Action, &st));
        let mut children = Vec::with_capacity(actions.len());
        for &a in &actions {
            let child = match a {
                Action::Fold => self.push(Node::new(NodeKind::Fold, &st)),
                Action::Check => {
                    if p == 0 && !st.checked {
                        self.action(State { player: 1, checked: true, ..st })
                    } else {
                        self.street_end(st)
                    }
                }
                Action::Call(amt) => {
                    let mut ns = st;
                    ns.commits[p] += amt;
                    self.street_end(ns)
                }
                Action::Bet(amt) | Action::Raise(amt) | Action::AllIn(amt) => {
                    let mut ns = st;
                    ns.commits[p] += amt;
                    ns.player = o as u8;
                    ns.last_inc = amt - to_call;
                    if to_call > EPS {
                        ns.raises += 1;
                    }
                    self.action(ns)
                }
            };
            children.push(child);
        }
        let node = &mut self.nodes[id as usize];
        node.actions = actions;
        node.children = children;
        id
    }

    fn street_end(&mut self, st: State) -> u32 {
        if st.street == 2 {
            self.push(Node::new(NodeKind::Showdown, &st))
        } else {
            self.chance(st)
        }
    }

    fn chance(&mut self, st: State) -> u32 {
        let id = self.push(Node::new(NodeKind::Chance, &st));
        let mut cards = Vec::new();
        let mut children = Vec::new();
        for c in 0..NUM_CARDS as Card {
            if st.board & card_bit(c) != 0 {
                continue;
            }
            let ns = State {
                street: st.street + 1,
                board: st.board | card_bit(c),
                player: 0,
                raises: 0,
                checked: false,
                last_inc: 0.0,
                ..st
            };
            let child = if self.anyone_allin(&ns) { self.street_end(ns) } else { self.action(ns) };
            cards.push(c);
            children.push(child);
        }
        let node = &mut self.nodes[id as usize];
        node.cards = cards;
        node.children = children;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn river_cfg() -> TreeConfig {
        let mut cfg = TreeConfig::uniform(10.0, 100.0, &[1.0], &[]);
        cfg.sizes[2][1] = StreetSizes::default(); // IP는 벳 못 함
        cfg.max_raises = 0;
        cfg.allow_allin = false;
        cfg
    }

    #[test]
    fn simple_river_tree() {
        let board = parse_cards("Kh Qd 7c 3s 2h").unwrap();
        let t = build(&river_cfg(), &board).unwrap();
        // root(OOP: Check, Bet 10)
        //   Check → IP(Check) → Showdown
        //   Bet   → IP(Fold, Call 10) → Fold / Showdown
        assert_eq!(t.nodes.len(), 6);
        assert_eq!(t.nodes[0].actions, vec![Action::Check, Action::Bet(10.0)]);
        assert_eq!(t.count(NodeKind::Showdown), 2);
        assert_eq!(t.count(NodeKind::Fold), 1);
        let facing = &t.nodes[t.nodes[0].children[1] as usize];
        assert_eq!(facing.actions, vec![Action::Fold, Action::Call(10.0)]);
    }

    #[test]
    fn allin_conversion_and_raise() {
        // 팟 10, 스택 15: 팟벳 10은 가능, 팟레이즈(40)는 스택 초과 → 올인 15
        let mut cfg = TreeConfig::uniform(10.0, 15.0, &[1.0], &[1.0]);
        cfg.allow_allin = false;
        let board = parse_cards("Kh Qd 7c 3s 2h").unwrap();
        let t = build(&cfg, &board).unwrap();
        assert_eq!(t.nodes[0].actions, vec![Action::Check, Action::Bet(10.0)]);
        let facing = &t.nodes[t.nodes[0].children[1] as usize];
        assert_eq!(facing.actions, vec![Action::Fold, Action::Call(10.0), Action::AllIn(15.0)]);
    }

    #[test]
    fn turn_chance_node() {
        let cfg = TreeConfig::uniform(10.0, 100.0, &[], &[]);
        let mut cfg = cfg;
        cfg.allow_allin = false;
        let board = parse_cards("Kh Qd 7c 3s").unwrap();
        let t = build(&cfg, &board).unwrap();
        // 턴 체크-체크 → 찬스(리버 48장) → 리버 체크-체크 → 쇼다운
        assert_eq!(t.count(NodeKind::Chance), 1);
        let chance = t.nodes.iter().find(|n| n.kind == NodeKind::Chance).unwrap();
        assert_eq!(chance.children.len(), 48);
        assert_eq!(t.count(NodeKind::Showdown), 48);
    }

    #[test]
    fn allin_runs_out_board() {
        // 턴에서 올인-콜 → 리버 찬스 → 쇼다운 (액션 없음)
        let mut cfg = TreeConfig::uniform(10.0, 20.0, &[], &[]);
        cfg.allow_allin = true;
        let board = parse_cards("Kh Qd 7c 3s").unwrap();
        let t = build(&cfg, &board).unwrap();
        let root = &t.nodes[0];
        assert_eq!(root.actions, vec![Action::Check, Action::AllIn(20.0)]);
        let facing = &t.nodes[root.children[1] as usize];
        let call = &t.nodes[facing.children[1] as usize];
        assert_eq!(call.kind, NodeKind::Chance);
        assert!(call.children.iter().all(|&c| t.nodes[c as usize].kind == NodeKind::Showdown));
    }
}
