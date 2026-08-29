/** 眨眼·双渲染法（生命感 §13）：无烘焙闭眼帧时，在已绘制的角色帧上叠加闭眼效果。
 *
 *  两法按素材类型选择（§13）：
 *   - 压缩法 compress（照片/3D 渲染风，也是通用安全兜底）：把眼部横带纵向压扁到 ~15%，
 *     上下皮肤像素拉伸补位（分条 drawImage），不引入违和卡通线条。
 *   - 遮盖法 mask（平涂 Q 版最佳）：眼位画肤色椭圆 + 描边色下弯弧「∪」，接近手绘。
 *
 *  仅在**双眼已校准**（认主仪式 §14.2）或素材非照片类时启用；照片类默认关闭（规则检查项）。
 *  绘制发生在 renderer 的角色变换空间内：单元格坐标 (cx,cy) → 目标坐标
 *    destX = -drawW/2 + cx/cellW·drawW,  destY = -drawH + cy/cellH·drawH。 */

import type { AnchorPoint } from "./props";

export type BlinkMethod = "compress" | "mask";

export interface BlinkGeom {
  cellW: number;
  cellH: number;
  drawW: number;
  drawH: number;
  eyeL: AnchorPoint;
  eyeR: AnchorPoint;
}

const OUTLINE = "#543A2C";
/** 肤色采样缓存（每张图集采一次，避免逐帧回读）。 */
const skinCache = new WeakMap<CanvasImageSource, string>();

/**
 * 在当前变换空间内叠加闭眼。closed ∈ [0,1]：0 睁眼（不绘制），1 全闭。
 * frameX/rowY 为当前帧在图集中的源像素左上角。
 */
export function drawBlink(
  ctx: CanvasRenderingContext2D,
  sheet: CanvasImageSource,
  frameX: number,
  rowY: number,
  geom: BlinkGeom,
  closed: number,
  method: BlinkMethod,
): void {
  if (closed <= 0.01) return;
  if (method === "mask") drawMask(ctx, sheet, frameX, rowY, geom, closed);
  else drawCompress(ctx, sheet, frameX, rowY, geom, closed);
}

/** 压缩法：眼带纵向压扁 + 上下皮肤拉伸补位。 */
function drawCompress(
  ctx: CanvasRenderingContext2D,
  sheet: CanvasImageSource,
  frameX: number,
  rowY: number,
  g: BlinkGeom,
  closed: number,
): void {
  const eyeYCell = (g.eyeL.y + g.eyeR.y) / 2;
  const bh = g.cellH * 0.065; // 眼带半高（cell px）
  const bandTopCell = eyeYCell - bh;
  const bandBotCell = eyeYCell + bh;

  const toDestY = (cy: number) => -g.drawH + (cy / g.cellH) * g.drawH;
  const left = -g.drawW / 2;
  const bandTop = toDestY(bandTopCell);
  const bandBot = toDestY(bandBotCell);
  const mid = (bandTop + bandBot) / 2;
  const eyeH = (bandBot - bandTop) * (1 - closed * 0.85); // 闭合时压到 ~15%
  const eyeTop = mid - eyeH / 2;
  const eyeBot = mid + eyeH / 2;

  ctx.save();
  ctx.imageSmoothingEnabled = false;
  // 上皮肤：眼带上缘那一行拉伸填满上半空缺
  if (eyeTop > bandTop) {
    ctx.drawImage(sheet, frameX, Math.max(0, rowY + bandTopCell - 1), g.cellW, 1, left, bandTop, g.drawW, eyeTop - bandTop);
  }
  // 下皮肤：眼带下缘那一行拉伸填满下半空缺
  if (bandBot > eyeBot) {
    ctx.drawImage(sheet, frameX, rowY + bandBotCell, g.cellW, 1, left, eyeBot, g.drawW, bandBot - eyeBot);
  }
  // 压缩的眼带本体
  ctx.drawImage(
    sheet,
    frameX,
    rowY + bandTopCell,
    g.cellW,
    bandBotCell - bandTopCell,
    left,
    eyeTop,
    g.drawW,
    Math.max(1, eyeH),
  );
  ctx.restore();
}

/** 遮盖法：肤色椭圆盖眼 + 下弯弧。 */
function drawMask(
  ctx: CanvasRenderingContext2D,
  sheet: CanvasImageSource,
  frameX: number,
  rowY: number,
  g: BlinkGeom,
  closed: number,
): void {
  const skin = sampleSkin(sheet, frameX, rowY, g);
  const toDest = (a: AnchorPoint) => ({
    x: -g.drawW / 2 + (a.x / g.cellW) * g.drawW,
    y: -g.drawH + (a.y / g.cellH) * g.drawH,
  });
  const rx = (g.drawW / g.cellW) * g.cellW * 0.05; // 眼半宽 ≈ 5% cell
  const ry = rx * 0.85;
  ctx.save();
  for (const eye of [g.eyeL, g.eyeR]) {
    const p = toDest(eye);
    // 肤色盖片
    ctx.fillStyle = skin;
    ctx.beginPath();
    ctx.ellipse(p.x, p.y, rx, ry, 0, 0, Math.PI * 2);
    ctx.fill();
    // 闭眼弧「∪」，随 closed 加深
    ctx.strokeStyle = OUTLINE;
    ctx.globalAlpha = closed;
    ctx.lineWidth = Math.max(1.4, rx * 0.28);
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.arc(p.x, p.y - ry * 0.2, rx * 0.9, Math.PI * 0.15, Math.PI * 0.85);
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
  ctx.restore();
}

/** 采样额头肤色（眼上方），缓存到图集。失败回退浅肤色。 */
function sampleSkin(sheet: CanvasImageSource, frameX: number, rowY: number, g: BlinkGeom): string {
  const cached = skinCache.get(sheet);
  if (cached) return cached;
  let color = "#F4D6BE";
  try {
    const cv = document.createElement("canvas");
    cv.width = 1;
    cv.height = 1;
    const c = cv.getContext("2d", { willReadFrequently: true })!;
    // 采样两眼中点略上方（眉间/鼻梁上）——稳定落在脸部肤色上；原 -0.09 偏高易采到帽檐/发际。
    const foreheadY = rowY + (g.eyeL.y + g.eyeR.y) / 2 - g.cellH * 0.04;
    const foreheadX = frameX + (g.eyeL.x + g.eyeR.x) / 2;
    c.drawImage(sheet, foreheadX, Math.max(0, foreheadY), 1, 1, 0, 0, 1, 1);
    const d = c.getImageData(0, 0, 1, 1).data;
    if ((d[3] ?? 0) > 40) color = `rgb(${d[0]},${d[1]},${d[2]})`;
  } catch {
    /* 采样失败：用兜底肤色 */
  }
  skinCache.set(sheet, color);
  return color;
}
