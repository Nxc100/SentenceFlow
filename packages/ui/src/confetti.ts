/**
 * 撒花 — 签名时刻专属(§6.2:60–90 粒,1.2s,取语法色板底色系)。
 * 其他一切场景禁用(§5.1 第 5 条)。
 */

import { CONFETTI_COLORS } from "./grammar";

export interface ConfettiHandle {
  cancel: () => void;
}

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  size: number;
  rotation: number;
  vr: number;
  color: string;
}

const DURATION_MS = 1200;

/** 在给定 canvas 上播放一次撒花;返回可取消句柄(按键跳终态时调用)。 */
export function playConfetti(canvas: HTMLCanvasElement): ConfettiHandle {
  const ctx = canvas.getContext("2d");
  if (!ctx) return { cancel: () => {} };

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || window.innerWidth;
  const h = canvas.clientHeight || window.innerHeight;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  ctx.scale(dpr, dpr);

  const count = 60 + Math.floor(Math.random() * 31); // 60–90 粒
  const particles: Particle[] = [];
  for (let i = 0; i < count; i++) {
    const fromLeft = i % 2 === 0;
    particles.push({
      x: fromLeft ? w * 0.25 : w * 0.75,
      y: h * 0.45,
      vx: (fromLeft ? 1 : -1) * (60 + Math.random() * 240) * (Math.random() > 0.5 ? 1 : 0.4),
      vy: -(260 + Math.random() * 300),
      size: 6 + Math.random() * 6,
      rotation: Math.random() * Math.PI,
      vr: (Math.random() - 0.5) * 12,
      color: CONFETTI_COLORS[Math.floor(Math.random() * CONFETTI_COLORS.length)]!,
    });
  }

  let raf = 0;
  let cancelled = false;
  const start = performance.now();
  let last = start;
  const gravity = 900;

  const frame = (now: number) => {
    if (cancelled) return;
    const t = now - start;
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now;
    ctx.clearRect(0, 0, w, h);
    if (t >= DURATION_MS) return;
    const fade = t > DURATION_MS - 300 ? (DURATION_MS - t) / 300 : 1;
    for (const p of particles) {
      p.vy += gravity * dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.rotation += p.vr * dt;
      ctx.save();
      ctx.globalAlpha = fade;
      ctx.translate(p.x, p.y);
      ctx.rotate(p.rotation);
      ctx.fillStyle = p.color;
      ctx.fillRect(-p.size / 2, -p.size / 4, p.size, p.size / 2);
      ctx.restore();
    }
    raf = requestAnimationFrame(frame);
  };
  raf = requestAnimationFrame(frame);

  return {
    cancel() {
      cancelled = true;
      cancelAnimationFrame(raf);
      ctx.clearRect(0, 0, w, h);
    },
  };
}
