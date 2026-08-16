/**
 * sf-wasm 引擎装载:JSON 字符串 ABI → CoreEngine(§7.9)。
 * pkg 缺失(未跑 `npm run build:wasm`)时回退 mockEngine 并在控制台警告。
 */

import type {
  CoreEngine,
  JudgePolicy,
  LevelSpec,
  LogRow,
  Mode,
  Outcome,
  Session,
  SrsState,
  StatsSummary,
  Verdict,
} from "@sentenceflow/ui";
import { mockEngine } from "./mockEngine";

interface WasmModule {
  default: (input?: unknown) => Promise<unknown>;
  parse_level_spec(yaml: string): string;
  build_session(
    due: string,
    pool: string,
    spec: string,
    now: bigint,
    seed: bigint,
    dailyNew?: number,
  ): string;
  apply_outcome(state: string, outcome: string, mode: string, spec: string, now: bigint): string;
  new_srs_state(now: bigint): string;
  judge(input: string, targets: string, policy?: string): string;
  fold_stats(logs: string, tz: number): string;
}

function wrap(m: WasmModule): CoreEngine {
  return {
    async parseLevelSpec(yaml: string): Promise<LevelSpec> {
      return JSON.parse(m.parse_level_spec(yaml));
    },
    async buildSession(
      due: Array<[number, SrsState]>,
      newPool: number[],
      spec: LevelSpec,
      now: number,
      seed: bigint,
      dailyNewOverride?: number,
    ): Promise<Session> {
      return JSON.parse(
        m.build_session(
          JSON.stringify(due),
          JSON.stringify(newPool),
          JSON.stringify(spec),
          BigInt(now),
          seed,
          dailyNewOverride,
        ),
      );
    },
    async applyOutcome(
      state: SrsState,
      outcome: Outcome,
      mode: Mode,
      spec: LevelSpec,
      now: number,
    ): Promise<SrsState> {
      return JSON.parse(
        m.apply_outcome(
          JSON.stringify(state),
          JSON.stringify(outcome),
          JSON.stringify(mode),
          JSON.stringify(spec),
          BigInt(now),
        ),
      );
    },
    async newSrsState(now: number): Promise<SrsState> {
      return JSON.parse(m.new_srs_state(BigInt(now)));
    },
    async judge(input: string, targets: string[], policy?: JudgePolicy): Promise<Verdict> {
      return JSON.parse(
        m.judge(input, JSON.stringify(targets), policy ? JSON.stringify(policy) : undefined),
      );
    },
    async foldStats(logs: LogRow[], tzOffsetSecs: number): Promise<StatsSummary> {
      return JSON.parse(m.fold_stats(JSON.stringify(logs), tzOffsetSecs));
    },
  };
}

let cached: Promise<CoreEngine> | null = null;

export function loadEngine(): Promise<CoreEngine> {
  cached ??= (async () => {
    try {
      // 运行时从静态资源加载(dev: public/;build:wasm 负责放置产物)。
      // 路径经变量传入,绕过打包器与 TS 的静态解析 — 这是运行时 URL。
      const wasmUrl = "/wasm/sf_wasm.js";
      const mod = (await import(/* @vite-ignore */ wasmUrl)) as unknown as WasmModule;
      await mod.default();
      return wrap(mod);
    } catch (err) {
      console.warn(
        "[sf] sf-wasm pkg 未构建(npm run build:wasm),已回退 DEV mock 引擎 — 仅供开发预览。",
        err,
      );
      return mockEngine;
    }
  })();
  return cached;
}
