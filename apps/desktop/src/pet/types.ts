/**
 * AI 萌宠前端类型层 —— 与 `src-tauri/src/pet/**` 及 `sf-pet` 的 Serialize 结构一一对应。
 *
 * 两套线上命名并存,都是有意的:
 * * `PetSettings` 用 **snake_case** —— 它是句流 `Settings` 的一个分节,
 *   与 `practice`/`appearance` 等分节同一约定;
 * * PetKit 契约与视图结构用 **camelCase** —— `pet.json` 的 `mirrorOf` 等字段
 *   是开放素材包规范的一部分,改名就不再是同一个 PetKit。
 */

// ---------------------------------------------------------------- 状态契约

export type StateKey =
  | "idle"
  | "walk-right"
  | "walk-left"
  | "happy"
  | "eat"
  | "sleep"
  | "remind"
  | "drag";

export const ALL_STATES: StateKey[] = [
  "idle",
  "walk-right",
  "walk-left",
  "happy",
  "eat",
  "sleep",
  "remind",
  "drag",
];

export const STATE_NAMES: Record<StateKey, string> = {
  idle: "待机",
  "walk-right": "右走",
  "walk-left": "左走",
  happy: "开心",
  eat: "吃东西",
  sleep: "睡觉",
  remind: "提醒",
  drag: "被拎起",
};

// ---------------------------------------------------------------- PetKit

export interface StateClip {
  row: number;
  frames: number;
  fps: number;
  loop: boolean;
  mirrorOf?: string;
  /** 播放模式:缺省按启发式(idle/sleep→pingpong 消接缝,walk→loop,happy/eat→once)。 */
  playMode?: "loop" | "pingpong" | "once";
  /** 逐帧时长(秒)覆盖 fps 的匀速播放,用于关键帧停顿。缺省匀速。 */
  frameDurations?: number[];
  /** idle 行的闭眼特殊帧列索引。缺省无烘焙闭眼帧,运行时用双渲染法。 */
  blinkFrame?: number;
  /** 每步幅像素(逻辑格坐标,速度-步频锁定,治滑步)。缺省按单元格宽启发。 */
  stride?: number;
  /** 素材来源:`"real"` = 导入的真素材(视频/网格),`"baked"` = 骨骼烘焙产物。
   *  走路据此决定播真帧还是走 cutout 分层剪纸。缺省视为 baked。 */
  source?: "real" | "baked";
}

export interface Anchor {
  x: number;
  y: number;
}

export interface PetSpec {
  spec: string;
  name: string;
  author: string;
  cell: { w: number; h: number };
  sheet: string;
  chroma?: string;
  states: Record<string, StateClip>;
  tier?: string;
  created?: string;
  /** 校准锚点:眼×2/嘴/尾尖/脚底等,单元格坐标系。 */
  anchors?: Record<string, Anchor>;
  /** 生命感等级:C 纯程序 | B 校准 | A 骨骼烘焙(可缺省)。 */
  rigLevel?: "C" | "B" | "A";
  /** 输入类型判定,决定卡通眨眼是否默认关闭(照片类默认关闭)。 */
  inputType?: "cartoon_flat" | "cartoon_3d" | "portrait" | "pet_photo";
}

export interface ActivePetAssets {
  petId: string;
  spec: PetSpec;
  /** data URL */
  sheet: string;
}

export interface PetMeta {
  id: string;
  name: string;
  tier: string;
  states: string[];
  created?: string;
}

// ---------------------------------------------------------------- 设置 / 启动包

/** 句流 `Settings.pet` 分节(snake_case,与 Rust `PetSettings` 逐字对应)。 */
export interface PetSettings {
  enabled: boolean;
  scale: number;
  roam: boolean;
  click_through: boolean;
  night_sleep: boolean;
  performance_mode: boolean;
  sleep_after_min: number;
  props_enabled: boolean;
  watcher_enabled: boolean;
  ffmpeg_path: string | null;
  active_pet: string | null;
  first_hatch_done: boolean;
}

/** 句流主题 id,与 `packages/ui/src/tokens.css` 的 `[data-theme=…]` 同名。 */
export type PetTheme = "light" | "dark" | "macaron" | "system";

export interface PetBootstrap {
  settings: PetSettings;
  theme: PetTheme;
  active: ActivePetAssets | null;
}

