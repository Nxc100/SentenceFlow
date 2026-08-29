/** 道具叠加引擎（改进方案环节四）：把 AI 最难画的持物类状态换成内置道具动画。
 *  eat → 饼干弧线飞向嘴部；remind → 闹钟头顶弹跳摆铃；sleep → 枕头淡入 + 月亮悬浮。
 *  道具为程序化绘制的 Q 版粗描边平涂风格，与素材规范同风格，100% 稳定。 */

import type { StateKey } from "../types";

export interface AnchorPoint {
  x: number;
  y: number;
}

/** 单元格坐标系锚点（0..192 × 0..208）。 */
export interface PetAnchors {
  mouth: AnchorPoint;
  head: AnchorPoint;
  feet: AnchorPoint;
  /** 双眼（认主校准 §14.2 写入，或包围盒启发）。缺省时眨眼双渲染不启用（保守，避免误伤）。 */
  eyeL?: AnchorPoint;
  eyeR?: AnchorPoint;
  /** 眼睛锚点是否来自校准（true）而非启发式（false）——决定眨眼是否默认开启。 */
  eyesCalibrated?: boolean;
}

const OUTLINE = "#543A2C";

function outlined(ctx: CanvasRenderingContext2D, lw: number, draw: () => void): void {
  ctx.save();
  ctx.lineWidth = lw;
  ctx.strokeStyle = OUTLINE;
  ctx.lineJoin = "round";
  draw();
  ctx.restore();
}

function drawCookie(ctx: CanvasRenderingContext2D, x: number, y: number, r: number): void {
  outlined(ctx, r * 0.22, () => {
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = "#E8A75D";
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = "#8A5A2B";
    const chips: Array<[number, number]> = [
      [-0.35, -0.25],
      [0.3, -0.1],
      [-0.05, 0.35],
      [0.25, 0.4],
      [-0.45, 0.2],
    ];
    for (const [dx, dy] of chips) {
      ctx.beginPath();
      ctx.arc(x + dx * r, y + dy * r, r * 0.14, 0, Math.PI * 2);
      ctx.fill();
    }
  });
}

function drawClock(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, ring: number): void {
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(ring * 0.25);
  outlined(ctx, r * 0.2, () => {
    // 双铃
    ctx.fillStyle = "#FFD166";
    for (const side of [-1, 1]) {
      ctx.beginPath();
      ctx.arc(side * r * 0.55, -r * 0.85, r * 0.32, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
    }
    // 表体
    ctx.beginPath();
    ctx.arc(0, 0, r, 0, Math.PI * 2);
    ctx.fillStyle = "#FF7694";
    ctx.fill();
    ctx.stroke();
    // 表盘
    ctx.beginPath();
    ctx.arc(0, 0, r * 0.68, 0, Math.PI * 2);
    ctx.fillStyle = "#FFF7E0";
    ctx.fill();
    // 指针
    ctx.beginPath();
    ctx.moveTo(0, 0);
    ctx.lineTo(0, -r * 0.5);
    ctx.moveTo(0, 0);
    ctx.lineTo(r * 0.36, r * 0.1);
    ctx.lineWidth = r * 0.12;
    ctx.stroke();
  });
  ctx.restore();
}

export function drawMoon(ctx: CanvasRenderingContext2D, x: number, y: number, r: number): void {
  outlined(ctx, r * 0.16, () => {
    ctx.beginPath();
    ctx.arc(x, y, r, Math.PI * 0.25, Math.PI * 1.75);
    ctx.arc(x + r * 0.55, y, r * 0.72, Math.PI * 1.62, Math.PI * 0.38, true);
    ctx.closePath();
    ctx.fillStyle = "#FFD166";
    ctx.fill();
    ctx.stroke();
  });
}

export function drawPillow(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number): void {
  outlined(ctx, h * 0.18, () => {
    ctx.beginPath();
    ctx.roundRect(x - w / 2, y - h / 2, w, h, h * 0.45);
    ctx.fillStyle = "#BFD9FF";
    ctx.fill();
    ctx.stroke();
  });
}

/**
 * 按状态渲染道具层（在角色图层之上）。
 * cellToScreen：单元格坐标 → 画布坐标（由 renderer 提供，含镜像/缩放）。
 */
export function drawProps(
  ctx: CanvasRenderingContext2D,
  mode: StateKey,
  t: number,
  anchors: PetAnchors,
  k: number,
  cellToScreen: (cx: number, cy: number) => { x: number; y: number },
): void {
  switch (mode) {
    case "eat": {
      // 饼干从右前方弧线飞向嘴部，接近后缩小消失（1.2s 一轮）
      const cycle = (t % 1.2) / 1.2;
      const from = { x: anchors.mouth.x + 74, y: anchors.mouth.y + 14 };
      const to = anchors.mouth;
      const px = from.x + (to.x - from.x) * cycle;
      const py = from.y + (to.y - from.y) * cycle - Math.sin(Math.PI * cycle) * 26;
      const shrink = cycle > 0.72 ? Math.max(0, 1 - (cycle - 0.72) / 0.28) : 1;
      if (shrink > 0.05) {
        const p = cellToScreen(px, py);
        drawCookie(ctx, p.x, p.y, 13 * k * shrink);
      }
      break;
    }
    case "remind": {
      // 闹钟在头顶侧方弹跳 + 摆铃
      const p = cellToScreen(anchors.head.x - 34, anchors.head.y - 20 - Math.abs(Math.sin(t * 6)) * 12);
      drawClock(ctx, p.x, p.y, 15 * k, Math.sin(t * 22));
      break;
    }
    case "sleep": {
      // 枕头淡入于脚下，月亮悬于头顶
      const fade = Math.min(1, t / 1.2);
      ctx.save();
      ctx.globalAlpha = fade * 0.95;
      const pillow = cellToScreen(anchors.feet.x, anchors.feet.y - 4);
      drawPillow(ctx, pillow.x, pillow.y, 84 * k, 26 * k);
      ctx.restore();
      ctx.save();
      ctx.globalAlpha = fade;
      const moon = cellToScreen(anchors.head.x + 46, anchors.head.y - 30 + Math.sin(t * 0.9) * 4);
      drawMoon(ctx, moon.x, moon.y, 14 * k);
      ctx.restore();
      break;
    }
    default:
      break;
  }
}
