/** 宠物窗主循环：渲染 × 行为脑 × 漫游 × 交互 × 事件（方案 §6 P0-1 / P0-4）。 */

import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

import { applyPetTheme, petEvents, petIpc } from "../ipc";
import type { PetSettings, StateKey, WorkArea } from "../types";
import "@sentenceflow/ui/src/tokens.css";
import "./pet.css";
import { BodySway, BEND_MAX } from "./bend";
import type { BlinkMethod } from "./blink";
import { hideBubble, isBubbleVisible, showBubble } from "./bubble";
import { isMenuOpen, setupInteractions } from "./interact";
import { Brain, type BrainOptions, type MicroAction } from "./machine";
import { IDENTITY, landingSquash, motionFor, type Transform } from "./motion";
import { Particles } from "./particles";
import { drawProps } from "./props";
import type { BlinkOverlay, DrawOpts } from "./renderer";
import { Renderer } from "./renderer";
import { frameAt, type ResolvedClip, SpriteSheet } from "./sprites";

const win = getCurrentWindow();
const renderer = new Renderer(document.getElementById("pet-canvas") as HTMLCanvasElement);
const particles = new Particles();
const brain = new Brain();

let settings: PetSettings;
let sheet: SpriteSheet | null = null;
let workArea: WorkArea | null = null;
let winPos = { x: 0, y: 0 };
let winSize = { w: 0, h: 0 };
let positionDirty = false;
let positionBusy = false;

/** 孵化仪式（3 秒：摇晃→裂纹→白闪→登场） */
let ceremonyStart = 0;
const CEREMONY_MS = 3000;
/** 拖拽放手后的下落/落地 */
let fallStart = 0;
let fallFromY = 0;
let landedAt = 0;
let pomodoroRunning = false;
let lastSpawn: Record<string, number> = {};

/** 安静模式（生命感 §14.1）：微动作频率减半、光标反应减弱。菜单切换，本地持久化。 */
const QUIET_KEY = "sf.pet.quiet";
let quietMode = localStorage.getItem(QUIET_KEY) === "1";
/** 步幅相位累积（BASE_W 空间的距离/步幅，速度-步频锁定 §12.2，治滑步）。 */
let walkPhase = 0;
/** 身体两段式弯曲弹簧（尾摆·L0 §13）。 */
const sway = new BodySway();
/** 状态过渡交叉淡入（§14.1）：切换瞬间冻结上一帧，120ms 内淡入新帧。 */
let lastRenderMode: StateKey | null = null;
let lastSnapshot: { clip: ResolvedClip; frame: number; tf: Transform } | null = null;
let prevDraw: { clip: ResolvedClip; frame: number; tf: Transform } | null = null;
let xfadeStart = 0;
const XFADE_MS = 120;
/** 光标速度追踪（光标感知 §14.1）。 */
let lastCursor: { x: number; y: number; t: number } | null = null;
/** 供 hitTest 内光标感知复用的行为选项快照。 */
let brainOpts: BrainOptions | null = null;
const BREATH_IDLE = 3.7; // idle 呼吸周期（s，L0）
const BREATH_SLEEP = 5.0; // sleep 呼吸周期（s，L0）
const SLEEP_ROT = -1.45; // 睡觉躺倒角度（≈-83°：头朝左、近乎平躺）
/** 本帧时间步（秒），tick 内更新，render 内供弹簧积分用。 */
let renderStep = 1 / 60;

// ---------------------------------------------------------------- 初始化

async function refreshWindowMetrics(): Promise<void> {
  try {
    workArea = await petIpc.workAreaGet();
    const pos = await win.outerPosition();
    const size = await win.outerSize();
    winPos = { x: pos.x, y: pos.y };
    winSize = { w: size.width, h: size.height };
  } catch {
    /* 窗口尚未就绪时忽略 */
  }
}

async function loadPet(): Promise<void> {
  const assets = await petIpc.activeAssets();
  sheet = assets ? await SpriteSheet.load(assets) : null;
}

