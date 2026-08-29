/**
 * AI 萌宠的类型化 IPC —— 宠物窗与主窗「AI 萌宠」页共用的唯一出入口。
 *
 * 命名空间(整合方案 §3.1 原则 3):命令一律 `pet_*`,事件一律 `pet://*`。
 * 两处都不散落裸 `invoke`/`listen`,新增命令只在这里露一次面。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ActivePetAssets,
  ComposeView,
  EvolveResult,
  ExporterStatus,
  FiredEvent,
  HatchResult,
  L0Preview,
  PetBootstrap,
  PetChangedEvent,
  PetMeta,
  PetSettings,
  PetTheme,
  PomodoroCfg,
  PomodoroStatus,
  Reminder,
  Report,
  SessionView,
  SpellbookBundle,
  StateKey,
  VideoProgress,
  WatcherFile,
  WorkArea,
} from "./types";

/** `pet_wizard_add_video_multi` 的分段入参(与 Rust `SegmentSpec` 对应)。 */
export interface VideoSegmentArg {
  stateKey: StateKey;
  start: number;
  end: number;
  frames?: number;
}

export const petIpc = {
  // ---- 启动包 / 设置 ----
  bootstrap: () => invoke<PetBootstrap>("pet_bootstrap"),
  settingsGet: () => invoke<PetSettings>("pet_settings_get"),
  settingsSet: (settings: PetSettings) =>
    invoke<PetSettings>("pet_settings_set", { settings }),

  // ---- 宠物库 ----
  list: () => invoke<PetMeta[]>("pet_list"),
  thumb: (petId: string) => invoke<string>("pet_thumb", { petId }),
  activeAssets: () => invoke<ActivePetAssets | null>("pet_active_assets"),
  setActive: (petId: string) => invoke<void>("pet_set_active", { petId }),
  setAnchors: (petId: string, anchors: Record<string, { x: number; y: number }>) =>
    invoke<void>("pet_set_anchors", { petId, anchors }),
  remove: (petId: string) => invoke<void>("pet_delete", { petId }),
  /** 本地骨骼烘焙:单图进化为会走会跳会呼吸的动画(异步,不卡 IPC)。 */
  rigBake: (petId: string) => invoke<void>("pet_rig_bake", { petId }),

  // ---- .petkit 宠物包 ----
  kitExport: (petId: string, dest: string) =>
    invoke<void>("pet_kit_export", { petId, dest }),
  kitImport: (src: string) => invoke<string>("pet_kit_import", { src }),
  kitValidate: (src: string) => invoke<Report>("pet_kit_validate", { src }),

  // ---- 合成向导(先孵化,后进化)----
  wizardReset: () => invoke<void>("pet_wizard_reset"),
  wizardSession: () => invoke<SessionView>("pet_wizard_session"),
  wizardAddStrip: (
    stateKey: StateKey,
    path: string,
    tolerance?: number,
    forcedFrames?: number,
  ) =>
    invoke<SessionView>("pet_wizard_add_strip", {
      stateKey,
      path,
      tolerance: tolerance ?? null,
      forcedFrames: forcedFrames ?? null,
    }),
  wizardAddSheet: (path: string, states: StateKey[], cols?: number) =>
    invoke<SessionView>("pet_wizard_add_sheet", { path, states, cols: cols ?? null }),
  wizardAddVideo: (path: string, stateKey: StateKey, frames?: number) =>
    invoke<SessionView>("pet_wizard_add_video", {
      path,
      stateKey,
      frames: frames ?? null,
    }),
  /** 一次生成、多状态点亮:一段脚本视频按时间分段,每段各成一条动画。 */
  wizardAddVideoMulti: (path: string, segments: VideoSegmentArg[]) =>
    invoke<SessionView>("pet_wizard_add_video_multi", { path, segments }),
  wizardSwapStates: (a: StateKey, b: StateKey) =>
    invoke<SessionView>("pet_wizard_swap_states", { a, b }),
  wizardRemoveStrip: (stateKey: StateKey) =>
    invoke<SessionView>("pet_wizard_remove_strip", { stateKey }),
  wizardPreview: () => invoke<ComposeView>("pet_wizard_preview"),
  wizardHatch: (name: string) => invoke<HatchResult>("pet_wizard_hatch", { name }),
  wizardEvolve: () => invoke<EvolveResult>("pet_wizard_evolve"),
  wizardReslice: (stateKey: StateKey, forcedFrames: number) =>
    invoke<SessionView>("pet_wizard_reslice", { stateKey, forcedFrames }),
  wizardReplaceFrame: (
    stateKey: StateKey,
    frameIdx: number,
    path: string,
    tolerance?: number,
  ) =>
    invoke<SessionView>("pet_wizard_replace_frame", {
      stateKey,
      frameIdx,
      path,
      tolerance: tolerance ?? null,
    }),
  wizardExportFrame: (stateKey: StateKey, frameIdx: number, dest: string) =>
    invoke<void>("pet_wizard_export_frame", { stateKey, frameIdx, dest }),
  l0Preview: (path: string, tolerance?: number) =>
    invoke<L0Preview>("pet_l0_preview", { path, tolerance: tolerance ?? null }),
  l0Hatch: (path: string, tolerance: number | null, name: string) =>
    invoke<HatchResult>("pet_l0_hatch", { path, tolerance, name }),
  /** 参考图导出。`dest` 省略时由 Rust 侧弹「另存为」,返回实际写入路径。 */
  referenceExport: (petId?: string, dest?: string) =>
    invoke<string>("pet_reference_export", {
      petId: petId ?? null,
      dest: dest ?? null,
    }),

  // ---- 下载目录监听(默认关,设置里可开)----
  watcherStart: () => invoke<void>("pet_watcher_start"),
  watcherStop: () => invoke<void>("pet_watcher_stop"),

  // ---- 咒语包 ----
  spellbookGet: () => invoke<SpellbookBundle>("pet_spellbook_get"),
  guideSave: (frames: number, rows: number | null, dest: string) =>
    invoke<void>("pet_guide_save", { frames, rows, dest }),
  rescueGet: (stateKey: string, frames?: number) =>
    invoke<string>("pet_rescue_get", { stateKey, frames: frames ?? null }),

  // ---- 提醒 / 番茄钟 ----
  remindersList: () => invoke<Reminder[]>("pet_reminders_list"),
  reminderCreate: (
    title: string,
    kind: "once" | "every",
    at?: string,
    minutes?: number,
  ) =>
    invoke<Reminder>("pet_reminder_create", {
      title,
      kind,
      at: at ?? null,
      minutes: minutes ?? null,
    }),
  reminderToggle: (id: string, enabled: boolean) =>
    invoke<void>("pet_reminder_toggle", { id, enabled }),
  reminderDelete: (id: string) => invoke<void>("pet_reminder_delete", { id }),
  reminderSnooze: (title: string, reminderId: string | null, minutes: number) =>
    invoke<void>("pet_reminder_snooze", { title, reminderId, minutes }),
  pomodoroStart: (cfg: PomodoroCfg) =>
    invoke<PomodoroStatus>("pet_pomodoro_start", { cfg }),
  pomodoroStop: () => invoke<PomodoroStatus>("pet_pomodoro_stop"),
  pomodoroStatus: () => invoke<PomodoroStatus>("pet_pomodoro_status"),

  // ---- 出生视频 ----
  exporterStatus: () => invoke<ExporterStatus>("pet_exporter_status"),
  exportBirthVideo: (
    petId: string,
    dest: string,
    nameCardPng: string | null,
    watermarkPng: string | null,
    aiNoticePng: string | null,
  ) =>
    invoke<void>("pet_export_birth_video", {
      petId,
      dest,
      nameCardPng,
      watermarkPng,
      aiNoticePng,
    }),

  // ---- 运行时 ----
  workAreaGet: () => invoke<WorkArea>("pet_work_area_get"),
  cursorPositionGet: () => invoke<[number, number]>("pet_cursor_position_get"),
  visibleSet: (visible: boolean) => invoke<void>("pet_visible_set", { visible }),
  clickThroughSet: (enabled: boolean) =>
    invoke<void>("pet_click_through_set", { enabled }),
  /** 「打开孵化器」= 主窗聚焦 + 「AI 萌宠」页内切签。 */
  openStudio: (tab?: string) => invoke<void>("pet_open_studio", { tab: tab ?? null }),

  // ---- 系统外壳(走 Rust 侧 opener,前端不持 opener 权限)----
  /** 打开生成平台官网。白名单在 Rust 侧,与咒语包数据同源。 */
  openUrl: (url: string) => invoke<void>("pet_open_url", { url }),
  /** 在文件管理器里定位刚导出的文件。 */
  revealInDir: (path: string) => invoke<void>("pet_reveal_in_dir", { path }),

  // ---- 文件对话框(走 rfd,不引入 dialog 插件)----
  pickFile: (title: string, filterName: string, extensions: string[]) =>
    invoke<string | null>("pet_pick_file", { title, filterName, extensions }),
  savePath: (
    title: string,
    defaultName: string,
    filterName: string,
    extensions: string[],
  ) =>
    invoke<string | null>("pet_save_path", {
      title,
      defaultName,
      filterName,
      extensions,
    }),
};

