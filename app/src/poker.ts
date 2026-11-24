export const RANKS = "AKQJT98765432";

/** 13×13 그리드 칸 → 핸드 클래스. 대각선 = 페어, 오른쪽 위 = 수딧, 왼쪽 아래 = 오프수트 */
export function gridClass(row: number, col: number): string {
  const a = RANKS[row];
  const b = RANKS[col];
  if (row === col) return a + b;
  return row < col ? a + b + "s" : b + a + "o";
}

const SUIT_SYMBOL: Record<string, string> = { s: "♠", h: "♥", d: "♦", c: "♣" };

export function suitSymbol(card: string): string {
  return SUIT_SYMBOL[card[1]] ?? card[1];
}

export function suitClass(card: string): string {
  return "suit-" + card[1];
}

/** 액션 라벨 → 색. 공격적 액션은 순서대로 진해짐 */
export function actionColors(actions: string[]): string[] {
  const aggressive = ["#f2a65a", "#e8743b", "#d9483b", "#b02e3a", "#8a1f3a"];
  let k = 0;
  return actions.map((a) => {
    if (a.startsWith("Fold")) return "#4f86e8";
    if (a.startsWith("Check") || a.startsWith("Call")) return "#3fae74";
    if (a.startsWith("AllIn")) return "#6d1a36";
    return aggressive[Math.min(k++, aggressive.length - 1)];
  });
}

export const pct = (x: number) => (x * 100).toFixed(1) + "%";

export const PRESETS: { name: string; oop: string; ip: string; pot: number; stack: number }[] = [
  {
    name: "BTN vs BB 싱글레이즈 (100bb)",
    oop: "22-JJ,A2s-AJs,K2s+,Q4s+,J6s+,T6s+,95s+,85s+,74s+,63s+,53s+,43s,A2o-AJo,K8o+,Q9o+,J9o+,T8o+,98o",
    ip: "22+,A2s+,K2s+,Q5s+,J7s+,T7s+,96s+,86s+,75s+,64s+,54s,A2o+,K9o+,Q9o+,J9o+,T9o",
    pot: 6,
    stack: 97,
  },
  {
    name: "리버 토이 게임 (너트+에어 vs 블러프캐처)",
    oop: "77,54s",
    ip: "88",
    pot: 10,
    stack: 100,
  },
];