async function init(): Promise<void> {
  // 一次 bootstrap 拿齐设置、主题与图集(原实现是三次往返 + 一套自带的
  // localStorage 主题;后者会跨窗覆写主窗的 data-theme,已整体废弃)。
  const boot = await petIpc.bootstrap();
  settings = boot.settings;
  applyPetTheme(boot.theme);
  sheet = boot.active ? await SpriteSheet.load(boot.active) : null;
  await refreshWindowMetrics();

  window.setInterval(() => void refreshWindowMetrics(), 45_000);

  await petEvents.onChanged(async (e) => {
    hideBubble();
    await loadPet();
    if (e.hatch && sheet) {
      ceremonyStart = performance.now();
    } else if (e.evolved?.length && sheet) {
      // 进化播报（改进方案 §4）：当场切换真动画 + 气泡庆祝
      brain.triggerHappy(1.6);
      particles.spawn("confetti", renderer.anchorX, renderer.anchorY - 120 * renderer.k, 14);
      showBubble(`我学会${e.evolved.join("、")}啦！`, [{ label: "棒！", onClick: () => {} }], 9000);
    }
  });
  await petEvents.onSettings((s) => {
    settings = s;
    setTimeout(() => renderer.resize(), 80); // Rust 侧可能调整了窗口尺寸
  });
  // 主窗切换外观时跟随(纯新增事件;两窗 data-theme 同源同值)
  await petEvents.onTheme(applyPetTheme);
  await petEvents.onReminder((e) => {
    brain.triggerRemind(45_000);
    particles.spawn("bell", renderer.anchorX, renderer.anchorY - 120 * renderer.k);
    const actions = [
      {
        label: "知道了",
        onClick: () => {
          brain.stopRemind();
        },
      },
    ];
    if (e.kind !== "pomodoro") {
      actions.push({
        label: "稍后 5 分钟",
        onClick: () => {
          void petIpc.reminderSnooze(e.title, e.reminderId ?? null, 5);
          brain.stopRemind();
        },
      });
    }
    showBubble(e.title, actions, 45_000);
  });
  await petEvents.onPomodoro((s) => {
    pomodoroRunning = s.running;
  });

  requestAnimationFrame(tick);
}

// ---------------------------------------------------------------- 交互

const interactions = setupInteractions(renderer.canvas, {
  onTap() {
    brain.poke();
    if (!sheet) {
      void petIpc.openStudio("wizard");
      return;
    }
    brain.triggerHappy(Math.max(1.0, sheet.oneShotDuration("happy")));
    particles.spawn("hearts", renderer.anchorX, renderer.anchorY - 140 * renderer.k);
  },
  onDragStart() {
    brain.poke();
    hideBubble();
  },
  onDragEnd() {
    void (async () => {
      await refreshWindowMetrics();
      if (!workArea) return;
      const floorY = workArea.y + workArea.h - winSize.h;
      if (winPos.y < floorY - 4) {
        fallStart = performance.now();
        fallFromY = winPos.y;
      } else {
        landedAt = performance.now();
      }
    })();
  },
  menuItems() {
    const items = [];
    if (sheet) {
      items.push(
        {
          label: "摸摸",
          icon: "🖐",
          onClick: () => {
            brain.triggerHappy(1.2);
            particles.spawn("hearts", renderer.anchorX, renderer.anchorY - 140 * renderer.k);
          },
        },
        {
          label: "投喂",
          icon: "🍪",
          onClick: () => {
            // ≥1.6s 让三口咀嚼（0.5/0.9/1.3s）与末尾小摆都完整播出（§13 吃）。
            brain.triggerEat(sheet ? Math.max(1.6, sheet.oneShotDuration("eat")) : 1.6);
            particles.spawn("crumbs", renderer.anchorX, renderer.anchorY - 60 * renderer.k);
          },
          separatorAfter: true,
        },
      );
    }
    items.push(
      {
        label: pomodoroRunning ? "停止番茄钟" : "开始番茄钟（25 分钟）",
        icon: "🍅",
        onClick: () => {
          if (pomodoroRunning) {
            void petIpc.pomodoroStop();
          } else {
            petIpc.pomodoroStart({ workMin: 25, breakMin: 5, rounds: 4 }).catch((e) => {
              showBubble(String(e), [{ label: "知道了", onClick: () => {} }], 8000);
            });
          }
        },
      },
      {
        label: "设置提醒…",
        icon: "⏰",
        onClick: () => void petIpc.openStudio("reminders"),
        separatorAfter: true,
      },
      {
        label: "打开孵化器",
        icon: "🥚",
        onClick: () => void petIpc.openStudio(),
      },
      {
        label: quietMode ? "取消安静模式" : "安静模式（少打扰）",
        icon: "🤫",
        onClick: () => {
          quietMode = !quietMode;
          localStorage.setItem(QUIET_KEY, quietMode ? "1" : "0");
        },
      },
      // 「退出」只隐藏宠物,不再退进程 —— 句流的进程归主窗所有,
      // 原实现的 app_quit 会把整个句流一起关掉。恢复入口在「AI 萌宠」页。
      {
        label: "收起宠物（在「AI 萌宠」页可再显示）",
        icon: "🫥",
        onClick: () => void petIpc.visibleSet(false),
      },
    );
    return items;
  },
});

