/**
 * Tauri command 的类型化封装 — 与 src-tauri/src/commands.rs 一一对应。
 * 参数名:Tauri 把 Rust 形参转 camelCase;结构体字段保持 serde snake_case。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  ErrorTag,
  LevelId,
  LevelSpec,
  Mode,
  Outcome,
  Sentence,
  Session,
  SrsState,
  StatsSummary,
  Verdict,
} from "@sentenceflow/ui";

/* ---------- settings (src-tauri/src/settings.rs) ---------- */

export type ChannelId = "opencode" | "deepseek" | "zen" | "ollama";

export interface Settings {
  practice: {
    strict_typing: boolean;
    auto_speak_answer: boolean;
    hide_chinese: boolean;
    daily_new: number | null;
    reorder_first: boolean | null;
  };
  sound: {
    accent: "gb" | "us";
    rate: number;
    key_sound: "off" | "soft" | "mechanical";
    fx_volume: number;
  };
  appearance: {
    theme: "light" | "dark" | "system";
    paper: boolean;
    practice_font_size: "small" | "medium" | "large";
  };
  accessibility: {
    reduce_motion: "system" | "on" | "off";
    dyslexic_font: boolean;
    high_contrast: boolean;
    color_blind_friendly: boolean;
  };
  ai: {
    channel: ChannelId | null;
    model: string | null;
    per_run_budget_cny: number;
    monthly_reminder_cny: number | null;
    price_override: { prompt_per_m: number; completion_per_m: number } | null;
    opencode_bin: string | null;
    proxy_url: string | null;
  };
  level: LevelId | null;
}

/* ---------- licensing ---------- */

export type LicenseState =
  | { kind: "licensed"; email_masked: string; edition: string; major_max: number }
  | { kind: "trial"; days_left: number }
  | { kind: "lapsed"; clock_rollback: boolean; daily_limit: number };

/* ---------- channels ---------- */

export interface ModelInfo {
  id: string;
  display_name: string;
  terms_note: string;
}

export type ChannelStatus =
  | { state: "not_installed" }
  | { state: "not_authed" }
  | { state: "ready"; models: ModelInfo[] }
  | { state: "error"; message: string };

/* ---------- workshop ---------- */

export interface JobParams {
  scene: string;
  level: string;
  total_sentences: number;
  microbatch: number;
  channel: string;
  model: string;
}

export type BatchState = "pending" | "running" | "done" | "failed";
export type JobState = "running" | "paused" | "completed" | "cancelled";

export interface GenJob {
  job_id: number;
  params: JobParams;
  state: JobState;
  batches: BatchState[];
  produced: number;
  created_at: number;
}

export type WorkshopCard =
  | { kind: "accepted"; job_id: number; sentence: Sentence }
  | { kind: "repairing"; job_id: number; en: string }
  | { kind: "discarded"; job_id: number; en: string; reason: string; recoverable: boolean };

export interface WorkshopProgress {
  job_id: number;
  batches: BatchState[];
  produced: number;
  state: JobState;
}

export interface WorkshopDone {
  job_id: number;
  state: JobState;
  produced: number;
  discarded: number;
  summary: string;
}

/* ---------- misc payloads ---------- */

export interface Bootstrap {
  specs: LevelSpec[];
  license: LicenseState;
  settings: Settings;
  content_rev: string | null;
  sentence_count: number;
}

export interface TodayOverview {
  due_count: number;
  new_available: number;
  streak_days: number;
  practiced_today: number;
}

export interface AttemptReport {
  sentence_id: number;
  mode: Mode;
  outcome: Outcome;
  dur_ms: number;
  errors: number;
  wpm: number;
  error_tags: ErrorTag[];
  tz_offset_secs: number;
}

export interface AttemptAck {
  srs: SrsState;
  lapsed_remaining: number | null;
}

export interface SpendSummary {
  today_requests: number;
  today_cost: number;
  month_requests: number;
  month_cost: number;
}

export interface BenchScore {
  model: string;
  score: number;
  json_ok_rate: number;
  pass_rate: number;
  over_level_rate: number;
  latency_ms: number;
}

export interface CmdError {
  code: string;
  message: string;
}

export const tzOffsetSecs = () => -new Date().getTimezoneOffset() * 60;

/* ---------- commands ---------- */

