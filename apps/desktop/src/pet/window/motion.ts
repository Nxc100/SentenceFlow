/** 分层动画栈·变形层（生命感 §12.2 / §13）。
 *
 *  v2.0：删除「有真素材即返回 IDENTITY」短路（病因#1）。改为**通道掩码混合**——
 *  有真帧时关闭形变通道(sx/sy)、保留物理通道(dx/dy/rot)，让真帧管形变、程序管物理；
 *  无真帧(L0)时走完整程序化动效。所有周期项由**帧相位**驱动，程序曲线与帧序永不漂移。 */

import type { StateKey } from "../types";

export interface Transform {
  dx: number; // 逻辑 px（renderer 按显示倍率缩放）
  dy: number;
  rot: number; // 弧度
  sx: number;
  sy: number;
  mirror: boolean;
}

export const IDENTITY: Transform = { dx: 0, dy: 0, rot: 0, sx: 1, sy: 1, mirror: false };

export interface MotionCtx {
  /** 有真实动画帧 → 关闭形变通道，只叠物理。 */
  hasRealClip: boolean;
  /** [0,1) 循环相位：真帧=帧相位；L0=时间/距离相位（相位锁定 §12.2）。 */
  phase: number;
  walkDir: -1 | 1;
  /** 0..1 调头预备进度（前 ~100ms 减速+挤压，§13「禁止行进中瞬翻」）。 */
  turnPrep: number;
  /** 动作预算幅度系数：宏观微动作进行时，呼吸幅度降到 30%（§13 动作预算原则）。 */
  budget: number;
}

const TAU = Math.PI * 2;

/** t：进入该状态后的秒数。 */
export function motionFor(mode: StateKey, t: number, ctx: MotionCtx): Transform {
  const { hasRealClip, phase, walkDir, turnPrep, budget } = ctx;

  switch (mode) {
    case "idle":
      return breathe(phase, hasRealClip, budget);

    case "walk-right":
    case "walk-left": {
      const mirror = mode === "walk-left";
      // 触地起伏：一循环两次触地（相位 0 与 0.5 为最低点）。
      const bob = hasRealClip ? 3.5 : 6;
      const lift = -Math.abs(Math.sin(TAU * phase)) * bob;
      // 行进前倾 2–4°；调头预备时收直（减速信号）。
      const lean = walkDir * 0.055 * (1 - 0.7 * turnPrep);
      const tf: Transform = { ...IDENTITY, dy: lift, rot: lean, mirror };
      if (!hasRealClip) {
        // 弹跳降级（团子/无腿，含从分层校验降级下来的圆润体型）：整体挤压弹跳 + 轻微 waddle
        // 左右摇——一个刚体、零切分零撕裂，是圆团子的正确动画语言（修复方案 §6）。
        const contact = 1 - Math.abs(Math.sin(TAU * phase));
        tf.sy = 1 - 0.07 * contact - 0.1 * turnPrep;
        tf.sx = 1 + 0.05 * contact + 0.07 * turnPrep;
        tf.rot = lean * 0.55 + 0.05 * Math.sin(TAU * phase); // 前倾略收 + waddle 摇摆(±2.9°)
      }
      return tf;
    }

    case "happy":
      return jump5(t, hasRealClip);

    case "eat":
      return eat(t, hasRealClip);

    case "sleep": {
      // 半速缓慢呼吸 + 轻微侧卧倾斜。
      const s = Math.sin(TAU * phase);
      if (hasRealClip) return { ...IDENTITY, rot: 0.04 + 0.004 * s };
      return { ...IDENTITY, sy: 1 + 0.018 * s, sx: 1 - 0.009 * s, rot: 0.04 };
    }

    case "remind":
      // 抖动 + 小跳（物理通道，真帧亦叠加）。
      return {
        ...IDENTITY,
        dx: 2.5 * Math.sin(t * 26),
        rot: 0.045 * Math.sin(t * 22),
        dy: -Math.abs(Math.sin(t * 6)) * 5,
      };

    case "drag":
      return { ...IDENTITY, rot: 0.16 + 0.05 * Math.sin(t * 3), dy: -4 };

    default:
      return IDENTITY;
  }
}