// ---------------------------------------------------------------- 窗口移动

function queuePosition(x: number, y: number): void {
  winPos = { x: Math.round(x), y: Math.round(y) };
  positionDirty = true;
}

function flushPosition(): void {
  if (!positionDirty || positionBusy) return;
  positionDirty = false;
  positionBusy = true;
  void win
    .setPosition(new PhysicalPosition(winPos.x, winPos.y))
    .catch(() => {})
    .finally(() => {
      positionBusy = false;
    });
}

// ---------------------------------------------------------------- 主循环

let lastT = performance.now();
let perfAccum = 0;

function tick(now: number): void {
  requestAnimationFrame(tick);
  const dt = Math.min(0.1, (now - lastT) / 1000);
  lastT = now;

  // 省电档：约 12fps
  if (settings?.performance_mode) {
    perfAccum += dt;
    if (perfAccum < 1 / 12) return;
  }
  const step = settings?.performance_mode ? perfAccum : dt;
  perfAccum = 0;
  renderStep = step;

  const dragging = interactions.isDragging();
  brainOpts = {
    dragging,
    roamEnabled: !!settings?.roam && !!sheet && ceremonyStart === 0,
    hasWalk: !!sheet?.has("walk-right"),
    nightSleep: !!settings?.night_sleep,
    sleepAfterMs: (settings?.sleep_after_min ?? 8) * 60_000,
    quiet: quietMode,
    oneShotDuration: (m: StateKey) => sheet?.oneShotDuration(m) ?? 1.1,
  };
  const out = brain.update(brainOpts);

  updateRoam(out.mode, step, dragging);
  updateFall(now, dragging);
  render(now, out.mode, out.walkDir, out.turnPrep, out.micro);
  spawnAmbient(now, out.mode);

  particles.update(step);
  particles.draw(renderer.ctx);
  flushPosition();
  updateHitTest(step, dragging);
}

// ---------------------------------------------------------------- 轮廓级点击穿透
// 窗口是 300×390 的矩形，但只有宠物"轮廓"（不透明像素）应当拦截鼠标；
// 其余区域自动 set_ignore_cursor_events(true)，不干扰底下的正常办公。
// 穿透状态下 webview 收不到鼠标事件，因此以 ~30Hz 轮询全局光标做命中测试。

let ignoreApplied: boolean | null = null;
let hitAccum = 0;
let hitBusy = false;

function applyIgnore(ignore: boolean): void {
  if (ignoreApplied === ignore) return;
  ignoreApplied = ignore;
  void win.setIgnoreCursorEvents(ignore).catch(() => {
    ignoreApplied = null; // 失败下次重试
  });
}

function updateHitTest(dt: number, dragging: boolean): void {
  if (!settings) return;
  if (settings.click_through) {
    // 全局穿透开关打开：由 Rust 统一管理，这里不插手
    ignoreApplied = null;
    return;
  }
  hitAccum += dt;
  const interval = settings.performance_mode ? 0.1 : 0.033;
  if (hitAccum < interval || hitBusy) return;
  hitAccum = 0;

  // 拖拽中 / 菜单展开 / 气泡展示中：整窗保持可交互
  if (dragging || isMenuOpen() || isBubbleVisible()) {
    applyIgnore(false);
    return;
  }

  hitBusy = true;
  void (async () => {
    try {
      const [cx, cy] = await petIpc.cursorPositionGet();
      const dpr = window.devicePixelRatio || 1;
      const lx = (cx - winPos.x) / dpr;
      const ly = (cy - winPos.y) / dpr;
      const inside = lx >= 0 && ly >= 0 && lx <= renderer.w && ly <= renderer.h;
      applyIgnore(!(inside && renderer.hitTest(lx, ly)));
      noticeCursor(cx, cy, dpr);
    } catch {
      /* 光标查询瞬时失败：保持现状 */
    } finally {
      hitBusy = false;
    }
  })();
}

