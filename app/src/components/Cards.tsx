import { suitClass, suitSymbol, RANKS } from "../poker";

export function CardChip({ card }: { card: string }) {
  return (
    <span className={"card " + suitClass(card)}>
      {card[0]}
      {suitSymbol(card)}
    </span>
  );
}

/** 찬스 노드: 나올 수 있는 카드 중에서 고르기 */
export function CardPicker({ dealable, onPick }: { dealable: string[]; onPick: (card: string) => void }) {
  const set = new Set(dealable);
  return (
    <div className="card-picker">
      {["s", "h", "d", "c"].map((s) => (
        <div key={s} className="card-picker-row">
          {[...RANKS].map((r) => {
            const c = r + s;
            const ok = set.has(c);
            return (
              <button key={c} className={"card pick " + suitClass(c)} disabled={!ok} onClick={() => onPick(c)}>
                {r}
                {suitSymbol(c)}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}
