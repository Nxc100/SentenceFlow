/** 分层剪纸式行走（Cutout Walk，修复方案《分层剪纸式行走执行方案》）。
 *
 *  把行走从"播放烘焙/生成的 walk 帧"（易香蕉弯、跳帧、滑步）改为**运行时三层合成**：
 *    身体(yCut 以上，逐帧同一块 drawImage、零形变) + 左腿 + 右腿（作为刚体绕躯干内虚拟髋点摆动）。
 *  几何在 sprite 载入时由 idle 首帧算一次并缓存（CutoutRig）；无可辨双腿的团子返回 null，
 *  调用方走"弹跳降级"（§6）。相位约定沿用 F6：相位 0 = 触地（双腿分开最大）。
 *
 *  绘制顺序（腿在下、身体在上）让所有接缝落在身体不透明区内，不外露（§4）。 */

import type { PetSpec } from "../types";
import type { Transform } from "./motion";

/** 定型数组按下标取值。
 *  `noUncheckedIndexedAccess` 把 TypedArray 的下标读也判成 `number | undefined`,
 *  而下面这些像素循环的上下界全部由数组自身尺寸算出,越界不可能发生 ——
 *  用一个具名读取器把这件事说清楚,而不是满屏撒 `!`。 */
const at = (a: Uint8ClampedArray | Int32Array, i: number): number => a[i] as number;

const DEG = Math.PI / 180;
const TAU = Math.PI * 2;

export interface CutoutRig {
  srcX: number; // idle 首帧在图集中的左上角（源采样原点）
  srcY: number;
  cellW: number;
  cellH: number;
  yCut: number; // 切分线（cell px）：以上=身体，以下=腿
  overlap: number; // 腿源向上重叠带（含躯干像素，随腿动但被身体盖住）
  pivotX: number; // 髋支点（埋在躯干内，接缝不外露的关键）
  pivotY: number;
  splitX: number; // 左右腿分界（cell px）
  legX0: number; // 腿区左界 / 右界（源矩形范围）
  legX1: number;
  legLen: number; // 杠杆：footY - pivotY
  A: number; // 摆角幅度（rad）
  T: number; // 平移补偿（cell px）：旋转吃不下的步幅由它补，保证脚步=位移
  lift: number; // 摆动腿抬起（cell px）
  bobMax: number; // 身体起伏（cell px）
}

interface BBox {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  w: number;
  h: number;
}

/** 在 [yTop,yBot] × [x0,x1] 带内找"两侧都有腿肉的最深内部谷"。
 *  返回 {sep, valleyX}；sep = 1 - 谷/峰。团子无双峰 → sep 低 / valleyX=-1。 */
function interiorValley(
  col: Int32Array,
  x0: number,
): { sep: number; valleyX: number } {
  let peak = 0;
  for (const c of col) peak = Math.max(peak, c);
  if (peak === 0) return { sep: 0, valleyX: -1 };
  const n = col.length;
  const sideTh = peak * 0.5; // 两侧腿肉阈值
  const lo = Math.floor(n * 0.15);
  const hi = Math.ceil(n * 0.85);
  let best = { sep: 0, valleyX: -1 };
  for (let i = lo; i < hi; i++) {
    // 该列须两侧都存在 ≥ sideTh 的峰（真正夹在两条腿之间）
    let leftPeak = 0;
    for (let j = 0; j < i; j++) leftPeak = Math.max(leftPeak, at(col, j));
    let rightPeak = 0;
    for (let j = i + 1; j < n; j++) rightPeak = Math.max(rightPeak, at(col, j));
    if (leftPeak < sideTh || rightPeak < sideTh) continue;
    const sep = 1 - at(col, i) / Math.min(leftPeak, rightPeak);
    if (sep > best.sep) best = { sep, valleyX: x0 + i };
  }
  return best;
}

function columnHist(
  alpha: Uint8ClampedArray,
  W: number,
  yTop: number,
  yBot: number,
  x0: number,
  x1: number,
): Int32Array {
  const col = new Int32Array(x1 - x0);
  for (let y = yTop; y <= yBot; y++) {
    const row = y * W;
    for (let x = x0; x < x1; x++) {
      if (at(alpha, row + x) > 128) col[x - x0] = at(col, x - x0) + 1;
    }
  }
  return col;
}

