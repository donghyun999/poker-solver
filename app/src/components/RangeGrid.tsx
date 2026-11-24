import type { ClassRow } from "../api";
import { gridClass, pct } from "../poker";

interface Props {
  rows: ClassRow[];
  colors: string[];
  selected: string | null;
  onHover: (cls: string | null) => void;
  onSelect: (cls: string) => void;
}

/** 13×13 핸드 그리드. 칸마다 액션 빈도를 가로 막대로 쌓아서 표시 */
export function RangeGrid({ rows, colors, selected, onHover, onSelect }: Props) {
  const byClass = new Map(rows.map((r) => [r.class, r]));
  const maxCombos = Math.max(1, ...rows.map((r) => r.combos));

  const cells = [];
  for (let i = 0; i < 13; i++) {
    for (let j = 0; j < 13; j++) {
      const cls = gridClass(i, j);
      const row = byClass.get(cls);
      cells.push(
        <button
          key={cls}
          className={"grid-cell" + (row ? "" : " empty") + (selected === cls ? " selected" : "")}
          onMouseEnter={() => onHover(row ? cls : null)}
          onMouseLeave={() => onHover(null)}
          onClick={() => row && onSelect(cls)}
          title={row ? `${cls}  ${row.freqs.map(pct).join(" / ")}` : cls}
        >
          {row && (
            // 막대 높이 = 이 노드에 도달하는 콤보 비율 (많이 남은 핸드일수록 꽉 참)
            <div className="bars" style={{ height: `${Math.max(18, (row.combos / maxCombos) * 100)}%` }}>
              {row.freqs.map((f, a) => (
                <div key={a} style={{ width: `${f * 100}%`, background: colors[a] }} />
              ))}
            </div>
          )}
          <span className="grid-label">{cls}</span>
        </button>,
      );
    }
  }
  return <div className="range-grid">{cells}</div>;
}
