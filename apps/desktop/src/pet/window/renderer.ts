/** Canvas 渲染器·帧层（生命感 §12.2）：按 caller 给定的帧索引绘制（播放模式/相位在上游算），
 *  支持交叉淡入 alpha、眨眼双渲染叠加、两段式弯曲切片绘制；蛋占位与孵化仪式不变。 */

import { drawBlink, type BlinkMethod } from "./blink";
import { BEND_OVERLAP, BEND_SPLIT } from "./bend";
import { drawBounceWalk, drawCutoutWalk, type CutoutRig, type LegSwingRig } from "./cutout";
import { drawMoon, drawPillow } from "./props";
import type { AnchorPoint } from "./props";
import type { ResolvedClip, SpriteSheet } from "./sprites";
import type { Transform } from "./motion";

export interface BlinkOverlay {
  closed: number; // [0,1]
  method: BlinkMethod;
  eyeL: AnchorPoint;
  eyeR: AnchorPoint;
}

export interface DrawOpts {
  /** 交叉淡入透明度（状态过渡 §14.1）。 */
  alpha?: number;
  /** 眨眼双渲染叠加（§13）。 */
  blink?: BlinkOverlay;
  /** 两段式弯曲上段角（尾摆·L0 §13）。 */
  bendAngle?: number;
}

/** 与 Rust runtime::PET_BASE_W 对应的逻辑基准宽度。 */
export const BASE_W = 300;