/** 光标感知（§14.1）：近距 + 快速移动 → 朝光标张望，20% 概率蹦一步过去（脑内处理）。 */
function noticeCursor(cx: number, cy: number, dpr: number): void {
  const now = performance.now();
  const centerX = winPos.x + winSize.w / 2;
  const side: -1 | 1 = cx >= centerX ? 1 : -1;
  const near = Math.abs(cx - centerX) < 180 * dpr && Math.abs(cy - (winPos.y + winSize.h / 2)) < 220 * dpr;
  if (lastCursor && brainOpts) {
    const dtc = (now - lastCursor.t) / 1000;
    if (dtc > 0.001) {
      const speed = Math.hypot(cx - lastCursor.x, cy - lastCursor.y) / dtc;
      brain.noticeCursor(now, side, near, speed > 700 * dpr, brainOpts);
    }
  }
  lastCursor = { x: cx, y: cy, t: now };
}

function updateRoam(mode: StateKey, dt: number, dragging: boolean): void {
  if (!workArea || dragging || fallStart > 0) return;
  if (mode !== "walk-right" && mode !== "walk-left") return;
  const dir = mode === "walk-right" ? 1 : -1;
  const speed = 46 * renderer.k * workArea.scale; // 物理 px/s
  let nx = winPos.x + dir * speed * dt;
  const minX = workArea.x + 4;
  const maxX = workArea.x + workArea.w - winSize.w - 4;
  if (nx <= minX || nx >= maxX) {
    nx = Math.max(minX, Math.min(maxX, nx));
    brain.bounce();
  }
  // 速度-步频锁定（§12.2）：相位按**实际位移/步幅**推进——撞墙不动则不迈步，治滑步。
  const stride = sheet?.resolve("walk-right")?.stride ?? sheet?.resolve(mode)?.stride ?? 30;
  const movedBaseW = Math.abs(nx - winPos.x) / renderer.k;
  // 相位与 sin/frameAt 皆以 1 为周期，取模 1 防长时会话浮点精度漂移。
  walkPhase = (walkPhase + movedBaseW / stride) % 1;
  const floorY = workArea.y + workArea.h - winSize.h;
  queuePosition(nx, floorY);
}

function updateFall(now: number, dragging: boolean): void {
  if (fallStart === 0 || dragging || !workArea) return;
  const t = (now - fallStart) / 450;
  const floorY = workArea.y + workArea.h - winSize.h;
  if (t >= 1) {
    queuePosition(winPos.x, floorY);
    fallStart = 0;
    landedAt = now;
    particles.spawn("sweat", renderer.anchorX, renderer.anchorY - 80 * renderer.k);
    return;
  }
  // easeInQuad 下落
  const y = fallFromY + (floorY - fallFromY) * t * t;
  queuePosition(winPos.x, y);
}

function compose(a: Transform, b: Transform): Transform {
  return {
    dx: a.dx + b.dx,
    dy: a.dy + b.dy,
    rot: a.rot + b.rot,
    sx: a.sx * b.sx,
    sy: a.sy * b.sy,
    mirror: a.mirror || b.mirror,
  };
}

