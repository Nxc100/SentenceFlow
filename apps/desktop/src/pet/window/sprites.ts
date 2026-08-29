/** 精灵图集：从 data URL 加载 sheet，解析状态契约、镜像派生与道具锚点。 */

import type { ActivePetAssets, PetSpec, StateClip, StateKey } from "../types";
import { buildRig, buildLegSwing, type CutoutRig, type LegSwingRig } from "./cutout";
import type { PetAnchors } from "./props";

export type PlayMode = "loop" | "pingpong" | "once";

export interface ResolvedClip {
  row: number;
  frames: number;
  fps: number;
  loop: boolean;
  mirror: boolean;
  /** 播放模式（生命感 §12.2）：spec 优先，否则启发式（idle/sleep→pingpong 消接缝）。 */
  playMode: PlayMode;
  /** 逐帧时长（秒），覆盖匀速。缺省 undefined = 按 fps 匀速。 */
  frameDurations?: number[];
  /** idle 烘焙闭眼帧列（A-1 第 6 格）；缺省 undefined = 无烘焙闭眼帧。 */
  blinkFrame?: number;
  /** 每步幅（BASE_W 逻辑 px，速度-步频锁定 §12.2）。 */
  stride: number;
  /** 是否为导入的真素材（视频/网格）。烘焙产物与旧档为 false。 */
  real: boolean;
}

/** 默认步幅（BASE_W=300 空间的 px/循环）。修复方案 F4：旧值 30 使 6 帧循环在 46px/s 漫游下
 *  达 ~1.5 周期/秒（步频快 3×，脚打滑=踏步机感）。真实步幅 ≈0.5×骨架高 ≈90px → 约 0.5 周期/秒。
 *  未写 stride 的宠物（如认主未重烘者）即用此值；Rust 侧写入 stride 后自动覆盖（sprites 无需改）。 */
export const DEFAULT_STRIDE = 90;

/** 纯函数：给定循环相位 [0,1) 与播放模式，取当前帧索引（相位锁定 §12.2）。 */
export function frameAt(clip: ResolvedClip, phase01: number): number {
  const n = clip.frames;
  if (n <= 1) return 0;
  const p = phase01 - Math.floor(phase01); // 归一化到 [0,1)
  if (clip.playMode === "pingpong") {
    const span = (n - 1) * 2; // 0..n-1..1 无缝往返
    const i = Math.floor(p * span) % span;
    return i < n ? i : span - i;
  }
  return Math.floor(p * n) % n;
}

export class SpriteSheet {
  readonly spec: PetSpec;
  readonly img: HTMLImageElement;
  readonly petId: string;
  /** 道具锚点（spec.anchors 优先，否则由 idle 首帧包围盒推导，改进方案 §7.1） */
  readonly anchors: PetAnchors;
  /** 分层剪纸行走绑定（有可辨双腿时非 null；团子为 null，走弹跳降级）。 */
  readonly rig: CutoutRig | null;
  /** 弹跳降级时的"腿部铰接摆动"绑定（下身足够时非 null）。 */
  readonly legSwing: LegSwingRig | null;

  private constructor(
    petId: string,
    spec: PetSpec,
    img: HTMLImageElement,
    anchors: PetAnchors,
    rig: CutoutRig | null,
    legSwing: LegSwingRig | null,
  ) {
    this.petId = petId;
    this.spec = spec;
    this.img = img;
    this.anchors = anchors;
    this.rig = rig;
    this.legSwing = legSwing;
  }