export class Renderer {
  readonly canvas: HTMLCanvasElement;
  readonly ctx: CanvasRenderingContext2D;
  w = 0;
  h = 0;
  /** 显示比例（窗口宽 / 基准宽） */
  k = 1;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    // willReadFrequently：hitTest 以 ~30Hz 读取像素，保持 CPU 后备位图避免 GPU 回读
    this.ctx = canvas.getContext("2d", { willReadFrequently: true })!;
    this.resize();
    window.addEventListener("resize", () => this.resize());
  }

  resize(): void {
    const dpr = window.devicePixelRatio || 1;
    this.w = window.innerWidth;
    this.h = window.innerHeight;
    this.canvas.width = Math.round(this.w * dpr);
    this.canvas.height = Math.round(this.h * dpr);
    this.canvas.style.width = `${this.w}px`;
    this.canvas.style.height = `${this.h}px`;
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.k = this.w / BASE_W;
  }

  clear(): void {
    this.ctx.clearRect(0, 0, this.w, this.h);
  }

  /** 宠物锚点：底边中点（挤压/旋转的支点）。 */
  get anchorX(): number {
    return this.w / 2;
  }

  get anchorY(): number {
    return this.h - 8 * this.k;
  }

  /** 绘制给定 clip 的第 frame 帧，施加变换 tf 与叠加项（眨眼/弯曲/淡入）。
   *  播放模式与相位在上游（main）计算后传入 frame，帧层只负责绘制。 */
  drawPet(sheet: SpriteSheet, clip: ResolvedClip, frame: number, tf: Transform, opts?: DrawOpts): void {
    const { cell } = sheet.spec;
    const ctx = this.ctx;
    const drawW = cell.w * this.k;
    const drawH = cell.h * this.k;
    const frameX = frame * cell.w;
    const rowY = clip.row * cell.h;
    const alpha = opts?.alpha ?? 1;

    ctx.save();
    if (alpha < 1) ctx.globalAlpha = alpha;
    ctx.imageSmoothingEnabled = false;
    ctx.translate(this.anchorX + tf.dx * this.k, this.anchorY + tf.dy * this.k);
    ctx.rotate(tf.rot);
    const flip = tf.mirror || clip.mirror;
    ctx.scale(tf.sx * (flip ? -1 : 1), tf.sy);

    const bend = opts?.bendAngle ?? 0;
    if (Math.abs(bend) > 1e-4) {
      this.drawBent(sheet.img, frameX, rowY, cell.w, cell.h, drawW, drawH, bend);
    } else {
      ctx.drawImage(sheet.img, frameX, rowY, cell.w, cell.h, -drawW / 2, -drawH, drawW, drawH);
    }

    if (opts?.blink && opts.blink.closed > 0.01) {
      const b = opts.blink;
      drawBlink(
        ctx,
        sheet.img,
        frameX,
        rowY,
        { cellW: cell.w, cellH: cell.h, drawW, drawH, eyeL: b.eyeL, eyeR: b.eyeR },
        b.closed,
        b.method,
      );
    }
    ctx.restore();
  }

  /** 分层剪纸式行走（Cutout Walk）：身体刚体 + 两腿绕虚拟髋点摆动。相位=距离锁定的 walkPhase。 */
  drawCutout(
    sheet: SpriteSheet,
    rig: CutoutRig,
    phase: number,
    walkDir: -1 | 1,
    opts?: { alpha?: number },
  ): void {
    drawCutoutWalk(
      { ctx: this.ctx, img: sheet.img, anchorX: this.anchorX, anchorY: this.anchorY, k: this.k },
      rig,
      phase,
      walkDir,
      opts,
    );
  }

  /** 弹跳降级 + 腿部铰接（团子）：上身整体弹跳，下身作为一整块绕髋微摆（无缝无撕裂）。 */
  drawBounceWalk(sheet: SpriteSheet, rig: LegSwingRig, phase: number, tf: Transform, alpha = 1): void {
    drawBounceWalk(
      { ctx: this.ctx, img: sheet.img, anchorX: this.anchorX, anchorY: this.anchorY, k: this.k },
      rig,
      phase,
      tf,
      alpha,
    );
  }

  /** 睡姿：把宠物"躺倒"（旋转→质心水平居中→贴地），头下垫枕、脚侧天角挂月，配闭眼+慢呼吸。
   *  躺倒仅一次整体旋转，不切分不形变，永不撕裂。 */
  drawSleeping(
    sheet: SpriteSheet,
    clip: ResolvedClip,
    frame: number,
    opts: { rot: number; breath?: number; blink?: BlinkOverlay; propsAlpha?: number; alpha?: number },
  ): void {
    const { cell } = sheet.spec;
    const ctx = this.ctx;
    const drawW = cell.w * this.k;
    const drawH = cell.h * this.k;
    const rot = opts.rot;
    const cos = Math.cos(rot);
    const sin = Math.sin(rot);
    // 旋转后的四角 → 水平居中(bcx)、最低点贴地(maxY)
    let minX = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const [x, y] of [
      [drawW / 2, 0],
      [-drawW / 2, 0],
      [drawW / 2, -drawH],
      [-drawW / 2, -drawH],
    ] as const) {
      const rx = x * cos - y * sin;
      const ry = x * sin + y * cos;
      if (rx < minX) minX = rx;
      if (rx > maxX) maxX = rx;
      if (ry > maxY) maxY = ry;
    }
    const ox = this.anchorX - (minX + maxX) / 2;
    const oy = this.anchorY - maxY;
    const lieMap = (ax: number, ay: number) => {
      const lx = -drawW / 2 + (ax / cell.w) * drawW;
      const ly = -drawH + (ay / cell.h) * drawH;
      return { x: ox + lx * cos - ly * sin, y: oy + lx * sin + ly * cos };
    };
    const a = sheet.anchors;
    const pa = (opts.propsAlpha ?? 1) * (opts.alpha ?? 1);

    // 枕头（垫在头/脸下，先画，宠物压其上）
    if (pa > 0.01 && a.eyeL && a.eyeR) {
      const fx = (a.head.x + a.eyeL.x + a.eyeR.x) / 3;
      const fy = (a.head.y + a.eyeL.y + a.eyeR.y) / 3;
      const face = lieMap(fx, fy);
      ctx.save();
      ctx.globalAlpha = pa * 0.95;
      drawPillow(ctx, face.x, face.y + 10 * this.k, 96 * this.k, 30 * this.k);
      ctx.restore();
    }

    // 宠物躺倒
    ctx.save();
    if ((opts.alpha ?? 1) < 1) ctx.globalAlpha = opts.alpha as number;
    ctx.imageSmoothingEnabled = false;
    ctx.translate(ox, oy);
    ctx.rotate(rot);
    const b = opts.breath ?? 1;
    if (b !== 1) ctx.scale(b, b);
    const frameX = frame * cell.w;
    const rowY = clip.row * cell.h;
    ctx.drawImage(sheet.img, frameX, rowY, cell.w, cell.h, -drawW / 2, -drawH, drawW, drawH);
    if (opts.blink) {
      drawBlink(
        ctx,
        sheet.img,
        frameX,
        rowY,
        { cellW: cell.w, cellH: cell.h, drawW, drawH, eyeL: opts.blink.eyeL, eyeR: opts.blink.eyeR },
        opts.blink.closed,
        opts.blink.method,
      );
    }
    ctx.restore();

    // 月亮（头朝向的天角：头顶上方——宠物睡前朝月亮方向躺、头冲着月亮）
    if (pa > 0.01) {
      const headP = lieMap(a.head.x, a.head.y);
      ctx.save();
      ctx.globalAlpha = pa;
      drawMoon(ctx, headP.x, this.anchorY - drawW * 0.95, 15 * this.k);
      ctx.restore();
    }
  }

  /** 两段式弯曲：下段(底 58%)正常绘制，上段(顶 42%+重叠)绕分割线旋转 bend 弧度（§13 尾摆·L0）。 */
  private drawBent(
    img: CanvasImageSource,
    frameX: number,
    rowY: number,
    cellW: number,
    cellH: number,
    drawW: number,
    drawH: number,
    bend: number,
  ): void {
    const ctx = this.ctx;
    const upperCell = cellH * (1 - BEND_SPLIT); // 顶部占比 42%
    const overlapCell = cellH * BEND_OVERLAP;
    const splitDestY = -drawH + (upperCell / cellH) * drawH; // 分割线（目标空间）
    const left = -drawW / 2;
    // 下段：源 [upperCell, cellH] → 目标 [splitDestY, 0]，高度 = -splitDestY
    const lowerSrcY = rowY + upperCell;
    const lowerSrcH = cellH - upperCell;
    ctx.drawImage(img, frameX, lowerSrcY, cellW, lowerSrcH, left, splitDestY, drawW, -splitDestY);
    // 上段：绕分割线中点旋转
    ctx.save();
    ctx.translate(0, splitDestY);
    ctx.rotate(bend);
    ctx.translate(0, -splitDestY);
    const upSrcH = upperCell + overlapCell;
    const upDestH = (upSrcH / cellH) * drawH;
    ctx.drawImage(img, frameX, rowY, cellW, upSrcH, left, -drawH, drawW, upDestH);
    ctx.restore();
  }

  /** 程序化蛋（无宠物占位 + 孵化仪式）。crack ∈ [0,1]，wobble 弧度。 */
  drawEgg(t: number, crack: number, wobble: number): void {
    const ctx = this.ctx;
    const k = this.k;
    const cx = this.anchorX;
    const bottom = this.anchorY;
    const rw = 62 * k;
    const rh = 80 * k;
    const cy = bottom - rh;

    ctx.save();
    ctx.translate(cx, bottom);
    ctx.rotate(wobble);
    ctx.translate(-cx, -bottom);

    // 蛋体
    ctx.beginPath();
    ctx.ellipse(cx, cy, rw, rh, 0, 0, Math.PI * 2);
    ctx.fillStyle = "#FFF7E0";
    ctx.fill();
    ctx.lineWidth = 5 * k;
    ctx.strokeStyle = "#543A2C";
    ctx.stroke();

    // 眼睛 + 腮红
    ctx.fillStyle = "#3C2A20";
    const eyeY = cy - rh * 0.12;
    ctx.beginPath();
    ctx.ellipse(cx - 20 * k, eyeY, 5 * k, 7 * k, 0, 0, Math.PI * 2);
    ctx.ellipse(cx + 20 * k, eyeY, 5 * k, 7 * k, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "rgba(255,150,160,0.6)";
    ctx.beginPath();
    ctx.ellipse(cx - 34 * k, cy + 6 * k, 9 * k, 5 * k, 0, 0, Math.PI * 2);
    ctx.ellipse(cx + 34 * k, cy + 6 * k, 9 * k, 5 * k, 0, 0, Math.PI * 2);
    ctx.fill();

    // 呼吸微动画（占位蛋会轻轻起伏是活着的信号）
    void t;

    // 裂纹
    if (crack > 0) {
      const pts: Array<[number, number]> = [
        [-rw * 0.9, 0.05],
        [-rw * 0.5, -0.09],
        [-rw * 0.15, 0.07],
        [rw * 0.2, -0.08],
        [rw * 0.55, 0.06],
        [rw * 0.9, -0.02],
      ];
      const visible = Math.max(2, Math.ceil(crack * pts.length));
      ctx.beginPath();
      for (let i = 0; i < Math.min(visible, pts.length); i++) {
        const [px, py] = pts[i] as [number, number];
        const x = cx + px;
        const y = cy + rh * (0.1 + py);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.lineWidth = 3.5 * k;
      ctx.strokeStyle = "#543A2C";
      ctx.stroke();
    }
    ctx.restore();
  }

  /**
   * 逐像素命中测试（CSS 坐标）：取样点周围 (2r+1)² 区域内是否有不透明像素。
   * 用于「宠物轮廓内才拦截鼠标」——轮廓外自动点击穿透，不干扰办公。
   */
  hitTest(cssX: number, cssY: number, radius = 6): boolean {
    const dpr = window.devicePixelRatio || 1;
    const r = Math.max(1, Math.round(radius * dpr));
    const px = Math.round(cssX * dpr);
    const py = Math.round(cssY * dpr);
    const x0 = Math.max(0, px - r);
    const y0 = Math.max(0, py - r);
    const w = Math.min(this.canvas.width - x0, r * 2 + 1);
    const h = Math.min(this.canvas.height - y0, r * 2 + 1);
    if (w <= 0 || h <= 0) return false;
    const data = this.ctx.getImageData(x0, y0, w, h).data;
    for (let i = 3; i < data.length; i += 4) {
      if ((data[i] ?? 0) > 24) return true;
    }
    return false;
  }

  /** 全屏白闪。 */
  flash(alpha: number): void {
    if (alpha <= 0) return;
    this.ctx.save();
    this.ctx.globalAlpha = Math.min(1, alpha);
    this.ctx.fillStyle = "#fff";
    this.ctx.fillRect(0, 0, this.w, this.h);
    this.ctx.restore();
  }
}
