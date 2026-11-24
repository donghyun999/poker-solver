import { useEffect, useState } from "react";
import { api, type NodeView, type Step } from "../api";
import { actionColors, pct } from "../poker";
import { CardChip, CardPicker } from "./Cards";
import { RangeGrid } from "./RangeGrid";

interface Crumb {
  step: Step;
  label: string;
}

const WHO = ["OOP", "IP"];

/** 솔브 결과 탐색: 액션을 눌러 트리를 내려가고, 노드마다 핸드별 전략을 그리드로 본다 */
export function Viewer({ version }: { version: number }) {
  const [path, setPath] = useState<Crumb[]>([]);
  const [node, setNode] = useState<NodeView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  // 새 결과가 오면 루트로
  useEffect(() => setPath([]), [version]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    api
      .node(path.map((c) => c.step))
      .then((n) => {
        if (!alive) return;
        setNode(n);
        setError(null);
      })
      .catch((e) => alive && setError(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [path, version]);

  if (error) return <div className="viewer empty-state">{error}</div>;
  if (!node) return <div className="viewer empty-state">불러오는 중…</div>;

  const colors = actionColors(node.actions);
  const detailClass = hover ?? selected;
  const detail = node.classes.find((c) => c.class === detailClass);
  const hasEv = node.classes.some((c) => c.ev !== null);

  return (
    <div className="viewer">
      <div className="breadcrumbs">
        <button className={path.length === 0 ? "crumb active" : "crumb"} onClick={() => setPath([])}>
          루트
        </button>
        {path.map((c, i) => (
          <span key={i}>
            <span className="crumb-sep">›</span>
            <button
              className={i === path.length - 1 ? "crumb active" : "crumb"}
              onClick={() => setPath(path.slice(0, i + 1))}
            >
              {c.label}
            </button>
          </span>
        ))}
        {loading && <span className="spinner">계산 중…</span>}
      </div>

      <div className="node-head">
        <div className="board">
          {node.board.map((c) => (
            <CardChip key={c} card={c} />
          ))}
        </div>
        <div className="node-meta">
          팟 <b>{node.pot.toFixed(2)}</b>
          {node.kind === "action" && (
            <>
              {" "}· <b>{WHO[node.player]}</b> 차례 · 콤보 {node.totalCombos.toFixed(1)}
            </>
          )}
        </div>
      </div>

      {node.kind === "fold" && <div className="terminal">{WHO[node.player]} 폴드로 종료</div>}
      {node.kind === "showdown" && <div className="terminal">쇼다운</div>}
      {node.kind === "chance" && (
        <div>
          <div className="hint">다음 카드를 고르세요</div>
          <CardPicker dealable={node.dealable} onPick={(c) => setPath([...path, { step: c, label: c }])} />
        </div>
      )}

      {node.kind === "action" && (
        <>
          <div className="actions">
            {node.actions.map((a, i) => (
              <button
                key={i}
                className="action-btn"
                style={{ borderColor: colors[i] }}
                onClick={() => setPath([...path, { step: i, label: `${WHO[node.player]} ${a}` }])}
              >
                <span className="swatch" style={{ background: colors[i] }} />
                {a}
                <span className="freq">{pct(node.overall[i] ?? 0)}</span>
              </button>
            ))}
          </div>
          <div className="overall-bar">
            {node.overall.map((f, i) => (
              <div key={i} style={{ width: `${f * 100}%`, background: colors[i] }} />
            ))}
          </div>

          <div className="grid-area">
            <RangeGrid
              rows={node.classes}
              colors={colors}
              selected={selected}
              onHover={setHover}
              onSelect={(c) => setSelected(c === selected ? null : c)}
            />
            <div className="detail">
              {detail ? (
                <>
                  <h3>{detail.class}</h3>
                  <div className="muted">콤보 {detail.combos.toFixed(2)}</div>
                  <table>
                    <tbody>
                      {node.actions.map((a, i) => (
                        <tr key={i}>
                          <td>
                            <span className="swatch" style={{ background: colors[i] }} />
                            {a}
                          </td>
                          <td className="num">{pct(detail.freqs[i])}</td>
                        </tr>
                      ))}
                      {detail.ev !== null && (
                        <tr>
                          <td>EV</td>
                          <td className="num">{detail.ev.toFixed(2)}</td>
                        </tr>
                      )}
                    </tbody>
                  </table>
                </>
              ) : (
                <div className="muted">
                  칸에 마우스를 올리면 상세가 보입니다.
                  <br />
                  막대 높이 = 이 노드까지 남은 콤보 비율
                  {hasEv && (
                    <>
                      <br />
                      EV는 루트에서만 표시
                    </>
                  )}
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