/** 呼吸：体积守恒挤压（sy 上鼓、sx 反向 50% 补偿），**无 dy 整体浮动**（病因#3）。
 *  有真帧时帧内已含呼吸，仅留极慢 ±0.3° 微旋消僵硬。 */
function breathe(phase: number, hasRealClip: boolean, budget: number): Transform {
  if (hasRealClip) {
    return { ...IDENTITY, rot: 0.005 * Math.sin(TAU * phase * 0.5) };
  }
  const amp = 0.025 * budget;
  const s = Math.sin(TAU * phase);
  return { ...IDENTITY, sy: 1 + amp * s, sx: 1 - amp * 0.5 * s };
}

/** 跳·五段物理（§13「跳」）：预备→拉伸→抛物线滞空→落地→弹簧回弹×2。
 *  有真帧时仅保留 dy/rot 物理，形变交给真帧。 */
function jump5(t: number, hasRealClip: boolean): Transform {
  const H = 34; // 滞空峰高（逻辑 px）
  let dx = 0,
    dy = 0,
    rot = 0,
    sx = 1,
    sy = 1;
  if (t < 0.1) {
    // 预备：下蹲蓄力
    const k = t / 0.1;
    sy = 1 - 0.12 * k;
    sx = 1 + 0.09 * k;
    dy = 3 * k;
  } else if (t < 0.2) {
    // 拉伸：蹬地起跳
    const k = (t - 0.1) / 0.1;
    sy = 1 + 0.08 * k;
    sx = 1 - 0.06 * k;
    dy = -H * 0.25 * k;
  } else if (t < 0.65) {
    // 抛物线滞空 ~450ms
    const k = (t - 0.2) / 0.45; // 0..1
    const arc = Math.sin(Math.PI * k);
    dy = -H * 0.25 - H * arc;
    rot = k < 0.5 ? -0.035 * (1 - k * 2) : 0.05 * (k - 0.5) * 2; // 升段后仰 2°→降段前倾 3°
    sy = 1 + 0.03 * arc;
  } else if (t < 0.71) {
    // 落地挤压
    const k = (t - 0.65) / 0.06;
    sy = 1 - 0.15 * (1 - k);
    sx = 1 + 0.12 * (1 - k);
  } else {
    // 弹簧回弹×2（阻尼正弦）
    const k = t - 0.71;
    const damp = Math.exp(-k * 6);
    sy = 1 + 0.06 * Math.sin(k * 22) * damp;
    sx = 1 - 0.04 * Math.sin(k * 22) * damp;
  }
  if (hasRealClip) {
    // 关形变通道：真帧管挤压表情，程序只驱动 dy 弹跳与旋转。
    return { ...IDENTITY, dx, dy, rot };
  }
  return { dx, dy, rot, sx, sy, mirror: false };
}

/** 吃·咀嚼编排（§13「吃」）：三口在 0.5/0.9/1.3s，每口 sy0.95/90ms + 头前探。
 *  饼干道具由 props.ts 绘制；此处只出咀嚼脉冲与末尾开心小摆。 */
function eat(t: number, hasRealClip: boolean): Transform {
  const bites = [0.5, 0.9, 1.3];
  let pulse = 0;
  for (const b of bites) {
    const d = t - b;
    if (d >= 0 && d < 0.09) pulse = Math.max(pulse, Math.sin((d / 0.09) * Math.PI));
  }
  const dy = pulse * 2 + (t > 1.5 ? Math.sin((t - 1.5) * 12) * 1.5 * Math.exp(-(t - 1.5) * 2) : 0);
  if (hasRealClip) {
    return { ...IDENTITY, dy };
  }
  return { ...IDENTITY, dy, sy: 1 - 0.05 * pulse, sx: 1 + 0.035 * pulse };
}

/** 落地挤压（拖拽放手后 0.35s）。 */
export function landingSquash(sinceLandMs: number): Transform {
  const t = sinceLandMs / 350;
  if (t >= 1) return IDENTITY;
  const k = Math.sin(Math.PI * t) * (1 - t);
  return { ...IDENTITY, sy: 1 - 0.22 * k, sx: 1 + 0.16 * k };
}
