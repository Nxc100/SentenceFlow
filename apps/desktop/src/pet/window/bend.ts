/** 两段式弯曲（生命感 §13 尾摆·L0）：无尾骨的 L0 宠物，把身体按高度切两段绘制，
 *  上段以**弹簧跟随**下段的驱动角轻摆，产生尾巴/身体的柔性摆动感。渲染在 renderer 中
 *  按 BEND_SPLIT 分两条 drawImage（重叠 BEND_OVERLAP 遮缝），本模块只提供弹簧状态与参数。
 *  幅度极小（≤1.4°）以保证任意画风都不违和（动作预算原则）。 */

/** 下段占比（自底向上 58%），上段 42% + 重叠。 */
export const BEND_SPLIT = 0.58;
export const BEND_OVERLAP = 0.03;
/** 上段最大偏摆角（弧度，≈1.4°）。 */
export const BEND_MAX = 0.024;

export class BodySway {
  private angle = 0; // 当前上段角（弧度）
  private vel = 0;

  /** 推进弹簧跟随。driver：下段驱动角（idle 用极慢正弦模拟尾摆意图）。返回上段当前角。 */
  update(dt: number, driver: number): number {
    const stiffness = 55;
    const damping = 8.5;
    const acc = (driver - this.angle) * stiffness - this.vel * damping;
    this.vel += acc * Math.min(dt, 0.05);
    this.angle += this.vel * Math.min(dt, 0.05);
    return this.angle;
  }

  get value(): number {
    return this.angle;
  }
}