// ---------------------------------------------------------------- 校验 / 向导

export interface Finding {
  level: "error" | "warning" | "info";
  code: string;
  message: string;
  state?: string;
  frame?: number;
}

export interface Report {
  ok: boolean;
  findings: Finding[];
}

export interface StripView {
  state: StateKey;
  stateName: string;
  frames: number;
  previews: string[];
  warnings: string[];
  background: string;
  recommended: [number, number];
}

export interface SessionView {
  strips: StripView[];
  tier: string;
}

export interface ComposeView {
  sheet: string;
  report: Report;
  tier: string;
}

export interface HatchResult {
  ok: boolean;
  petId: string | null;
  report: Report;
}

export interface EvolveResult {
  ok: boolean;
  petId: string;
  report: Report;
  unlocked: string[];
}

export interface L0Preview {
  preview: string;
  background: string;
  tolerance: number;
  warnings: string[];
}

/** 视频处理进度(抠像是秒级操作,前端必须有反馈)。 */
export interface VideoProgress {
  stage: string;
  done: number;
  total: number;
}

export interface WatcherFile {
  path: string;
  kind: "image" | "video";
}

// ---------------------------------------------------------------- 咒语包
// spellbook 模块未做 camelCase 重命名(它是数据化的文案表),保持 snake_case。

export interface SpellEntry {
  state: StateKey;
  state_name: string;
  frames: number;
  note: string;
  text: string;
  purified: string;
  rescue: string;
}

export interface GridSpell {
  id: string;
  title: string;
  desc: string;
  primary: boolean;
  free: boolean;
  states: StateKey[];
  state_names: string[];
  rows: number;
  cols: number;
  text: string;
  purified: string;
}

/** 脚本里的一个动作片段:既是分镜文案,也是导入时的切段契约。 */
export interface VideoSegmentSpell {
  state: StateKey;
  stateName: string;
  seconds: number;
  /** 片段起止,占全片时长的比例(0..1) */
  start: number;
  end: number;
  frames: number;
  desc: string;
}

export interface VideoPlan {
  id: string;
  title: string;
  desc: string;
  free: boolean;
  seconds: number;
  segments: VideoSegmentSpell[];
  text: string;
}

export interface VideoSpell {
  label: string;
  setup: string;
  idle: string;
  walk: string;
  /** 一次生成、多状态点亮(默认路线) */
  plans: VideoPlan[];
  fullSetup: string;
}

export interface PlatformSpells {
  id: string;
  name: string;
  role: "default" | "advanced" | "optional";
  url: string;
  tips: string[];
  grid: GridSpell[];
  singles: SpellEntry[];
  video: VideoSpell | null;
}

export interface FailureCheck {
  symptom: string;
  cause: string;
  action: string;
}

export interface SpellbookBundle {
  version: string;
  default_platform: string;
  hard_rules: string;
  video_note: string;
  platforms: PlatformSpells[];
  failure_checks: FailureCheck[];
  /** 知名形象拦截关键词:匹配平台报错文案 → 弹本地路线引导卡。 */
  intercept_keywords: string[];
}

// ---------------------------------------------------------------- 提醒 / 番茄钟

export interface Reminder {
  id: string;
  title: string;
  kind: "once" | "every";
  at?: string;
  minutes?: number;
  enabled: boolean;
  lastFired?: string;
  created: string;
}

export interface FiredEvent {
  kind: "once" | "every" | "pomodoro" | "snooze";
  title: string;
  reminderId?: string;
}

export interface PomodoroCfg {
  workMin: number;
  breakMin: number;
  rounds: number;
}

export interface PomodoroStatus {
  running: boolean;
  phase: "work" | "break" | "idle";
  round: number;
  rounds: number;
  remainingSecs: number;
  totalSecs: number;
}

// ---------------------------------------------------------------- 运行时

export interface ExporterStatus {
  found: boolean;
  path: string | null;
}

export interface WorkArea {
  x: number;
  y: number;
  w: number;
  h: number;
  scale: number;
}

export interface PetChangedEvent {
  petId: string | null;
  hatch: boolean;
  /** 进化播报:本次点亮/更新的状态中文名 */
  evolved?: string[] | null;
}