function render(now: number, mode: StateKey, walkDir: -1 | 1, turnPrep: number, micro: MicroAction | null): void {
  renderer.clear();

  // —— 孵化仪式 ——
  if (ceremonyStart > 0) {
    const t = (now - ceremonyStart) / 1000;
    if (t < 2.3) {
      const wobble = Math.sin(t * 9) * 0.1 * Math.min(1, t / 1.2);
      renderer.drawEgg(t, Math.max(0, (t - 1.0) / 1.2), wobble);
    } else if (t < CEREMONY_MS / 1000) {
      if (sheet) {
        const c = sheet.resolve("idle");
        if (c) renderer.drawPet(sheet, c, 0, IDENTITY);
      }
      renderer.flash(1 - (t - 2.3) / 0.7);
      if (!lastSpawn["ceremony"]) {
        lastSpawn["ceremony"] = now;
        particles.spawn("confetti", renderer.anchorX, renderer.anchorY - 100 * renderer.k, 18);
      }
    } else {
      ceremonyStart = 0;
      delete lastSpawn["ceremony"];
      brain.triggerHappy(1.4);
    }
    lastRenderMode = null;
    prevDraw = null;
    return;
  }

  if (!sheet) {
    // 占位蛋：轻轻呼吸，点击去孵化
    renderer.drawEgg(now / 1000, 0, Math.sin(now / 700) * 0.02);
    lastRenderMode = null;
    return;
  }

  // —— 走路：有真步态素材就播真的，没有才由程序接管 ——
  //   程序化分层剪纸当初是为了治**烘焙** walk 帧的香蕉弯/滑步（那些帧是把 idle 扭出来的）。
  //   但视频/网格导入的 walk 是真实步态，必须原样播放 —— 否则用户特意生成的
  //   行走/奔跑动画会被程序化走路悄悄顶掉（正是「宠物不按完全体动作活动」的原因）。
  const isWalk = mode === "walk-left" || mode === "walk-right";
  const walkClip = isWalk ? sheet.resolve(mode) : null;
  const realWalk = !!walkClip?.real && !sheet.isL0;
  if (isWalk && !realWalk && sheet.rig) {
    const dir: -1 | 1 = mode === "walk-right" ? 1 : -1;
    renderer.drawCutout(sheet, sheet.rig, walkPhase, dir);
    lastRenderMode = mode;
    lastSnapshot = null;
    prevDraw = null;
    return;
  }

  // walk 无真帧且无 rig 时按 idle 帧走弹跳降级（hasReal 置 false → 变形层出挤压弹跳）。
  const hasReal = sheet.has(mode) && !sheet.isL0 && (!isWalk || realWalk);
  const drawClip = hasReal ? sheet.resolve(mode) : sheet.resolve("idle");
  if (!drawClip) return;
  const timeInMode = (now - brain.modeSince) / 1000;
  const { frame, phase } = frameAndPhase(drawClip, mode, timeInMode, hasReal);

  // —— 变形层（通道掩码混合）+ 微动作叠加 + 落地挤压 ——
  let tf = motionFor(mode, timeInMode, {
    hasRealClip: hasReal,
    phase,
    walkDir,
    turnPrep,
    budget: micro?.macro ? 0.3 : 1,
  });
  tf = compose(tf, microTransform(micro));
  if (landedAt > 0) {
    const since = now - landedAt;
    if (since < 350) tf = compose(tf, landingSquash(since));
    else landedAt = 0;
  }

  // —— 两段式弯曲（尾摆·L0）：仅 idle 且无真帧时给柔性摆动，其余让弹簧松弛归零 ——
  let bendAngle = 0;
  if (mode === "idle" && !hasReal && !settings?.performance_mode) {
    bendAngle = sway.update(renderStep, BEND_MAX * Math.sin((now / 1000) * 0.9));
  } else {
    sway.update(renderStep, 0);
  }

  // —— 眨眼（相位门控帧交换 或 双渲染叠加；未校准照片类不眨）——
  const blink = blinkFor(micro, drawClip, mode, phase);
  const drawFrame = blink.frame ?? frame;
  const opts: DrawOpts = {};
  if (blink.overlay) opts.blink = blink.overlay;
  if (Math.abs(bendAngle) > 1e-4) opts.bendAngle = bendAngle;

  // —— 状态过渡交叉淡入（§14.1）——
  if (lastRenderMode !== null && mode !== lastRenderMode && lastSnapshot) {
    prevDraw = lastSnapshot;
    xfadeStart = now;
  }
  lastRenderMode = mode;
  if (prevDraw && now - xfadeStart < XFADE_MS) {
    const p = (now - xfadeStart) / XFADE_MS;
    renderer.drawPet(sheet, prevDraw.clip, prevDraw.frame, prevDraw.tf, { alpha: 1 - p });
    opts.alpha = p;
  } else {
    prevDraw = null;
  }
  if (mode === "sleep") {
    // 睡觉=躺下：整体旋转躺倒 + 头下垫枕 + 闭眼 + 慢呼吸（不切分不形变，永不撕裂）。
    const breath = 1 + 0.02 * Math.sin((Math.PI * 2 * timeInMode) / BREATH_SLEEP);
    const propsAlpha = settings?.props_enabled === false ? 0 : Math.min(1, timeInMode / 1.2);
    renderer.drawSleeping(sheet, drawClip, drawFrame, {
      rot: SLEEP_ROT,
      breath,
      blink: sleepBlinkOverlay(),
      propsAlpha,
      alpha: opts.alpha,
    });
    lastSnapshot = null;
  } else if (isWalk && !realWalk && sheet.legSwing) {
    // 团子弹跳降级 + 腿部铰接：上身整体弹跳，下身一整块绕髋微摆（无缝无撕裂）补足走路感。
    // 与上面的 cutout 同理，**只在没有真步态素材时**才接管：legSwing 绑定是拿 idle 帧
    // 建的，若无条件生效，视频抠出来的真实步态会被"正面站姿+弹跳"顶掉，
    // 表现就是"正面站着平移"。
    renderer.drawBounceWalk(sheet, sheet.legSwing, walkPhase, tf, opts.alpha ?? 1);
    lastSnapshot = null;
  } else {
    renderer.drawPet(sheet, drawClip, drawFrame, tf, opts);
    lastSnapshot = { clip: drawClip, frame: drawFrame, tf };
  }

  // —— 道具叠加层（改进方案环节四）：eat/remind 的道具动画（sleep 道具由 drawSleeping 负责）——
  if (settings?.props_enabled !== false && (mode === "eat" || mode === "remind")) {
    const cell = sheet.spec.cell;
    drawProps(renderer.ctx, mode, timeInMode, sheet.anchors, renderer.k, (cx, cy) => ({
      x: renderer.anchorX + (cx - cell.w / 2) * renderer.k,
      y: renderer.anchorY + (cy - cell.h) * renderer.k,
    }));
  }
}

