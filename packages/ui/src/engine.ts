/**
 * CoreEngine — sf-core 的宿主无关访问接口。
 * 桌面端由 Tauri command 实现,试用版由 sf-wasm 实现;组件层只见此接口。
 */

import type {
  JudgePolicy,
  LevelSpec,
  LogRow,
  Mode,
  Outcome,
  Session,
  SrsState,
  StatsSummary,
  Verdict,
} from "./types";

export interface CoreEngine {
  parseLevelSpec(yaml: string): Promise<LevelSpec>;
  buildSession(
    due: Array<[number, SrsState]>,
    newPool: number[],
    spec: LevelSpec,
    now: number,
    seed: bigint,
    dailyNewOverride?: number,
  ): Promise<Session>;
  applyOutcome(
    state: SrsState,
    outcome: Outcome,
    mode: Mode,
    spec: LevelSpec,
    now: number,
  ): Promise<SrsState>;
  newSrsState(now: number): Promise<SrsState>;
  judge(input: string, targets: string[], policy?: JudgePolicy): Promise<Verdict>;
  foldStats(logs: LogRow[], tzOffsetSecs: number): Promise<StatsSummary>;
}

export interface SpeakOptions {
  /** 语速倍率 0.6–1.4 (§4.8) */
  rate?: number;
  /** "gb" 英音 / "us" 美音 */
  voice?: "gb" | "us";
}

export interface SpeechService {
  speak(text: string, options?: SpeakOptions): void;
  stop(): void;
}

/** 静音实现 — 无 TTS 环境的兜底 */
export const silentSpeech: SpeechService = {
  speak() {},
  stop() {},
};
