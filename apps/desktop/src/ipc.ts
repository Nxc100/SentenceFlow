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
    /** 所选模型的展示名(界面显示用,不露原始 id) */
    model_label: string | null;
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
  /** 国内直连不可用、需代理(channels.json 名单标注) */
  needs_proxy: boolean;
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
  /** level = 按等级练习句(默认);scenario = 场景对话(不分等级) */
  mode?: "level" | "scenario";
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

/** 生成过程活跃信号(慢通道上的"AI 正在工作"反馈) */
export interface WorkshopActivity {
  job_id: number;
  /** connect 连接中 / streaming 产出中(n=已接收字符) / repairing 修补中(n=句数) */
  phase: "connect" | "streaming" | "repairing";
  n: number;
  batch: number;
  batches: number;
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
  /** 场景练习:只记日志,不进等级复习队列 */
  skip_srs?: boolean;
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

/* ---------- 场景练习 (方案 §3.5) ---------- */

export interface ScenePackInfo {
  pack: string;
  name: string;
  category: string;
  intro: string;
  /** 参考难度(展示为阶段名;不构成任何约束) */
  reference_level: LevelId | null;
  sentence_count: number;
  /** true = 生成工坊产出的包(可删) */
  from_user: boolean;
  practiced: boolean;
  /** 从头数连续练过的句数(断点续练用) */
  practiced_count: number;
}

/* ---------- 定级测试 (sf-core placement,方案 §3.3) ---------- */

export type PlacementItem =
  | { kind: "vocab"; word: string }
  | { kind: "sentence"; sentence: Sentence; mode: Mode }
  | { kind: "grammar"; topic_zh: string; prompt_zh: string; stem: string; options: string[] };

export type PlacementAnswer =
  | { kind: "vocab"; known: boolean }
  | { kind: "sentence"; word_errors: number; seen_answer: boolean; dur_ms: number; wpm: number }
  | { kind: "grammar"; choice: number };

export interface PlacementResult {
  level: LevelId;
  vocab_est: number;
  sentence_accuracy: number;
  false_alarm_rate: number;
  low_confidence: boolean;
  grammar_notes: string[];
  taken_at: number;
}

export interface PlacementStep {
  item: PlacementItem | null;
  progress: number;
  result: PlacementResult | null;
}

/* ---------- AI 聊天 (chat.rs,doc/AI聊天模块-实现方案.md) ---------- */

export type ChatMode = "free" | "roleplay" | "agent";

export interface ChatThreadInfo {
  id: number;
  mode: ChatMode;
  title: string;
  role_id: string;
  workdir: string;
  /** 本会话固定的通道/模型(空串 = 跟随「设置 · AI 接入」) */
  channel: ChannelId | "";
  model: string;
  model_label: string;
  updated_at: number;
}

/** 删除会话的结果(智能体可选连带把工作目录移入回收站) */
export interface DeleteOutcome {
  workdir_trashed: boolean;
  note: string;
}

/** 纠错小卡:更好的说法 + 一句为什么 */
export interface FixCard {
  better: string;
  why: string;
}

/** 智能体的任务清单条目(opencode todowrite;每步推进重发完整清单) */
export interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed" | string;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  fix: FixCard | null;
  /** 这一轮的任务清单(空数组 = 没有) */
  todos: TodoItem[];
  ts: number;
}

export interface ChatTodoEvent {
  thread_id: number;
  todos: TodoItem[];
}

export interface ChatChunkEvent {
  thread_id: number;
  text: string;
}

export interface ChatToolEvent {
  thread_id: number;
  label: string;
  status: "pending" | "running" | "completed" | "error" | string;
  /** skill = 加载技能(🧠),tool = 普通工具(⚙) */
  kind: "skill" | "tool" | string;
}

/* ---------- opencode 技能 (skills.rs) ---------- */

/** active=opencode 已登记(AI 可自动调用) / unloaded=未登记(仍可手动注入) */
export type SkillState = "active" | "unloaded";

export interface SkillInfo {
  name: string;
  description: string;
  /** SKILL.md 绝对路径;内置技能为空串 */
  path: string;
  scope: "builtin" | "global" | "project" | string;
  source_label: string;
  state: SkillState;
  editable: boolean;
  /** 磁盘上同名副本份数(>1 表示装了多处,只有一份生效) */
  copies: number;
  /** 未加载时的人话原因(诊断得出才有值) */
  reason: string;
}