/** 事件订阅(返回取消函数;React 侧在 `useEffect` 清理里调用)。 */
export const petEvents = {
  onChanged: (cb: (e: PetChangedEvent) => void): Promise<UnlistenFn> =>
    listen<PetChangedEvent>("pet://changed", (ev) => cb(ev.payload)),
  onSettings: (cb: (s: PetSettings) => void): Promise<UnlistenFn> =>
    listen<PetSettings>("pet://settings", (ev) => cb(ev.payload)),
  onTheme: (cb: (t: PetTheme) => void): Promise<UnlistenFn> =>
    listen<PetTheme>("pet://theme", (ev) => cb(ev.payload)),
  onReminder: (cb: (e: FiredEvent) => void): Promise<UnlistenFn> =>
    listen<FiredEvent>("pet://reminder", (ev) => cb(ev.payload)),
  onPomodoro: (cb: (s: PomodoroStatus) => void): Promise<UnlistenFn> =>
    listen<PomodoroStatus>("pet://pomodoro", (ev) => cb(ev.payload)),
  onExportProgress: (
    cb: (p: { done: number; total: number }) => void,
  ): Promise<UnlistenFn> =>
    listen<{ done: number; total: number }>("pet://export-progress", (ev) =>
      cb(ev.payload),
    ),
  onVideoProgress: (cb: (p: VideoProgress) => void): Promise<UnlistenFn> =>
    listen<VideoProgress>("pet://video-progress", (ev) => cb(ev.payload)),
  onWatcherFile: (cb: (f: WatcherFile) => void): Promise<UnlistenFn> =>
    listen<WatcherFile>("pet://watcher-file", (ev) => cb(ev.payload)),
  onNav: (cb: (tab: string) => void): Promise<UnlistenFn> =>
    listen<string>("pet://nav", (ev) => cb(ev.payload)),
};

/**
 * 主题落地:把句流的主题 id 写进 `data-theme`。
 *
 * `system` 要先解析成 light/dark —— tokens.css 只认具体主题值。
 * 宠物窗与主窗同源同值,不再有 HatchDesk 那套 localStorage 主题
 * (它会跨窗互踩,直接覆写主窗的 `<html data-theme>`)。
 */
export function applyPetTheme(theme: PetTheme): void {
  const resolved =
    theme === "system"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light"
      : theme;
  document.documentElement.dataset.theme = resolved;
}