  static async load(assets: ActivePetAssets): Promise<SpriteSheet> {
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("图集加载失败"));
      img.src = assets.sheet;
    });
    const spec = assets.spec;
    // 分层行走步幅须与运行时 walkPhase 推进同源（spec 优先，缺省 DEFAULT_STRIDE）。
    const stride = spec.states["walk-right"]?.stride ?? DEFAULT_STRIDE;
    const rig = buildRig(img, spec, stride);
    // 团子(无 rig)才需要腿部铰接摆动；有 rig 走分层，不需要。
    const legSwing = rig ? null : buildLegSwing(img, spec);
    return new SpriteSheet(assets.petId, spec, img, deriveAnchors(spec, img), rig, legSwing);
  }

  /** 真实存在的动画条（镜像态解析到源行）。缺失返回 null（由 L0 引擎降级）。 */
  resolve(state: StateKey): ResolvedClip | null {
    const clip = this.spec.states[state];
    if (!clip) return null;
    if (clip.mirrorOf) {
      const src = this.spec.states[clip.mirrorOf];
      if (!src) return null;
      return this.build(clip.mirrorOf, src, true);
    }
    return this.build(state, clip, false);
  }

  private build(key: string, c: StateClip, mirror: boolean): ResolvedClip {
    // 播放模式：spec 优先 → 非循环即 once → idle/sleep 用 pingpong 消接缝 → 其余 loop。
    const playMode: PlayMode =
      c.playMode ?? (!c.loop ? "once" : key === "idle" || key === "sleep" ? "pingpong" : "loop");
    return {
      row: c.row,
      frames: c.frames,
      fps: c.fps,
      loop: c.loop,
      mirror,
      playMode,
      frameDurations: c.frameDurations,
      blinkFrame: c.blinkFrame,
      stride: c.stride ?? DEFAULT_STRIDE,
      real: c.source === "real",
    };
  }

  has(state: StateKey): boolean {
    return this.resolve(state) !== null;
  }

  /** L0：仅 idle 单帧 —— 一切动效交给程序化引擎。 */
  get isL0(): boolean {
    const idle = this.spec.states["idle"];
    return (this.spec.tier === "L0") || (!!idle && idle.frames <= 1 && Object.keys(this.spec.states).length <= 2);
  }

  /** 一次性动画（happy/eat）的时长（秒）。 */
  oneShotDuration(state: StateKey): number {
    const clip = this.resolve(state);
    if (!clip) return 1.1;
    const dur = clip.frameDurations
      ? clip.frameDurations.reduce((a, b) => a + b, 0)
      : clip.frames / clip.fps;
    return Math.max(0.6, dur);
  }
}

/** 锚点推导：先算启发式基线（扫描 idle 首帧 alpha 包围盒），再用 spec.anchors 逐项覆盖。
 *  关键：认主校准写入的是 eyeL/eyeR/mouth/tail_tip（无 head），故不能要求 head 才采纳校准——
 *  只要有校准双眼即 eyesCalibrated=true 开启眨眼，缺失项（如 head）回退启发式。 */
function deriveAnchors(spec: PetSpec, img: HTMLImageElement): PetAnchors {
  const cellW = spec.cell.w;
  const cellH = spec.cell.h;

  // —— 启发式基线 ——
  let base: PetAnchors = {
    mouth: { x: cellW / 2, y: cellH * 0.55 },
    head: { x: cellW / 2, y: cellH * 0.25 },
    feet: { x: cellW / 2, y: cellH - 12 },
    eyeL: { x: cellW / 2 - cellW * 0.18, y: cellH * 0.3 },
    eyeR: { x: cellW / 2 + cellW * 0.18, y: cellH * 0.3 },
  };
  const idle = spec.states["idle"];
  if (idle) {
    try {
      const canvas = document.createElement("canvas");
      canvas.width = cellW;
      canvas.height = cellH;
      const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
      ctx.drawImage(img, 0, idle.row * cellH, cellW, cellH, 0, 0, cellW, cellH);
      const data = ctx.getImageData(0, 0, cellW, cellH).data;
      let x0 = cellW, y0 = cellH, x1 = 0, y1 = 0, found = false;
      for (let y = 0; y < cellH; y++) {
        for (let x = 0; x < cellW; x++) {
          if ((data[(y * cellW + x) * 4 + 3] ?? 0) > 24) {
            found = true;
            if (x < x0) x0 = x;
            if (x > x1) x1 = x;
            if (y < y0) y0 = y;
            if (y > y1) y1 = y;
          }
        }
      }
      if (found) {
        const cx = (x0 + x1) / 2;
        const bw = x1 - x0;
        const eyeY = y0 + (y1 - y0) * 0.3; // 上 1/3（§3.2 启发式）
        base = {
          mouth: { x: cx, y: y0 + (y1 - y0) * 0.38 },
          head: { x: cx, y: y0 },
          feet: { x: cx, y: y1 },
          eyeL: { x: cx - bw * 0.18, y: eyeY },
          eyeR: { x: cx + bw * 0.18, y: eyeY },
        };
      }
    } catch {
      /* 采样失败：用比例兜底 base */
    }
  }

  // —— 校准锚点逐项覆盖基线；有校准双眼即开启眨眼（§13/§14.2）——
  const d = spec.anchors;
  return {
    mouth: d?.mouth ?? base.mouth,
    head: d?.head ?? base.head,
    feet: d?.feet ?? base.feet,
    eyeL: d?.eyeL ?? base.eyeL,
    eyeR: d?.eyeR ?? base.eyeR,
    eyesCalibrated: !!(d?.eyeL && d?.eyeR),
  };
}