/** 自底向上找"最高的仍双峰分离的一行"（§2.1）。找不到→null（团子）。 */
function findCut(
  alpha: Uint8ClampedArray,
  W: number,
  bb: BBox,
): { yCut: number; valleyX: number } | null {
  let yCut = -1;
  for (let y = bb.y1 - 4; y > bb.y0 + bb.h * 0.45; y -= 2) {
    const col = columnHist(alpha, W, y, bb.y1, bb.x0, bb.x1);
    const r = interiorValley(col, bb.x0);
    if (r.sep >= 0.55 && r.valleyX > bb.x0 + 4 && r.valleyX < bb.x1 - 4) {
      yCut = y; // 记录仍分离的最高行
    } else if (yCut >= 0) {
      break; // 一旦并拢即到达躯干，停止
    }
  }
  if (yCut < 0) return null;
  // 用整条腿带重算分界，稳定
  const col = columnHist(alpha, W, yCut, bb.y1, bb.x0, bb.x1);
  const r = interiorValley(col, bb.x0);
  if (r.valleyX < 0) return null;
  return { yCut, valleyX: r.valleyX };
}

interface FrameAlpha {
  a8: Uint8ClampedArray; // 每像素一字节的 alpha，索引 = y*cellW + x
  bb: BBox;
  cellW: number;
  cellH: number;
  srcX: number;
  srcY: number;
}