export const ipc = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),

  getLicenseState: () => invoke<LicenseState>("get_license_state"),
  activateLicense: (sflic: string) => invoke<LicenseState>("activate_license", { sflic }),
  exportLicense: () => invoke<string>("export_license"),

  listScenes: (level: LevelId) => invoke<string[]>("list_scenes", { level }),
  listSentences: (level: LevelId, scene?: string) =>
    invoke<Sentence[]>("list_sentences", { level, scene: scene ?? null }),
  getSentence: (id: number) => invoke<Sentence | null>("get_sentence", { id }),
  deleteUserSentence: (id: number) => invoke<void>("delete_user_sentence", { id }),
  importTabSentences: (level: LevelId, text: string) =>
    invoke<number>("import_tab_sentences", { level, text }),

  todayOverview: (level: LevelId) =>
    invoke<TodayOverview>("today_overview", { level, tzOffsetSecs: tzOffsetSecs() }),
  startSession: (level: LevelId) =>
    invoke<Session>("start_session", { level, tzOffsetSecs: tzOffsetSecs() }),
  startCustomSession: (ids: number[]) => invoke<Session>("start_custom_session", { ids }),
  submitAttempt: (report: AttemptReport) => invoke<AttemptAck>("submit_attempt", { report }),
  judgeText: (sentenceId: number, input: string) =>
    invoke<Verdict>("judge_text", { sentenceId, input }),

  wrongbook: () => invoke<number[]>("wrongbook"),
  favorites: () => invoke<number[]>("favorites"),
  favoriteToggle: (sentenceId: number, on: boolean) =>
    invoke<void>("favorite_toggle", { sentenceId, on }),

  getStats: () => invoke<StatsSummary>("get_stats", { tzOffsetSecs: tzOffsetSecs() }),
  importTrialProgress: (exportJson: unknown) =>
    invoke<[number, number]>("import_trial_progress", { export: exportJson }),

  probeChannel: (channel: ChannelId) => invoke<ChannelStatus>("probe_channel", { channel }),
  testChannelKey: (channel: ChannelId, key: string) =>
    invoke<ChannelStatus>("test_channel_key", { channel, key }),
  clearChannelKey: (channel: ChannelId) => invoke<void>("clear_channel_key", { channel }),
  spendSummary: () => invoke<SpendSummary>("spend_summary", { tzOffsetSecs: tzOffsetSecs() }),

  runBench: () => invoke<BenchScore[]>("run_bench"),
  workshopStart: (params: JobParams) => invoke<number>("workshop_start", { params }),
  workshopStop: () => invoke<void>("workshop_stop"),
  workshopResume: (jobId: number) => invoke<void>("workshop_resume", { jobId }),
  workshopJobs: () => invoke<GenJob[]>("workshop_jobs"),
  workshopRecover: (sentence: Sentence) => invoke<number>("workshop_recover", { sentence }),

  askAi: (sentenceId: number, question: string, history: Array<{ q: string; a: string }>) =>
    invoke<void>("ask_ai", { sentenceId, question, history }),
  weeklyReview: () => invoke<string>("weekly_review", { tzOffsetSecs: tzOffsetSecs() }),

  backupExport: (dest: string) => invoke<string>("backup_export", { dest }),
  backupRestore: (src: string, apply: boolean) =>
    invoke<{ srs_incoming: number; srs_newer: number; logs_incoming: number }>("backup_restore", {
      src,
      apply,
    }),
  ttsSpeak: (text: string, usAccent: boolean, rate: number) =>
    invoke<string | null>("tts_speak", { text, usAccent, rate }),
  diagnostics: () => invoke<Record<string, unknown>>("diagnostics"),
};

/* ---------- events ---------- */

export const events = {
  onWorkshopCard: (cb: (c: WorkshopCard) => void): Promise<UnlistenFn> =>
    listen<WorkshopCard>("workshop://card", (e) => cb(e.payload)),
  onWorkshopProgress: (cb: (p: WorkshopProgress) => void): Promise<UnlistenFn> =>
    listen<WorkshopProgress>("workshop://progress", (e) => cb(e.payload)),
  onWorkshopBackoff: (
    cb: (p: { job_id: number; wait_secs: number; suggest_switch: boolean }) => void,
  ): Promise<UnlistenFn> => listen("workshop://backoff", (e) => cb(e.payload as never)),
  onWorkshopMeter: (
    cb: (p: { job_id: number; cost_cny: number | null; warning: boolean }) => void,
  ): Promise<UnlistenFn> => listen("workshop://meter", (e) => cb(e.payload as never)),
  onWorkshopDone: (cb: (p: WorkshopDone) => void): Promise<UnlistenFn> =>
    listen<WorkshopDone>("workshop://done", (e) => cb(e.payload)),
  onWorkshopError: (
    cb: (p: { job_id: number; message: string; budget_stop?: boolean }) => void,
  ): Promise<UnlistenFn> => listen("workshop://error", (e) => cb(e.payload as never)),
  onAskChunk: (cb: (text: string) => void): Promise<UnlistenFn> =>
    listen<string>("ask://chunk", (e) => cb(e.payload)),
  onAskDone: (cb: () => void): Promise<UnlistenFn> => listen("ask://done", () => cb()),
  onAskError: (cb: (msg: string) => void): Promise<UnlistenFn> =>
    listen<string>("ask://error", (e) => cb(e.payload)),
};
