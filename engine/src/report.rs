//! 결과 조회용 요약: 한 노드의 전략을 핸드 클래스(AA, AKs, …)별로 묶는다.
//! CLI와 데스크탑 앱이 같이 쓴다.

use crate::card::*;
use crate::solver::{Solver, Step};
use crate::tree::NodeKind;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ClassRow {
    /// 예: "AKs"
    pub class: String,
    /// 이 노드에 도달하는 가중 콤보 수
    pub combos: f64,
    /// 액션별 빈도 (합 1)
    pub freqs: Vec<f64>,
    /// 평균 EV (칩): 지금 팟에서 앞으로 가져갈 기대 칩 − 앞으로 더 넣을 칩
    pub ev: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct NodeSummary {
    pub node: u32,
    pub kind: NodeKind,
    /// Action: 행동할 플레이어 / Fold: 폴드한 플레이어
    pub player: u8,
    pub board: Vec<Card>,
    pub pot: f64,
    /// 액션 라벨 (Action 노드)
    pub actions: Vec<String>,
    /// 레인지 전체의 액션 빈도
    pub overall: Vec<f64>,
    pub total_combos: f64,
    /// AA, AKs, AKo, … 순서
    pub classes: Vec<ClassRow>,
    /// Chance 노드: 나올 수 있는 카드
    pub dealable: Vec<Card>,
}

impl Solver {
    pub fn summarize(&self, path: &[Step]) -> Result<NodeSummary, String> {
        let g = &self.game;
        let w = self.walk_full(path)?;
        let (id, reach) = (w.node, w.reach);
        let node = &g.tree.nodes[id as usize];
        let board_mask = mask_of(&w.board);
        let mut out = NodeSummary {
            node: id,
            kind: node.kind,
            player: node.player,
            board: w.board.clone(),
            pot: g.tree.pot(id),
            actions: node.actions.iter().map(|a| a.label()).collect(),
            overall: Vec::new(),
            total_combos: 0.0,
            classes: Vec::new(),
            dealable: if node.kind == NodeKind::Chance {
                (0..NUM_CARDS as Card).filter(|&c| board_mask & card_bit(c) == 0).collect()
            } else {
                Vec::new()
            },
        };
        if node.kind != NodeKind::Action {
            return Ok(out);
        }

        let p = node.player as usize;
        let n = g.hands[p].len();
        let k = node.actions.len();
        let strat = self.strategy(id);
        let r = &reach[p];
        let evs = Some(self.node_hand_evs(id, p, &reach[1 - p]));

        let total: f64 = r.iter().map(|&x| x as f64).sum();
        out.total_combos = total;
        out.overall = (0..k)
            .map(|a| {
                if total > 0.0 {
                    (0..n).map(|h| (r[h] * strat[a * n + h]) as f64).sum::<f64>() / total
                } else {
                    0.0
                }
            })
            .collect();

        let mut rows: BTreeMap<u32, (ClassRow, f64)> = BTreeMap::new();
        for (h, &(a, b)) in g.hands[p].iter().enumerate() {
            if r[h] <= 0.0 {
                continue;
            }
            let (row, ev_sum) = rows.entry(hand_class_order(a, b)).or_insert_with(|| {
                (ClassRow { class: hand_class(a, b), combos: 0.0, freqs: vec![0.0; k], ev: None }, 0.0)
            });
            let w = r[h] as f64;
            row.combos += w;
            for act in 0..k {
                row.freqs[act] += w * strat[act * n + h] as f64;
            }
            if let Some(e) = &evs {
                *ev_sum += w * e[h] as f64;
            }
        }
        out.classes = rows
            .into_values()
            .map(|(mut row, ev_sum)| {
                row.freqs.iter_mut().for_each(|f| *f /= row.combos);
                if evs.is_some() {
                    row.ev = Some(ev_sum / row.combos);
                }
                row
            })
            .collect();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Game, Range, SolveConfig, StreetSizes, TreeConfig};

    #[test]
    fn summary_of_toy_game() {
        let mut tree = TreeConfig::uniform(10.0, 100.0, &[1.0], &[]);
        tree.sizes[2][1] = StreetSizes::default();
        tree.max_raises = 0;
        tree.allow_allin = false;
        let cfg = SolveConfig {
            board: parse_cards("Kh Qd 7c 3s 2h").unwrap(),
            ranges: [Range::parse("77,54s").unwrap(), Range::parse("88").unwrap()],
            tree,
        };
        let mut s = Solver::new(Game::new(&cfg).unwrap());
        for _ in 0..2000 {
            s.iterate();
        }
        let root = s.summarize(&[]).unwrap();
        assert_eq!(root.kind, NodeKind::Action);
        assert_eq!(root.actions, vec!["Check", "Bet 10"]);
        assert_eq!(root.total_combos, 7.0);
        let classes: Vec<&str> = root.classes.iter().map(|c| c.class.as_str()).collect();
        assert_eq!(classes, vec!["77", "54s"]);
        assert!((root.overall[1] - 4.5 / 7.0).abs() < 0.01, "벳 3+1.5 콤보 / 7");
        assert!((root.classes[0].ev.unwrap() - 15.0).abs() < 0.05);

        // 벳 후 IP 노드: 도달 콤보 6, 콜 50%
        let facing = s.summarize(&[Step::Action(1)]).unwrap();
        assert_eq!(facing.player, 1);
        assert!((facing.overall[1] - 0.5).abs() < 0.01);
        // 88은 콜/폴드가 무차별 → EV ≈ 0 (폴드하면 0, 지금까지 넣은 칩 없음)
        assert!(facing.classes[0].ev.unwrap().abs() < 0.1, "{:?}", facing.classes[0].ev);

        // 종료 노드
        let fold = s.summarize(&[Step::Action(1), Step::Action(0)]).unwrap();
        assert_eq!(fold.kind, NodeKind::Fold);
        assert_eq!(fold.board.len(), 5);
    }
}
