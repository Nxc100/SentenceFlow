/** 行为脑：8 状态机 + 漫游决策 + 睡眠/提醒仲裁（§5.2 触发契约）
 *  ＋生命感 §14：微行为调度器（眨眼/张望/伸懒腰/坐下，泊松间隔）、光标感知、
 *  调头预备（禁止行进中瞬翻）、安静模式。微动作以叠加字段实现，不扩状态机。 */

import type { StateKey } from "../types";

export interface BrainOptions {
  dragging: boolean;
  roamEnabled: boolean;
  hasWalk: boolean;
  nightSleep: boolean;
  sleepAfterMs: number;
  /** 安静模式：微动作频率减半、光标反应减弱（§14.1）。 */
  quiet: boolean;
  /** 一次性动画时长（秒） */
  oneShotDuration: (mode: StateKey) => number;
}

export type MicroKind = "blink" | "look" | "stretch" | "sit";

export interface MicroAction {
  kind: MicroKind;
  /** 进入该微动作后的秒数。 */
  t: number;
  /** 0..1 进度（over 其时长）。 */
  progress: number;
  /** 张望方向。 */
  dir: -1 | 1;
  /** 是否为「宏观」微动作（look/stretch/sit）——期间呼吸降幅、互斥。 */
  macro: boolean;
}

export interface BrainOutput {
  mode: StateKey;
  walkDir: -1 | 1;
  /** 0..1 调头预备进度（§14.1）。 */
  turnPrep: number;
  /** 叠加微动作（§14.1），null=无。 */
  micro: MicroAction | null;
}

function rand(a: number, b: number): number {
  return a + Math.random() * (b - a);
}

function isNight(): boolean {
  const h = new Date().getHours();
  return h >= 22 || h < 7;
}

interface MicroState {
  kind: MicroKind;
  start: number;
  dur: number;
  dir: -1 | 1;
  macro: boolean;
}

const MICRO_DUR: Record<MicroKind, () => number> = {
  blink: () => 0.125,
  look: () => rand(1.2, 3.0),
  stretch: () => 0.9,
  sit: () => rand(3, 8),
};

export class Brain {
  private lastInteract = performance.now();
  private oneShot: { mode: StateKey; until: number } | null = null;
  private remindUntil = 0;
  private walkUntil = 0;
  private walking = false;
  private nextDecision = performance.now() + 2500;
  walkDir: -1 | 1 = 1;
  /** 当前模式起始时间（供渲染取动画相位） */
  modeSince = performance.now();
  private lastMode: StateKey = "idle";

  // —— 调头预备 ——
  private pendingFlip = false;
  private turnStart = 0;
  private static TURN_MS = 110;

  // —— 微行为调度 ——
  private micro: MicroState | null = null;
  private nextBlink = performance.now() + rand(2500, 6000);
  private nextMacro = performance.now() + rand(15_000, 40_000);
  private cursorCooldownUntil = 0;

  poke(): void {
    this.lastInteract = performance.now();
  }

  triggerHappy(duration: number): void {
    this.poke();
    this.oneShot = { mode: "happy", until: performance.now() + duration * 1000 };
  }

  triggerEat(duration: number): void {
    this.poke();
    this.oneShot = { mode: "eat", until: performance.now() + duration * 1000 };
  }

  triggerRemind(durationMs = 30_000): void {
    this.remindUntil = performance.now() + durationMs;
  }

  stopRemind(): void {
    this.remindUntil = 0;
  }

  get reminding(): boolean {
    return performance.now() < this.remindUntil;
  }

  update(opts: BrainOptions): BrainOutput {
    const now = performance.now();
    const mode = this.decide(now, opts);
    if (mode !== this.lastMode) {
      this.lastMode = mode;
      this.modeSince = now;
    }
    const micro = this.updateMicro(now, mode, opts);
    const turnPrep = this.pendingFlip
      ? Math.min(1, (now - this.turnStart) / Brain.TURN_MS)
      : 0;
    return { mode, walkDir: this.walkDir, turnPrep, micro };
  }