export interface SkillCatalog {
  skills: SkillInfo[];
  active_count: number;
  /** 取清单失败时的人话原因 */
  warning: string;
}

export interface SkillSource {
  name: string;
  description: string;
  body: string;
}

export interface ChatDoneEvent {
  thread_id: number;
  text: string;
  fix: FixCard | null;
  /** true = 被停止/超时截断的部分回复 */
  partial: boolean;
}

export interface ChatErrorEvent {
  thread_id: number;
  message: string;
  retry_after_secs: number | null;
}

/* ---------- opencode 一键安装 (installer.rs) ---------- */

export interface InstallProgress {
  phase: "resolve" | "download" | "extract" | "verify";
  received: number;
  total: number | null;
}

export interface InstallDone {
  version: string;
  bin_path: string;
  /** 安装目录已加入用户 PATH(新开终端可直接用 opencode 命令) */
  on_path: boolean;
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

  /** origin: "factory" 出厂库 / "user" 用户句集,缺省两库并集 */
  listScenes: (level: LevelId, origin?: "factory" | "user") =>
    invoke<string[]>("list_scenes", { level, origin: origin ?? null }),
  listSentences: (level: LevelId, scene?: string) =>
    invoke<Sentence[]>("list_sentences", { level, scene: scene ?? null }),
  getSentence: (id: number) => invoke<Sentence | null>("get_sentence", { id }),
  deleteUserSentence: (id: number) => invoke<void>("delete_user_sentence", { id }),
  /** 按场景批量删除「我的句集」里的句子,返回删除条数 */
  deleteUserSentencesByScene: (level: LevelId, scene: string) =>
    invoke<number>("delete_user_sentences_by_scene", { level, scene }),
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

  listScenePacks: () => invoke<ScenePackInfo[]>("list_scene_packs"),
  listPackSentences: (pack: string) => invoke<Sentence[]>("list_pack_sentences", { pack }),
  startScenarioSession: (pack: string) => invoke<Session>("start_scenario_session", { pack }),
  deleteUserScenePack: (pack: string) => invoke<number>("delete_user_scene_pack", { pack }),
  /** 批量删除自建场景包(出厂包自动跳过),返回删掉的句子总数 */
  deleteUserScenePacks: (packs: string[]) =>
    invoke<number>("delete_user_scene_packs", { packs }),

  placementStart: (allowListening: boolean) =>
    invoke<PlacementStep>("placement_start", { allowListening }),
  placementAnswer: (answer: PlacementAnswer) =>
    invoke<PlacementStep>("placement_answer", { answer }),
  placementLast: () => invoke<PlacementResult | null>("placement_last"),

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

  chatThreadCreate: (args: {
    mode: ChatMode;
    title: string;
    roleId?: string;
    roleSystem?: string;
    opener?: string;
    workdir?: string;
  }) =>
    invoke<ChatThreadInfo>("chat_thread_create", {
      mode: args.mode,
      title: args.title,
      roleId: args.roleId ?? "",
      roleSystem: args.roleSystem ?? "",
      opener: args.opener ?? "",
      workdir: args.workdir ?? "",
    }),
  chatThreads: () => invoke<ChatThreadInfo[]>("chat_threads"),
  chatHistory: (threadId: number) => invoke<ChatMessage[]>("chat_history", { threadId }),
  /** deleteWorkdir 仅对智能体会话有意义:把工作目录移入系统回收站 */
  chatThreadDelete: (threadId: number, deleteWorkdir = false) =>
    invoke<DeleteOutcome>("chat_thread_delete", { threadId, deleteWorkdir }),
  /** 本会话固定模型;channel 传 null = 恢复跟随设置。下一条消息起生效 */
  chatThreadSetModel: (
    threadId: number,
    channel: ChannelId | null,
    model: string,
    modelLabel: string,
  ) => invoke<ChatThreadInfo>("chat_thread_set_model", { threadId, channel, model, modelLabel }),
  chatSend: (threadId: number, text: string, fixEnabled: boolean) =>
    invoke<void>("chat_send", { threadId, text, fixEnabled }),
  /** [停止]:只停这一个会话的流,其他会话继续 */
  chatStop: (threadId: number) => invoke<void>("chat_stop", { threadId }),
  /** 仍在生成回复的会话 id(离开页面再回来时恢复「生成中」指示) */
  chatActiveThreads: () => invoke<number[]>("chat_active_threads"),
  /** skillPath 非空 = 手动触发型技能:后端把技能正文注入本轮消息 */
  agentSend: (threadId: number, text: string, skillPath = "") =>
    invoke<void>("agent_send", { threadId, text, skillPath }),

