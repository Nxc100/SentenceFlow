/**
 * 与 Rust serde JSON 逐字段一致的类型定义(§7.3 双端逐比特一致)。
 * 变更必须与 crates/sf-core 的 serde 形状同步。
 */

export type LevelId = "L1" | "L2" | "L3" | "L4" | "L5" | "L6";

/** 词性短码 — sf-core PosTag 的 serde rename */
export type PosTag =
  | "pron" | "n" | "v" | "aux" | "modal" | "adj" | "wh"
  | "adv" | "prep" | "art" | "conj" | "num" | "propn" | "part";

/** 成分短码 — sf-core RoleTag 的 serde rename */
export type RoleTag =
  | "subj" | "pred" | "link" | "obj" | "comp" | "advl" | "objc" | "marker";

export interface Word {
  w: string;
  /** 英式 IPA,无斜杠 */
  ipa: string;
  pos: PosTag;
}

export interface Chunk {
  r: RoleTag;
  /** 指向 words 的 0 起索引 */
  i: number[];
}

export interface Sentence {
  id: number;
  level: LevelId;
  scene: string;
  func: string;
  pattern: string;
  zh: string;
  en: string;
  /** 句末标点,直显不输入 */
  punct: string;
  words: Word[];
  chunks: Chunk[];
  note: string;
  simhash: number;
}

/* ---------- LevelSpec (§4.9) ---------- */

export type FlowKind = "reorder_then_typing" | "typing" | "mixed";
export type HintVisibility = "always" | "on_click" | "hidden";

export interface HintSpec {
  ipa: HintVisibility;
  first_letter: boolean;
  zh_hideable: boolean;
}

export interface JudgeSpec {
  strict: boolean;
  track_article_preposition?: boolean;
}

export interface SrsSpec {
  daily_new_default: number;
  daily_new_range: [number, number];
  review_cap: number;
  box_intervals_days: [number, number, number, number];
  box5_recheck_days: number;
  listening_weight: number;
}

export interface PracticeSpec {
  flow: FlowKind;
  review_listening_ratio: number;
  dictation_min_box: number;
  hints: HintSpec;
  judge: JudgeSpec;
  srs: SrsSpec;
}

export interface LevelSpec {
  id: LevelId;
  cefr: string;
  vocab_band: number;
  max_words: number;
  grammar_whitelist: string[];
  can_do: string[];
  practice: PracticeSpec;
}

/* ---------- SRS ---------- */

export type Mode = "typing" | "reorder" | "listening" | "dictation";

export interface SrsState {
  box_idx: number;
  progress: number;
  due_at: number;
  err: number;
  last_mode: Mode | null;
  last_at: number;
  seen_answer: boolean;
  marked_unfamiliar: boolean;
}

export type Outcome =
  | { kind: "correct"; seen_answer: boolean }
  | { kind: "wrong" }
  | { kind: "mark_mastered" }
  | { kind: "mark_unfamiliar" };

/* ---------- Session ---------- */

export interface SessionItem {
  sentence_id: number;
  mode: Mode;
  is_review: boolean;
  reorder_first: boolean;
}

export interface Session {
  items: SessionItem[];
  overflow_reviews: number;
}

/* ---------- Judge ---------- */

export interface JudgePolicy {
  case_insensitive: boolean;
  ignore_punct: boolean;
}

export type WordVerdict =
  | { kind: "correct"; word: string }
  | { kind: "wrong"; expected: string; got: string }
  | { kind: "missing"; expected: string }
  | { kind: "extra"; got: string };

export interface Verdict {
  words: WordVerdict[];
  correct: boolean;
  errors: number;
}

/* ---------- 统计 ---------- */

export type LogResult = "correct" | "wrong";

export type ErrorTag =
  | { t: "pos"; v: PosTag }
  | { t: "role"; v: RoleTag };

export interface LogRow {
  ts: number;
  sentence_id: number;
  mode: Mode;
  result: LogResult;
  dur_ms: number;
  errors: number;
  wpm: number;
  seen_answer?: boolean;
  error_tags?: ErrorTag[];
}

export interface DayStats {
  attempts: number;
  correct: number;
  practice_ms: number;
  avg_wpm: number;
  accuracy: number;
}

export interface WeakPoint<T> {
  tag: T;
  errors: number;
  share: number;
}

export interface StatsSummary {
  /** BTreeMap<i64, DayStats> → 键为本地日索引(字符串化数字) */
  days: Record<string, DayStats>;
  streak_days: number;
  best_correct_streak: number;
  weak_pos: WeakPoint<PosTag>[];
  weak_roles: WeakPoint<RoleTag>[];
  total_attempts: number;
  total_correct: number;
}

/* ---------- 授权/试用 ---------- */

export type TrialVerdict =
  | { kind: "active"; days_left: number }
  | { kind: "expired" }
  | { kind: "expired_clock_rollback" };