/** 帧索引与运动相位（相位锁定 §12.2）：走=距离相位（治滑步），idle/sleep=呼吸相位，
 *  一次性动画随时间推进到末帧停住。无真帧(L0)一律绘制 idle 首帧、由变形层供生命感。 */
function frameAndPhase(
  clip: ResolvedClip,
  mode: StateKey,
  timeInMode: number,
  hasReal: boolean,
): { frame: number; phase: number } {
  const cycleLen = clip.playMode === "pingpong" ? Math.max(1, (clip.frames - 1) * 2) : clip.frames;
  if (mode === "walk-right" || mode === "walk-left") {
    return { frame: hasReal ? frameAt(clip, walkPhase) : 0, phase: walkPhase };
  }
  if (mode === "happy" || mode === "eat") {
    const frame = hasReal ? Math.min(Math.floor(timeInMode * clip.fps), clip.frames - 1) : 0;
    return { frame, phase: timeInMode };
  }
  if (mode === "idle") {
    const phase = hasReal ? (timeInMode * clip.fps) / cycleLen : timeInMode / BREATH_IDLE;
    return { frame: hasReal ? frameAt(clip, phase) : 0, phase };
  }
  if (mode === "sleep") {
    const phase = hasReal ? (timeInMode * clip.fps) / (2 * cycleLen) : timeInMode / BREATH_SLEEP;
    return { frame: hasReal ? frameAt(clip, phase) : 0, phase };
  }
  // remind（loop）/ drag
  const phase = hasReal ? (timeInMode * clip.fps) / cycleLen : timeInMode;
  return { frame: hasReal ? frameAt(clip, phase) : 0, phase };
}