  private decide(now: number, opts: BrainOptions): StateKey {
    if (opts.dragging) return "drag";

    if (this.oneShot) {
      if (now < this.oneShot.until) return this.oneShot.mode;
      this.oneShot = null;
    }

    if (now < this.remindUntil) return "remind";

    const asleep =
      now - this.lastInteract > opts.sleepAfterMs || (opts.nightSleep && isNight());
    if (asleep) {
      this.walking = false;
      return "sleep";
    }

    // —— 漫游决策 ——
    if (!opts.roamEnabled) {
      this.walking = false;
      return "idle";
    }
    if (this.walking && now < this.walkUntil) {
      // 调头预备完成后才真正翻向（§14.1 禁止行进中瞬翻）
      if (this.pendingFlip && now - this.turnStart >= Brain.TURN_MS) {
        this.walkDir = this.walkDir > 0 ? -1 : 1;
        this.pendingFlip = false;
      }
      return this.walkDir > 0 ? "walk-right" : "walk-left";
    }
    this.walking = false;
    this.pendingFlip = false;
    if (now >= this.nextDecision) {
      if (Math.random() < 0.45) {
        this.walking = true;
        this.walkDir = Math.random() < 0.5 ? -1 : 1;
        this.walkUntil = now + rand(2000, 5200);
        this.nextDecision = this.walkUntil + rand(4000, 10_000);
        return this.walkDir > 0 ? "walk-right" : "walk-left";
      }
      this.nextDecision = now + rand(5000, 13_000);
    }
    return "idle";
  }

  /** 微行为：仅在 idle 期间调度；宏观微动作互斥，眨眼可独立。 */
  private updateMicro(now: number, mode: StateKey, opts: BrainOptions): MicroAction | null {
    const q = opts.quiet ? 2 : 1;

    // 结束到期微动作
    if (this.micro && now - this.micro.start >= this.micro.dur * 1000) this.micro = null;

    // 非 idle：清掉宏观微动作（保留正在进行的眨眼收尾）
    if (mode !== "idle") {
      if (this.micro && this.micro.macro) this.micro = null;
    } else {
      // 眨眼（泊松，独立通道；12% 双眨由渲染侧加时）
      if (now >= this.nextBlink && (!this.micro || !this.micro.macro)) {
        if (!this.micro) this.micro = { kind: "blink", start: now, dur: MICRO_DUR.blink(), dir: 1, macro: false };
        this.nextBlink = now + rand(2500, 6000) * q;
      }
      // 宏观微动作（张望/伸懒腰/坐下），仅在无宏观进行时
      if (now >= this.nextMacro && !(this.micro && this.micro.macro)) {
        const roll = Math.random();
        const kind: MicroKind = roll < 0.5 ? "look" : roll < 0.8 ? "stretch" : "sit";
        this.micro = {
          kind,
          start: now,
          dur: MICRO_DUR[kind](),
          dir: Math.random() < 0.5 ? -1 : 1,
          macro: true,
        };
        const base = kind === "look" ? rand(15_000, 40_000) : kind === "stretch" ? rand(60_000, 180_000) : rand(90_000, 240_000);
        this.nextMacro = now + base * q;
      }
    }

    if (!this.micro) return null;
    const t = (now - this.micro.start) / 1000;
    return {
      kind: this.micro.kind,
      t,
      progress: Math.min(1, t / this.micro.dur),
      dir: this.micro.dir,
      macro: this.micro.macro,
    };
  }

  /** 光标感知（§14.1）：近距 + 快速移动 → 朝向光标张望，20% 概率蹦一步过去；冷却 30s。
   *  cursorSide：光标在宠物左(-1)/右(+1)侧。返回是否触发了「蹦一步」。 */
  noticeCursor(now: number, cursorSide: -1 | 1, near: boolean, fast: boolean, opts: BrainOptions): boolean {
    if (!near || !fast || now < this.cursorCooldownUntil) return false;
    if (this.reminding || this.oneShot || opts.dragging) return false;
    this.cursorCooldownUntil = now + 30_000;
    // 朝光标张望
    this.micro = { kind: "look", start: now, dur: rand(1.2, 2.4), dir: cursorSide, macro: true };
    // 20% 蹦一步过去（安静模式减半到 10%）
    if (opts.roamEnabled && Math.random() < (opts.quiet ? 0.1 : 0.2)) {
      this.walking = true;
      this.walkDir = cursorSide;
      this.walkUntil = now + rand(500, 1000);
      this.nextDecision = this.walkUntil + rand(4000, 10_000);
      return true;
    }
    return false;
  }

  /** 漫游撞到工作区边缘时调头——进入 100ms 调头预备，预备完成再翻向。 */
  bounce(): void {
    if (this.pendingFlip) return;
    this.pendingFlip = true;
    this.turnStart = performance.now();
  }
}
