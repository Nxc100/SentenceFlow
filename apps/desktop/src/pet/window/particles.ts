/** 粒子引擎（方案 §6 P0-4）：爱心 / Zzz / 汗滴 / 铃铛 / 饼干屑 / 彩纸 / 星光。 */

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number; // 0..1 递减
  decay: number;
  text: string;
  size: number;
  rot: number;
  vr: number;
  gravity: number;
}

export type ParticleKind =
  | "hearts"
  | "zzz"
  | "sweat"
  | "bell"
  | "crumbs"
  | "confetti"
  | "sparkle";

const rand = (a: number, b: number) => a + Math.random() * (b - a);

const GLYPHS: Record<ParticleKind, string[]> = {
  hearts: ["❤️", "💕", "💗"],
  zzz: ["💤"],
  sweat: ["💦"],
  bell: ["🔔", "⏰"],
  crumbs: ["🍪", "•", "•"],
  confetti: ["🎉", "✨", "🎊"],
  sparkle: ["✨", "⭐"],
};

export class Particles {
  private list: Particle[] = [];

  spawn(kind: ParticleKind, x: number, y: number, count = 0): void {
    const glyphs = GLYPHS[kind];
    const n =
      count ||
      ({ hearts: 5, zzz: 1, sweat: 2, bell: 1, crumbs: 4, confetti: 14, sparkle: 3 }[kind]);
    for (let i = 0; i < n; i++) {
      // GLYPHS 每一项都至少一个字形,`?? glyphs[0]` 只为让类型收敛
      const text = glyphs[Math.floor(Math.random() * glyphs.length)] ?? glyphs[0] ?? "";
      let p: Particle;
      switch (kind) {
        case "hearts":
          p = { x: x + rand(-28, 28), y: y + rand(-16, 8), vx: rand(-14, 14), vy: rand(-70, -40), life: 1, decay: rand(0.5, 0.9), text, size: rand(13, 20), rot: rand(-0.3, 0.3), vr: rand(-1, 1), gravity: -12 };
          break;
        case "zzz":
          p = { x: x + rand(8, 22), y: y + rand(-30, -14), vx: rand(8, 18), vy: rand(-26, -16), life: 1, decay: 0.42, text, size: rand(14, 19), rot: rand(-0.2, 0.2), vr: 0.3, gravity: -4 };
          break;
        case "sweat":
          p = { x: x + rand(-24, 24), y: y + rand(-30, -10), vx: rand(-8, 8), vy: rand(10, 30), life: 1, decay: 1.2, text, size: rand(11, 15), rot: 0, vr: 0, gravity: 60 };
          break;
        case "bell":
          p = { x: x + rand(-8, 8), y: y - rand(38, 50), vx: 0, vy: -6, life: 1, decay: 0.8, text, size: 20, rot: 0, vr: rand(-3, 3), gravity: 0 };
          break;
        case "crumbs":
          p = { x: x + rand(-16, 16), y: y + rand(-8, 4), vx: rand(-26, 26), vy: rand(-40, -12), life: 1, decay: 1.1, text, size: rand(8, 12), rot: rand(0, 6), vr: rand(-4, 4), gravity: 140 };
          break;
        case "confetti":
          p = { x: x + rand(-60, 60), y: y + rand(-90, -50), vx: rand(-40, 40), vy: rand(-30, 20), life: 1, decay: rand(0.35, 0.6), text, size: rand(12, 18), rot: rand(0, 6), vr: rand(-4, 4), gravity: 60 };
          break;
        default: // sparkle
          p = { x: x + rand(-34, 34), y: y + rand(-50, -6), vx: rand(-10, 10), vy: rand(-20, -6), life: 1, decay: 0.9, text, size: rand(10, 16), rot: rand(0, 6), vr: rand(-2, 2), gravity: 0 };
      }
      this.list.push(p);
    }
  }

  update(dt: number): void {
    for (const p of this.list) {
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.vy += p.gravity * dt;
      p.rot += p.vr * dt;
      p.life -= p.decay * dt;
    }
    this.list = this.list.filter((p) => p.life > 0);
  }

  draw(ctx: CanvasRenderingContext2D): void {
    for (const p of this.list) {
      ctx.save();
      ctx.globalAlpha = Math.min(1, p.life * 1.6);
      ctx.translate(p.x, p.y);
      ctx.rotate(p.rot);
      ctx.font = `${p.size}px "Segoe UI Emoji", "Apple Color Emoji", sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(p.text, 0, 0);
      ctx.restore();
    }
  }

  clear(): void {
    this.list = [];
  }

  get count(): number {
    return this.list.length;
  }
}
