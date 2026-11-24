//! Kuhn Poker CFR — learning/kuhn_cfr.py 의 Rust 이식
//!
//! Python 버전과 다른 점 하나:
//! - Python: 매 반복마다 카드를 무작위로 한 번 딜 (샘플링)
//! - Rust  : 매 반복마다 가능한 딜 6가지를 전부 계산 (전수 순회)
//!   → 무작위성이 없어서 결과가 매번 같고, 정답에 더 깔끔하게 수렴한다.
//!   실제 엔진도 찬스 노드(턴·리버)를 이렇게 전부 순회한다.

use std::collections::HashMap;

/// p = 체크/폴드, b = 벳/콜
pub const ACTIONS: [char; 2] = ['p', 'b'];
pub const CARD_NAMES: [char; 3] = ['J', 'Q', 'K'];

/// 가능한 딜 6가지: [P1 카드, P2 카드]
const DEALS: [[usize; 2]; 6] = [[0, 1], [0, 2], [1, 0], [1, 2], [2, 0], [2, 1]];

/// 정보 집합 하나: '내 카드 + 지금까지 액션'이 같은 상황
#[derive(Default, Clone, Debug)]
pub struct Node {
    /// 액션별 누적 아쉬움 점수 (regret)
    regret_sum: [f64; 2],
    /// 액션별 누적 전략 (평균 전략 계산용)
    strategy_sum: [f64; 2],
}

impl Node {
    /// Regret Matching: 아쉬움 점수가 플러스인 액션에 비례해서 확률 배분
    fn current_strategy(&self) -> [f64; 2] {
        let pos = [self.regret_sum[0].max(0.0), self.regret_sum[1].max(0.0)];
        normalize(pos)
    }

    /// 최종 답은 '마지막 전략'이 아니라 '전체 반복의 평균 전략'
    pub fn average_strategy(&self) -> [f64; 2] {
        normalize(self.strategy_sum)
    }
}

fn normalize(v: [f64; 2]) -> [f64; 2] {
    let total = v[0] + v[1];
    if total > 0.0 { [v[0] / total, v[1] / total] } else { [0.5, 0.5] }
}

/// 끝난 상황이면 '지금 행동할 차례인 사람' 기준 손익, 아니면 None
fn payoff(cards: [usize; 2], h: &str) -> Option<f64> {
    let b = h.as_bytes();
    if b.len() < 2 {
        return None;
    }
    let player = b.len() % 2;
    let win = cards[player] > cards[1 - player];
    match (b[b.len() - 2], b[b.len() - 1]) {
        (b'b', b'p') => Some(1.0),                            // 벳에 폴드 → 벳한 쪽(지금 차례) +1
        (b'p', b'p') => Some(if win { 1.0 } else { -1.0 }), // 체크-체크 쇼다운
        (b'b', b'b') => Some(if win { 2.0 } else { -2.0 }), // 벳-콜 쇼다운
        _ => None,
    }
}

#[derive(Default)]
pub struct Trainer {
    /// 키 예: "Kpb" = K를 들고, P1 체크 → P2 벳 상황
    pub nodes: HashMap<String, Node>,
}

impl Trainer {
    pub fn new() -> Self {
        Self::default()
    }

    /// CFR을 iterations 회 반복
    pub fn train(&mut self, iterations: u32) {
        let mut h = String::new();
        for _ in 0..iterations {
            for cards in DEALS {
                self.cfr(cards, &mut h, [1.0, 1.0]);
            }
        }
    }

    /// reach[i] = 플레이어 i가 자기 전략대로 이 상황까지 올 확률
    /// 반환: 지금 행동할 사람 기준 EV
    fn cfr(&mut self, cards: [usize; 2], h: &mut String, reach: [f64; 2]) -> f64 {
        if let Some(u) = payoff(cards, h) {
            return u;
        }

        let player = h.len() % 2;
        let key = format!("{}{}", CARD_NAMES[cards[player]], h);
        let s = self.nodes.entry(key.clone()).or_default().current_strategy();

        // 1) 각 액션을 해봤을 때 EV (다음 차례는 상대라 부호 반전)
        let mut action_ev = [0.0; 2];
        for a in 0..2 {
            h.push(ACTIONS[a]);
            let mut next_reach = reach;
            next_reach[player] *= s[a];
            action_ev[a] = -self.cfr(cards, h, next_reach);
            h.pop();
        }

        // 2) 지금 전략의 EV
        let node_ev = s[0] * action_ev[0] + s[1] * action_ev[1];

        // 3) 아쉬움 점수 = (그 액션 EV − 지금 전략 EV), 상대가 여기 올 확률로 가중
        //    평균 전략은 내가 여기 올 확률로 가중해서 누적
        let node = self.nodes.get_mut(&key).expect("방금 만든 노드");
        for a in 0..2 {
            node.regret_sum[a] += reach[1 - player] * (action_ev[a] - node_ev);
            node.strategy_sum[a] += reach[player] * s[a];
        }
        node_ev
    }

    /// 특정 상황의 평균 전략. 예: avg("Q") = P1이 Q를 들고 첫 액션
    pub fn avg(&self, key: &str) -> [f64; 2] {
        self.nodes.get(key).map_or([0.5, 0.5], Node::average_strategy)
    }

    /// 평균 전략끼리 붙였을 때 P1의 정확한 EV (딜 6가지 평균)
    pub fn game_value(&self) -> f64 {
        let mut h = String::new();
        DEALS.iter().map(|&c| self.eval(c, &mut h)).sum::<f64>() / DEALS.len() as f64
    }

    fn eval(&self, cards: [usize; 2], h: &mut String) -> f64 {
        if let Some(u) = payoff(cards, h) {
            return u;
        }
        let player = h.len() % 2;
        let s = self.avg(&format!("{}{}", CARD_NAMES[cards[player]], h));
        let mut v = 0.0;
        for a in 0..2 {
            h.push(ACTIONS[a]);
            v -= s[a] * self.eval(cards, h);
            h.pop();
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 이론상 정답과 비교. 오차 허용 0.02
    #[test]
    fn converges_to_known_equilibrium() {
        let mut t = Trainer::new();
        t.train(50_000);
        let tol = 0.02;
        let close = |a: f64, b: f64| (a - b).abs() < tol;

        let j_bet = t.avg("J")[1];

        assert!(close(t.game_value(), -1.0 / 18.0), "P1 게임 가치 = -1/18");
        assert!(close(t.avg("Q")[1], 0.0), "P1 Q로 먼저 벳 안 함");
        assert!(close(t.avg("K")[1], 3.0 * j_bet), "P1 K 벳 = J 블러프 x 3");
        assert!(close(t.avg("Jp")[1], 1.0 / 3.0), "P2 J 블러프 1/3");
        assert!(close(t.avg("Qb")[1], 1.0 / 3.0), "P2 Q 콜 1/3");
        assert!(close(t.avg("Qpb")[1], j_bet + 1.0 / 3.0), "P1 Q 콜 = J 블러프 + 1/3");
        assert!(close(t.avg("Kb")[1], 1.0), "P2 K 항상 콜");
        assert!(close(t.avg("Jb")[1], 0.0), "P2 J 항상 폴드");
    }
}