/** 微动作变形叠加（§14.1）：张望=侧倾偏移；伸懒腰=纵伸；坐下=下压。眨眼另行处理。 */
function microTransform(micro: MicroAction | null): Transform {
  if (!micro || micro.kind === "blink") return IDENTITY;
  const p = micro.progress;
  const env = Math.max(0, Math.min(1, p / 0.2, (1 - p) / 0.2)); // 前后各 20% 渐入渐出
  switch (micro.kind) {
    case "look":
      return { ...IDENTITY, rot: micro.dir * 0.05 * env, dx: micro.dir * 3 * env };
    case "stretch":
      return { ...IDENTITY, sy: 1 + 0.06 * env, sx: 1 - 0.04 * env, dy: -2 * env };
    case "sit":
      return { ...IDENTITY, sy: 1 - 0.06 * env, sx: 1 + 0.04 * env, dy: 3 * env };
    default:
      return IDENTITY;
  }
}

/** 眨眼决策（§13）：A 级有烘焙闭眼帧→相位门控帧交换；B 级双眼已校准→双渲染叠加；
 *  照片类或未校准→不眨（保守，避免误伤）。 */
function blinkFor(
  micro: MicroAction | null,
  clip: ResolvedClip,
  mode: StateKey,
  phase: number,
): { frame?: number; overlay?: BlinkOverlay } {
  if (!sheet || !micro || micro.kind !== "blink" || mode !== "idle") return {};
  if (sheet.spec.inputType === "pet_photo") return {}; // 照片类默认关闭（规则检查项）
  const closed = Math.sin(Math.PI * micro.progress); // 睁→闭→睁
  if (clip.blinkFrame != null) {
    const f = ((phase % 1) + 1) % 1;
    const neutral = f < 0.15 || f > 0.85; // 相位门控：呼吸回到中性帧时才插入
    return neutral || closed > 0.5 ? { frame: clip.blinkFrame } : {};
  }
  const a = sheet.anchors;
  if (a.eyesCalibrated && a.eyeL && a.eyeR) {
    // 遮盖法(mask)只在眼睛处画肤色盖片+下弯弧，仅闭合眼睛；压缩法(compress)会把整条
    // 眼带横切纵压、拉伸皮肤补位，在平涂/卡通脸上表现为"整头横向挤压"。故：拟真渲染
    // (cartoon_3d/portrait)才用压缩法，平涂 / 卡通 / 未标注(绝大多数孵化宠)一律遮盖法。
    // 照片类(pet_photo)已在上方直接关闭眨眼。
    const realistic =
      sheet.spec.inputType === "cartoon_3d" || sheet.spec.inputType === "portrait";
    const method: BlinkMethod = realistic ? "compress" : "mask";
    return { overlay: { closed, method, eyeL: a.eyeL, eyeR: a.eyeR } };
  }
  return {};
}

/** 睡觉闭眼叠加：眼已校准且非照片类时，睡眠期间常闭眼（closed=1）。 */
function sleepBlinkOverlay(): BlinkOverlay | undefined {
  if (!sheet || sheet.spec.inputType === "pet_photo") return undefined;
  const a = sheet.anchors;
  if (!(a.eyesCalibrated && a.eyeL && a.eyeR)) return undefined;
  const realistic =
    sheet.spec.inputType === "cartoon_3d" || sheet.spec.inputType === "portrait";
  const method: BlinkMethod = realistic ? "compress" : "mask";
  return { closed: 1, method, eyeL: a.eyeL, eyeR: a.eyeR };
}

/** 状态氛围粒子：睡觉 Zzz、提醒铃铛。 */
function spawnAmbient(now: number, mode: StateKey): void {
  if (settings?.performance_mode) return;
  const every = (key: string, ms: number, fn: () => void) => {
    if (!lastSpawn[key] || now - lastSpawn[key] > ms) {
      lastSpawn[key] = now;
      fn();
    }
  };
  if (mode === "sleep") {
    every("zzz", 1700, () =>
      particles.spawn("zzz", renderer.anchorX + 30 * renderer.k, renderer.anchorY - 150 * renderer.k),
    );
  } else if (mode === "remind") {
    every("bell", 1400, () =>
      particles.spawn("bell", renderer.anchorX, renderer.anchorY - 170 * renderer.k),
    );
  } else if (mode === "eat") {
    every("crumbs", 700, () =>
      particles.spawn("crumbs", renderer.anchorX, renderer.anchorY - 60 * renderer.k, 2),
    );
  }
}

void init();
