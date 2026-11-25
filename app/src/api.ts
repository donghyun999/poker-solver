// Rust 백엔드(src-tauri/src/lib.rs) 명령과 1:1 대응
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface SolveConfig {
  board: string;
  oopRange: string;
  ipRange: string;
  pot: number;
  stack: number;
  flopBet: string;
  turnBet: string;
  riverBet: string;
  flopRaise: string;
  turnRaise: string;
  riverRaise: string;
  maxRaises: number;
  allowAllin: boolean;
  compress: boolean;
}

export interface Estimate {
  oopCombos: number;
  ipCombos: number;
  actionNodes: number;
  chanceNodes: number;
  terminalNodes: number;
  memoryMb: number;
}

export interface Progress {
  iteration: number;
  exploitability: number;
  elapsedSecs: number;
}

export interface SolveDone extends Progress {
  ev: [number, number];
  cancelled: boolean;
}

export interface ClassRow {
  class: string;
  combos: number;
  freqs: number[];
  ev: number | null;
}

export interface NodeView {
  kind: "action" | "chance" | "fold" | "showdown";
  player: number;
  board: string[];
  pot: number;
  actions: string[];
  overall: number[];
  totalCombos: number;
  classes: ClassRow[];
  dealable: string[];
}

export interface Loaded {
  config: SolveConfig;
  iteration: number;
  exploitability: number;
  ev: [number, number];
}

/** 경로 한 걸음: 숫자 = 액션 번호, 문자열 = 카드 */
export type Step = number | string;

export const api = {
  estimate: (config: SolveConfig) => invoke<Estimate>("estimate", { config }),
  solve: (config: SolveConfig, maxIterations: number, targetExploitability: number, maxMemoryGb: number) =>
    invoke<SolveDone>("solve", { config, maxIterations, targetExploitability, maxMemoryGb }),
  cancel: () => invoke<void>("cancel"),
  node: (path: Step[]) => invoke<NodeView>("node", { path }),
  save: (path: string) => invoke<void>("save", { path }),
  load: (path: string) => invoke<Loaded>("load", { path }),
  onProgress: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("solve-progress", (e) => cb(e.payload)),
};