/** 抽取 idle 首帧的 alpha 位图与不透明包围盒。失败/全透明 → null。 */
function readAlpha(img: CanvasImageSource, spec: PetSpec): FrameAlpha | null {
  const cellW = spec.cell.w;
  const cellH = spec.cell.h;
  const idle = spec.states["idle"];
  const srcX = 0;
  const srcY = (idle ? idle.row : 0) * cellH;
  let a8: Uint8ClampedArray;
  try {
    const cv = document.createElement("canvas");
    cv.width = cellW;
    cv.height = cellH;
    const ctx = cv.getContext("2d", { willReadFrequently: true })!;
    ctx.drawImage(img, srcX, srcY, cellW, cellH, 0, 0, cellW, cellH);
    a8 = ctx.getImageData(0, 0, cellW, cellH).data.filter((_, i) => i % 4 === 3) as unknown as Uint8ClampedArray;
  } catch {
    return null;
  }
  let x0 = cellW,
    y0 = cellH,
    x1 = 0,
    y1 = 0,
    found = false;
  for (let y = 0; y < cellH; y++) {
    for (let x = 0; x < cellW; x++) {
      if (at(a8, y * cellW + x) > 128) {
        found = true;
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  if (!found) return null;
  return { a8, bb: { x0, y0, x1, y1, w: x1 - x0, h: y1 - y0 }, cellW, cellH, srcX, srcY };
}

/** 扫描 idle 首帧 alpha，构建分层行走绑定。stride 与运行时 walkPhase 同单位（BASE_W≈cell px）。
 *  返回 null = 该形象不适合分层（团子 / 腿太短 / 步幅太大），调用方走弹跳降级（§6）。 */
export function buildRig(
  img: CanvasImageSource,
  spec: PetSpec,
  stride: number,
): CutoutRig | null {
  const fa = readAlpha(img, spec);
  if (!fa) return null;
  const { a8: A8, bb, cellW, cellH, srcX, srcY } = fa;

  const cut = findCut(A8, cellW, bb);
  if (!cut) return null;
  const { yCut, valleyX } = cut;

  // —— 腿区几何检查（§6：碎片非腿即降级）——
  const legX0 = bb.x0;
  const legX1 = bb.x1;
  const leftW = valleyX - legX0;
  const rightW = legX1 - valleyX;
  if (Math.min(leftW, rightW) < bb.w * 0.12) return null; // 一侧过窄=非双腿
  if (bb.y1 - yCut < bb.h * 0.1) return null; // 腿太矮=碎片

  // —— 真实腿间隙校验（关键，防"身体割裂"）——
  // 分层旋转只在"两腿之间是背景空隙"时不撕裂。若切分列在腿高内仍大部分是实体像素
  // （连续躯干 / 毛绒连体衣 / 团子），旋转两半会把身体撕开 → 判不可分层，走弹跳降级(§6)。
  const bandH = bb.y1 - yCut;
  let gapOpaque = bandH;
  for (let vx = Math.max(legX0, valleyX - 2); vx <= Math.min(legX1, valleyX + 2); vx++) {
    let cnt = 0;
    for (let y = yCut; y <= bb.y1; y++) if (at(A8, y * cellW + vx) > 128) cnt++;
    gapOpaque = Math.min(gapOpaque, cnt);
  }
  if (gapOpaque > 0.25 * bandH) return null; // 间隙大部分是实体 → 会撕裂 → 降级

  // —— 支点与杠杆（§2.2）——
  const pivotY = yCut - 0.18 * bb.h; // 埋进躯干
  const pivotX = valleyX; // 双腿几何中界即髋中心
  const footY = bb.y1 - 0.02 * bb.h;
  const legLen = footY - pivotY;
  if (legLen < 8) return null;

  // —— 摆角与平移由步幅反解（§2.3），先旋转吃步幅、不足平移补，步幅自洽由构造保证 ——
  const halfStride = stride / 2;
  const solve = (capDeg: number) => {
    const a = Math.min(capDeg * DEG, Math.asin(Math.min(0.95, halfStride / legLen)));
    const t = Math.max(0, halfStride - legLen * Math.sin(a));
    return { a, t };
  };
  let { a: A, t: T } = solve(20);
  if (T > 0.12 * bb.w) ({ a: A, t: T } = solve(28)); // 提高摆角上限重解
  if (T > 0.12 * bb.w) return null; // 仍超标：腿太短/步幅太大 → 降级

  return {
    srcX,
    srcY,
    cellW,
    cellH,
    yCut,
    overlap: 0.1 * bb.h,
    pivotX,
    pivotY,
    splitX: valleyX,
    legX0,
    legX1,
    legLen,
    A,
    T,
    lift: 0.035 * bb.h,
    bobMax: 0.022 * bb.h,
  };
}

export interface LegXf {
  theta: number;
  dx: number;
  dy: number;
  front: boolean;
}

/** 相位 → 左右腿变换（§3，闭式）。walkDir=+1 右行 / -1 左行（镜像在渲染外层处理）。 */
export function legTransforms(rig: CutoutRig, phase: number): { L: LegXf; R: LegXf } {
  const phiL = 2 * Math.PI * phase;
  const phiR = phiL + Math.PI; // 左右反相
  const leg = (phi: number): LegXf => {
    const c = Math.cos(phi);
    return {
      theta: rig.A * c, // 前摆为正
      dx: rig.T * c, // 平移补足步幅
      dy: -rig.lift * Math.max(0, -Math.sin(phi)), // 仅"向前摆动中"的腿抬起
      front: c >= 0, // 余弦为正者在前（后画）
    };
  };
  return { L: leg(phiL), R: leg(phiR) };
}

/** 身体层起伏与前倾（§3）：触地最低、经过位最高；不做形变。 */
export function bodyMotion(rig: CutoutRig, phase: number, walkDir: -1 | 1): { dy: number; lean: number } {
  const phiL = 2 * Math.PI * phase;
  return { dy: -rig.bobMax * Math.abs(Math.sin(phiL)), lean: walkDir * 0.035 };
}

export interface CutoutGeom {
  ctx: CanvasRenderingContext2D;
  img: CanvasImageSource;
  anchorX: number;
  anchorY: number;
  k: number; // 显示比例
}

/** 三层绘制：后腿(压暗) → 前腿 → 身体(盖住腿顶接缝)。bend 仅施加于身体层。 */
export function drawCutoutWalk(
  g: CutoutGeom,
  rig: CutoutRig,
  phase: number,
  walkDir: -1 | 1,
  opts?: { alpha?: number; bend?: number; drawBent?: (bodyBottomCell: number) => void },
): void {
  const { ctx, img, anchorX, anchorY, k } = g;
  const drawW = rig.cellW * k;
  const drawH = rig.cellH * k;
  const mirror = walkDir < 0;
  // 镜像时交换左右腿相位（§4）
  const { L, R } = legTransforms(rig, mirror ? phase + 0.5 : phase);
  const body = bodyMotion(rig, phase, walkDir);

  ctx.save();
  ctx.globalAlpha = opts?.alpha ?? 1;
  ctx.imageSmoothingEnabled = false;
  ctx.translate(anchorX, anchorY + body.dy * k);
  ctx.rotate(body.lean);
  if (mirror) ctx.scale(-1, 1);
  ctx.translate(-drawW / 2, -drawH); // 原点=cell 左上，cell 坐标 ×k 直出

  const legTop = rig.yCut - rig.overlap;
  const legSrcH = rig.cellH - legTop;
  const drawLeg = (xf: LegXf, sx0: number, sx1: number, darken: number) => {
    ctx.save();
    if (darken < 1) ctx.filter = `brightness(${darken})`;
    ctx.translate((rig.pivotX + xf.dx) * k, (rig.pivotY + xf.dy) * k);
    ctx.rotate(xf.theta);
    ctx.translate(-rig.pivotX * k, -rig.pivotY * k);
    ctx.drawImage(
      img,
      rig.srcX + sx0,
      rig.srcY + legTop,
      sx1 - sx0,
      legSrcH,
      sx0 * k,
      legTop * k,
      (sx1 - sx0) * k,
      legSrcH * k,
    );
    ctx.restore();
  };

  // 后腿先画并压暗 8%（剪纸景深），前腿后画
  const back = L.front ? R : L;
  const front = L.front ? L : R;
  const backIsLeft = back === L;
  drawLeg(back, backIsLeft ? rig.legX0 : rig.splitX, backIsLeft ? rig.splitX : rig.legX1, 0.92);
  drawLeg(front, backIsLeft ? rig.splitX : rig.legX0, backIsLeft ? rig.legX1 : rig.splitX, 1);

  // 身体层：yCut 以上整块直出，盖住腿顶接缝；bend 仅此层
  if (opts?.bend && opts.drawBent) {
    opts.drawBent(rig.yCut);
  } else {
    ctx.drawImage(img, rig.srcX, rig.srcY, rig.cellW, rig.yCut, 0, 0, drawW, rig.yCut * k);
  }
  ctx.restore();
}

// ============================================================================
//  弹跳降级 + 腿部铰接摆动（团子/连续躯干专用）
//  —— 不切分、不分片：下身作为"一整块"绕髋线做微小旋转（铰接），上身盖住接缝。
//     连续块旋转在数学上不会产生任何割裂/裁剪（对比"切成两片旋转"必撕裂）。
//     同 bend.ts 尾摆的成熟手法。摆动使一脚微抬、一脚微落 → 交替步态的观感。
// ============================================================================

export interface LegSwingRig {
  srcX: number;
  srcY: number;
  cellW: number;
  cellH: number;
  hipY: number; // 髋铰接线：其下一整块绕髋摆动
  overlap: number; // 上身盖住铰接缝的重叠带
  pivotX: number; // 摆动支点 x（下身水平质心）
  swing: number; // 最大摆角(rad)
}

/** 构建腿部铰接：取下 ~22% 作为腿段，支点=下身水平质心。连续块旋转，任何形象都不会撕裂。 */
export function buildLegSwing(img: CanvasImageSource, spec: PetSpec): LegSwingRig | null {
  const fa = readAlpha(img, spec);
  if (!fa) return null;
  const { a8, bb, cellW, cellH, srcX, srcY } = fa;
  const hipY = Math.round(bb.y1 - bb.h * 0.22);
  if (bb.y1 - hipY < 6) return null; // 腿段太薄
  let sumX = 0;
  let n = 0;
  for (let y = hipY; y <= bb.y1; y++) {
    for (let x = bb.x0; x <= bb.x1; x++) {
      if (at(a8, y * cellW + x) > 128) {
        sumX += x;
        n++;
      }
    }
  }
  const pivotX = n ? sumX / n : (bb.x0 + bb.x1) / 2;
  return {
    srcX,
    srcY,
    cellW,
    cellH,
    hipY,
    overlap: Math.max(4, bb.h * 0.06),
    pivotX,
    swing: 0.075, // ≈4.3°，下身整体微摆
  };
}

/** 弹跳 + 腿部铰接：上身随 tf 整体弹跳；下身(hipY 以下)作为一整块绕髋点小幅旋转（铰接，绝不撕裂），
 *  再由上身盖住接缝。摆动令两脚交替微抬/微落，产生走路观感。tf = motion 的弹跳变换。 */
export function drawBounceWalk(
  g: CutoutGeom,
  rig: LegSwingRig,
  phase: number,
  tf: Transform,
  alpha = 1,
): void {
  const { ctx, img, anchorX, anchorY, k } = g;
  const drawW = rig.cellW * k;
  const drawH = rig.cellH * k;
  const flip = tf.mirror;
  const swing = rig.swing * Math.sin(TAU * (flip ? phase + 0.5 : phase));

  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.imageSmoothingEnabled = false;
  ctx.translate(anchorX + tf.dx * k, anchorY + tf.dy * k);
  ctx.rotate(tf.rot);
  ctx.scale(tf.sx * (flip ? -1 : 1), tf.sy);
  ctx.translate(-drawW / 2, -drawH); // 原点=cell 左上

  // 下段：绕髋点旋转（一整块连续像素，铰接处与上身共线、由重叠+上身遮盖，无缝无撕裂）
  const lowTop = rig.hipY - rig.overlap;
  ctx.save();
  ctx.translate(rig.pivotX * k, rig.hipY * k);
  ctx.rotate(swing);
  ctx.translate(-rig.pivotX * k, -rig.hipY * k);
  ctx.drawImage(
    img,
    rig.srcX,
    rig.srcY + lowTop,
    rig.cellW,
    rig.cellH - lowTop,
    0,
    lowTop * k,
    drawW,
    (rig.cellH - lowTop) * k,
  );
  ctx.restore();

  // 上身：hipY 以上整块直出，盖住铰接缝
  ctx.drawImage(img, rig.srcX, rig.srcY, rig.cellW, rig.hipY, 0, 0, drawW, rig.hipY * k);
  ctx.restore();
}