  /** 技能总目录:opencode 已登记的 + 磁盘上未登记的(带状态标注) */
  skillCatalog: (workdir: string) => invoke<SkillCatalog>("skill_catalog", { workdir }),
  skillSource: (path: string) => invoke<SkillSource>("skill_source", { path }),
  /** path 非空 = 改写已有技能(名字不可变);返回写入的文件路径 */
  skillSave: (args: {
    path?: string;
    scope: "global" | "project";
    workdir?: string;
    name: string;
    description: string;
    body: string;
  }) =>
    invoke<string>("skill_save", {
      path: args.path ?? "",
      scope: args.scope,
      workdir: args.workdir ?? "",
      name: args.name,
      description: args.description,
      body: args.body,
    }),
  /** 删除技能:整个技能目录移入回收站,返回被删目录 */
  skillDelete: (path: string) => invoke<string>("skill_delete", { path }),
  /** 系统文件夹选择器;null = 用户取消 */
  pickFolder: (title: string) => invoke<string | null>("pick_folder", { title }),

  backupExport: (dest: string) => invoke<string>("backup_export", { dest }),
  backupRestore: (src: string, apply: boolean) =>
    invoke<{ srs_incoming: number; srs_newer: number; logs_incoming: number }>("backup_restore", {
      src,
      apply,
    }),
  ttsSpeak: (text: string, usAccent: boolean, rate: number) =>
    invoke<string | null>("tts_speak", { text, usAccent, rate }),
  diagnostics: () => invoke<Record<string, unknown>>("diagnostics"),

  /** opencode 一键安装(免 Node/免终端);进度经 install://progress */
  opencodeInstall: () => invoke<InstallDone>("opencode_install"),
  /** 弹出独立控制台窗口运行 opencode auth login */
  opencodeLogin: () => invoke<void>("opencode_login"),
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
  onWorkshopActivity: (cb: (p: WorkshopActivity) => void): Promise<UnlistenFn> =>
    listen<WorkshopActivity>("workshop://activity", (e) => cb(e.payload)),
  onWorkshopDone: (cb: (p: WorkshopDone) => void): Promise<UnlistenFn> =>
    listen<WorkshopDone>("workshop://done", (e) => cb(e.payload)),
  onWorkshopError: (
    cb: (p: { job_id: number; message: string; budget_stop?: boolean }) => void,
  ): Promise<UnlistenFn> => listen("workshop://error", (e) => cb(e.payload as never)),
  onInstallProgress: (cb: (p: InstallProgress) => void): Promise<UnlistenFn> =>
    listen<InstallProgress>("install://progress", (e) => cb(e.payload)),
  onAskChunk: (cb: (text: string) => void): Promise<UnlistenFn> =>
    listen<string>("ask://chunk", (e) => cb(e.payload)),
  onAskDone: (cb: () => void): Promise<UnlistenFn> => listen("ask://done", () => cb()),
  onAskError: (cb: (msg: string) => void): Promise<UnlistenFn> =>
    listen<string>("ask://error", (e) => cb(e.payload)),
  onChatChunk: (cb: (p: ChatChunkEvent) => void): Promise<UnlistenFn> =>
    listen<ChatChunkEvent>("chat://chunk", (e) => cb(e.payload)),
  onChatTool: (cb: (p: ChatToolEvent) => void): Promise<UnlistenFn> =>
    listen<ChatToolEvent>("chat://tool", (e) => cb(e.payload)),
  onChatTodo: (cb: (p: ChatTodoEvent) => void): Promise<UnlistenFn> =>
    listen<ChatTodoEvent>("chat://todo", (e) => cb(e.payload)),
  onChatDone: (cb: (p: ChatDoneEvent) => void): Promise<UnlistenFn> =>
    listen<ChatDoneEvent>("chat://done", (e) => cb(e.payload)),
  onChatError: (cb: (p: ChatErrorEvent) => void): Promise<UnlistenFn> =>
    listen<ChatErrorEvent>("chat://error", (e) => cb(e.payload)),
};
