import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type Estimate, type Progress, type SolveConfig, type SolveDone } from "../api";
import { PRESETS } from "../poker";

interface Props {
  onSolved: () => void;
}

const DEFAULT: SolveConfig = {
  board: "Kh 9d 4c 2s",
  oopRange: PRESETS[0].oop,
  ipRange: PRESETS[0].ip,
  pot: 20,
  stack: 87,
  flopBet: "33,75",
  turnBet: "50,100",
  riverBet: "50,100",
  flopRaise: "100",
  turnRaise: "100",
  riverRaise: "100",
  maxRaises: 1,
  allowAllin: true,
  compress: false,
};

const FILTERS = [{ name: "Solver result", extensions: ["pks"] }];

export function ConfigPanel({ onSolved }: Props) {
  const [cfg, setCfg] = useState<SolveConfig>(DEFAULT);
  const [iters, setIters] = useState(1000);
  const [target, setTarget] = useState(0.5);
  const [maxMem, setMaxMem] = useState(12);
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [done, setDone] = useState<SolveDone | null>(null);
  const [busy, setBusy] = useState<"" | "solve" | "estimate" | "file">("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const un = api.onProgress(setProgress);
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 설정이 바뀌면 예상치 초기화
  useEffect(() => setEstimate(null), [cfg]);

  const set = <K extends keyof SolveConfig>(k: K, v: SolveConfig[K]) => setCfg({ ...cfg, [k]: v });

  const run = async <T,>(kind: typeof busy, f: () => Promise<T>) => {
    setBusy(kind);
    setError(null);
    try {
      return await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  };

  const doEstimate = () => run("estimate", async () => setEstimate(await api.estimate(cfg)));

  const doSolve = () =>
    run("solve", async () => {
      setProgress(null);
      setDone(null);
      const r = await api.solve(cfg, iters, target, maxMem);
      setDone(r);
      onSolved();
    });

  const doSave = () =>
    run("file", async () => {
      const path = await save({ filters: FILTERS, defaultPath: "solve.pks" });
      if (path) await api.save(path);
    });

  const doLoad = () =>
    run("file", async () => {
      const path = await open({ filters: FILTERS, multiple: false });
      if (!path) return;
      const r = await api.load(path as string);
      setCfg(r.config);
      setDone({ iteration: r.iteration, exploitability: r.exploitability, elapsedSecs: 0, ev: r.ev, cancelled: false });
      setProgress(null);
      onSolved();
    });

  const street = cfg.board.replace(/[^A-Za-z0-9]/g, "").length / 2;

  return (
    <div className="config">
      <h2>스팟 설정</h2>

      <label>
        프리셋
        <select
          onChange={(e) => {
            const p = PRESETS[Number(e.target.value)];
            if (p) setCfg({ ...cfg, oopRange: p.oop, ipRange: p.ip, pot: p.pot, stack: p.stack });
          }}
          defaultValue=""
        >
          <option value="" disabled>
            레인지 프리셋 선택…
          </option>
          {PRESETS.map((p, i) => (
            <option key={i} value={i}>
              {p.name}
            </option>
          ))}
        </select>
      </label>

      <label>
        보드 (3~5장)
        <input value={cfg.board} onChange={(e) => set("board", e.target.value)} placeholder="Kh 9d 4c" />
      </label>
      <label>
        OOP 레인지
        <textarea rows={3} value={cfg.oopRange} onChange={(e) => set("oopRange", e.target.value)} />
      </label>
      <label>
        IP 레인지
        <textarea rows={3} value={cfg.ipRange} onChange={(e) => set("ipRange", e.target.value)} />
      </label>

      <div className="row2">
        <label>
          시작 팟
          <input type="number" value={cfg.pot} onChange={(e) => set("pot", Number(e.target.value))} />
        </label>
        <label>
          유효 스택
          <input type="number" value={cfg.stack} onChange={(e) => set("stack", Number(e.target.value))} />
        </label>
      </div>

      <div className="sizes">
        <div className="muted small">벳 · 레이즈 사이즈 (팟 대비 %, 쉼표 구분)</div>
        {street <= 3 && (
          <div className="row2">
            <label>
              플랍 벳
              <input value={cfg.flopBet} onChange={(e) => set("flopBet", e.target.value)} />
            </label>
            <label>
              플랍 레이즈
              <input value={cfg.flopRaise} onChange={(e) => set("flopRaise", e.target.value)} />
            </label>
          </div>
        )}
        {street <= 4 && (
          <div className="row2">
            <label>
              턴 벳
              <input value={cfg.turnBet} onChange={(e) => set("turnBet", e.target.value)} />
            </label>
            <label>
              턴 레이즈
              <input value={cfg.turnRaise} onChange={(e) => set("turnRaise", e.target.value)} />
            </label>
          </div>
        )}
        <div className="row2">
          <label>
            리버 벳
            <input value={cfg.riverBet} onChange={(e) => set("riverBet", e.target.value)} />
          </label>
          <label>
            리버 레이즈
            <input value={cfg.riverRaise} onChange={(e) => set("riverRaise", e.target.value)} />
          </label>
        </div>
        <div className="row2">
          <label>
            최대 레이즈
            <input type="number" value={cfg.maxRaises} onChange={(e) => set("maxRaises", Number(e.target.value))} />
          </label>
        </div>
        <label className="check">
          <input type="checkbox" checked={cfg.allowAllin} onChange={(e) => set("allowAllin", e.target.checked)} />
          올인 선택지 추가
        </label>
        <label className="check">
          <input type="checkbox" checked={cfg.compress} onChange={(e) => set("compress", e.target.checked)} />
          16비트 압축 (메모리 절반, 플랍 추천)
        </label>
      </div>

      <div className="row3">
        <label>
          최대 반복
          <input type="number" value={iters} onChange={(e) => setIters(Number(e.target.value))} />
        </label>
        <label>
          목표 %
          <input type="number" step="0.1" value={target} onChange={(e) => setTarget(Number(e.target.value))} />
        </label>
        <label>
          메모리 GB
          <input type="number" value={maxMem} onChange={(e) => setMaxMem(Number(e.target.value))} />
        </label>
      </div>

      <div className="buttons">
        <button onClick={doEstimate} disabled={busy !== ""}>
          {busy === "estimate" ? "계산 중…" : "트리 크기 확인"}
        </button>
        {busy === "solve" ? (
          <button className="danger" onClick={() => api.cancel()}>
            중단
          </button>
        ) : (
          <button className="primary" onClick={doSolve} disabled={busy !== ""}>
            솔브
          </button>
        )}
      </div>
      <div className="buttons">
        <button onClick={doSave} disabled={busy !== "" || !done}>
          결과 저장
        </button>
        <button onClick={doLoad} disabled={busy !== ""}>
          불러오기
        </button>
      </div>

      {estimate && (
        <div className="info">
          콤보 OOP {estimate.oopCombos} / IP {estimate.ipCombos}
          <br />
          액션 노드 {estimate.actionNodes.toLocaleString()} · 찬스 {estimate.chanceNodes.toLocaleString()}
          <br />
          메모리 <b>{estimate.memoryMb >= 1000 ? (estimate.memoryMb / 1000).toFixed(1) + " GB" : estimate.memoryMb.toFixed(0) + " MB"}</b>
        </div>
      )}
      {busy === "solve" && (
        <div className="info">
          {progress
            ? `반복 ${progress.iteration} · 익스플로이터빌리티 ${progress.exploitability.toFixed(3)}% · ${progress.elapsedSecs.toFixed(1)}초`
            : "트리 만드는 중…"}
        </div>
      )}
      {done && busy !== "solve" && (
        <div className="info ok">
          {done.cancelled ? "중단됨" : "완료"} · 반복 {done.iteration} · 익스플로이터빌리티{" "}
          {done.exploitability.toFixed(3)}%{done.elapsedSecs > 0 && ` · ${done.elapsedSecs.toFixed(1)}초`}
          <br />
          EV OOP {done.ev[0].toFixed(2)} / IP {done.ev[1].toFixed(2)}
        </div>
      )}
      {error && <div className="info err">{error}</div>}
    </div>
  );
}
