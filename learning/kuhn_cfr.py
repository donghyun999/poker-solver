"""
Kuhn Poker CFR - 솔버 원리 학습용

규칙
- 카드 3장: J(0) < Q(1) < K(2). 두 사람이 한 장씩 받음
- 둘 다 1칩 앤티 (팟 2)
- P1 먼저: 체크(p) 또는 벳(b, 1칩)
- 액션 기록(history) 예: "pb" = P1 체크, P2 벳
- 끝나는 경우
    pp        : 둘 다 체크 → 쇼다운, 이긴 쪽 +1
    bp / pbp  : 벳에 폴드 → 벳한 쪽 +1
    bb / pbb  : 벳에 콜 → 쇼다운, 이긴 쪽 +2

이론상 정답 (검증 기준)
- P1의 게임 가치 = -1/18 ≈ -0.0556
- P1은 Q로 절대 먼저 벳하지 않음
- P1의 K 벳 빈도 = J 블러프 빈도 x 3
"""

import random

ACTIONS = ["p", "b"]  # p = 체크/폴드, b = 벳/콜
CARD_NAMES = ["J", "Q", "K"]


class Node:
    """정보 집합 하나: '내 카드 + 지금까지 액션'이 같은 상황"""

    def __init__(self):
        self.regret_sum = [0.0, 0.0]    # 액션별 누적 아쉬움 점수
        self.strategy_sum = [0.0, 0.0]  # 액션별 누적 전략 (평균 전략 계산용)

    def strategy(self, my_reach):
        # Regret Matching: 아쉬움 점수가 플러스인 액션에 비례해서 확률 배분
        pos = [max(r, 0.0) for r in self.regret_sum]
        total = sum(pos)
        s = [p / total for p in pos] if total > 0 else [0.5, 0.5]
        # 평균 전략 누적 (내가 이 상황에 올 확률로 가중)
        for a in range(2):
            self.strategy_sum[a] += my_reach * s[a]
        return s

    def average_strategy(self):
        # 최종 답은 '마지막 전략'이 아니라 '전체 반복의 평균 전략'
        total = sum(self.strategy_sum)
        return [x / total for x in self.strategy_sum] if total > 0 else [0.5, 0.5]


nodes = {}


def payoff(cards, h):
    """끝난 상황이면 '지금 행동할 차례인 사람' 기준 손익, 아니면 None"""
    player = len(h) % 2
    opp = 1 - player
    if len(h) < 2:
        return None
    if h[-1] == "p" and h[-2] == "b":       # 벳에 폴드 → 지금 차례인 사람(벳한 쪽)이 +1
        return 1
    if h == "pp":                            # 체크-체크 쇼다운
        return 1 if cards[player] > cards[opp] else -1
    if h[-2:] == "bb":                       # 벳-콜 쇼다운
        return 2 if cards[player] > cards[opp] else -2
    return None


def cfr(cards, h, p0, p1):
    """
    h  : 지금까지 액션
    p0 : P1이 이 상황까지 올 확률 (P1 전략 기준)
    p1 : P2가 이 상황까지 올 확률 (P2 전략 기준)
    반환: 지금 행동할 사람 기준 EV
    """
    result = payoff(cards, h)
    if result is not None:
        return result

    player = len(h) % 2
    key = CARD_NAMES[cards[player]] + h
    node = nodes.setdefault(key, Node())

    my_reach = p0 if player == 0 else p1
    opp_reach = p1 if player == 0 else p0
    s = node.strategy(my_reach)

    # 1) 각 액션을 해봤을 때 EV 계산 (상대 EV의 반대 부호)
    action_ev = [0.0, 0.0]
    for a in range(2):
        nh = h + ACTIONS[a]
        if player == 0:
            action_ev[a] = -cfr(cards, nh, p0 * s[a], p1)
        else:
            action_ev[a] = -cfr(cards, nh, p0, p1 * s[a])

    # 2) 지금 전략의 EV
    node_ev = s[0] * action_ev[0] + s[1] * action_ev[1]

    # 3) 아쉬움 점수 = (그 액션 EV - 지금 전략 EV), 상대가 여기 올 확률로 가중
    for a in range(2):
        node.regret_sum[a] += opp_reach * (action_ev[a] - node_ev)

    return node_ev


def train(iterations):
    cards = [0, 1, 2]
    total_ev = 0.0
    for _ in range(iterations):
        random.shuffle(cards)          # 카드 딜 (찬스)
        total_ev += cfr(cards, "", 1.0, 1.0)
    return total_ev / iterations


def label(key):
    card, h = key[0], key[1:]
    who = "P1" if len(h) % 2 == 0 else "P2"
    situation = {
        "": "첫 액션",
        "p": "P1 체크 후",
        "b": "P1 벳에 대응",
        "pb": "P2 벳에 대응",
    }[h]
    acts = ("체크", "벳") if h in ("", "p") else ("폴드", "콜")
    return who, card, situation, acts


if __name__ == "__main__":
    random.seed(0)
    ITER = 200_000
    value = train(ITER)

    print(f"반복 {ITER:,}회\n")
    print(f"{'플레이어':<6}{'카드':<5}{'상황':<14}전략")
    for key in sorted(nodes, key=lambda k: (len(k[1:]), k[1:], "JQK".index(k[0]))):
        who, card, situation, acts = label(key)
        s = nodes[key].average_strategy()
        print(f"{who:<8}{card:<6}{situation:<14}{acts[0]} {s[0]:5.1%}  {acts[1]} {s[1]:5.1%}")

    # ---- 이론값과 비교 ----
    j_bet = nodes["J"].average_strategy()[1]
    q_bet = nodes["Q"].average_strategy()[1]
    k_bet = nodes["K"].average_strategy()[1]
    print("\n[검증]")
    print(f"P1 게임 가치      : {value:+.4f}   (이론 -0.0556)")
    print(f"P1 Q 먼저 벳      : {q_bet:.1%}     (이론 0%)")
    print(f"P1 K 벳 / J 벳    : {k_bet / j_bet if j_bet else float('nan'):.2f}      (이론 3.00)")
