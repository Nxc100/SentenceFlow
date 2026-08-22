/**
 * 撒花 — 签名时刻专属(§6.2:答对整句是全产品唯一允许铺张的动效;
 * 其他场景一律禁用,§5.1 第 5 条)。
 *
 * 与规范有两处**有意偏差**,都是"用户说看不见"逼出来的:
 *
 * ① 颜色:规范写"取语法色板底色系",但那是 #FFE3EE 一类近白粉彩,压在
 *    浅色 / 纸色 / 马卡龙底上对比度只有 ~1.1:1 —— 等于没放。改为运行时读
 *    **当前主题**的语法色板*文字色*(`--sf-pos-*-text` / `--sf-role-*-text`):
 *    仍是同一套教学色板,而且每套主题都为这些色调过对比度(深色主题下它们
 *    本就是亮色),于是四套主题下都看得见,不必为每套主题各写一份色表。
 *
 * ② 规模与构图:60–90 粒 → 按视口面积 120–200 粒;发射点从"画面中部两点"
 *    改成**中心一炸 + 底部两侧礼炮**。中心那炸是关键 —— 签名时刻 720ms 后
 *    就允许按空格进下一题,大量用户只看得到前 300ms,礼炮从底部飞上来还在
 *    路上,必须有一炸当场铺满视野。时长 1.2s → 1.8s(末 0.5s 淡出)。
 *
 * 仍然可被任意按键立即取消(§5.1 第 4 条,由调用方 ParseView 负责触发);
 * 减少动效下调用方直接不放(§6.1)。
 */

import { CONFETTI_COLORS } from "./grammar";

export interface ConfettiHandle {
  cancel: () => void;
}

type Shape = "ribbon" | "dot" | "streamer";

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  size: number;
  /** 自转 */
  rot: number;
  vr: number;
  /** 纸片翻面(绕长轴翻转)的相位 */
  flutter: number;
  vf: number;
  color: string;
  shape: Shape;
}

const DURATION_MS = 1800;
const FADE_MS = 500;
const GRAVITY = 1500;

/** 参与撒花的教学色组;灰色系(part / marker)不进 —— 撒花要彩色 */
const POS_KEYS = [
  "pron", "n", "v", "aux", "modal", "adj", "wh",
  "adv", "prep", "art", "conj", "num", "propn",
];
const ROLE_KEYS = ["subj", "pred", "link", "obj", "comp", "advl", "objc"];

/** 读当前主题的语法色板文字色;拿不到(SSR/测试)就退回静态表 */
function themeColors(): string[] {
  if (typeof window === "undefined" || typeof document === "undefined") return CONFETTI_COLORS;
  const cs = getComputedStyle(document.documentElement);
  const colors = [
    ...POS_KEYS.map((k) => cs.getPropertyValue(`--sf-pos-${k}-text`).trim()),
    ...ROLE_KEYS.map((k) => cs.getPropertyValue(`--sf-role-${k}-text`).trim()),
  ].filter(Boolean);
  return colors.length >= 6 ? colors : CONFETTI_COLORS;
}

/** 从一点按扇形角区间喷一簇(角度用 canvas 坐标系:−π/2 为正上方) */
function emit(
  out: Particle[],
  colors: string[],
  n: number,
  x: number,
  y: number,
  angleFrom: number,
  angleTo: number,
  speedFrom: number,
  speedTo: number,
) {
  for (let i = 0; i < n; i++) {
    const angle = angleFrom + Math.random() * (angleTo - angleFrom);
    const speed = speedFrom + Math.random() * (speedTo - speedFrom);
    const roll = Math.random();
    out.push({
      x,
      y,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed,
      size: 9 + Math.random() * 9,
      rot: Math.random() * Math.PI * 2,
      vr: (Math.random() - 0.5) * 14,
      flutter: Math.random() * Math.PI * 2,
      vf: 6 + Math.random() * 6,
      color: colors[(Math.random() * colors.length) | 0]!,
      shape: roll < 0.6 ? "ribbon" : roll < 0.85 ? "dot" : "streamer",
    });
  }
}

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

  const colors = themeColors();
  // 粒子数随视口面积走:小窗口不糊屏,大窗口不显稀
  const total = Math.round(Math.max(120, Math.min(200, 100 + (w * h) / 9000)));
  const particles: Particle[] = [];
  // 中心一炸(成分卡所在高度):保证第一帧就有画面。初速给足,
  // 纸片才会尽快让开句子本身 —— 庆祝归庆祝,别糊住刚答对的那句话。
  emit(particles, colors, Math.round(total * 0.4), w * 0.5, h * 0.44, 0, Math.PI * 2, 320, 780);
  // 底部两侧礼炮:斜向对射,把整屏兜住
  emit(particles, colors, Math.round(total * 0.3), w * 0.06, h * 1.02, -1.4, -0.6, 1200, 1700);
  emit(particles, colors, Math.round(total * 0.3), w * 0.94, h * 1.02, -2.54, -1.74, 1200, 1700);

  let raf = 0;
  let cancelled = false;
  const start = performance.now();
  let last = start;

  const frame = (now: number) => {
    if (cancelled) return;
    const t = now - start;
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now;
    ctx.clearRect(0, 0, w, h);
    if (t >= DURATION_MS) return;

    const fade = t > DURATION_MS - FADE_MS ? (DURATION_MS - t) / FADE_MS : 1;
    // 空气阻力:横向衰减快,纵向慢 —— 纸片才会"飘"下来而不是抛物线砸下来
    const dragX = Math.exp(-1.5 * dt);
    const dragY = Math.exp(-0.55 * dt);

    for (const p of particles) {
      p.vx *= dragX;
      p.vy = (p.vy + GRAVITY * dt) * dragY;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.rot += p.vr * dt;
      p.flutter += p.vf * dt;
      if (p.y - p.size > h) continue; // 落出画面就不画了

      ctx.save();
      ctx.globalAlpha = fade;
      ctx.translate(p.x, p.y);
      ctx.rotate(p.rot);
      ctx.fillStyle = p.color;
      if (p.shape === "dot") {
        ctx.beginPath();
        ctx.arc(0, 0, p.size * 0.34, 0, Math.PI * 2);
        ctx.fill();
      } else {
        // 绕长轴翻面:压扁高度,纸片感的来源
        ctx.scale(1, 0.2 + 0.8 * Math.abs(Math.cos(p.flutter)));
        const pw = p.shape === "streamer" ? p.size * 1.7 : p.size;
        const ph = p.shape === "streamer" ? p.size * 0.3 : p.size * 0.5;
        ctx.fillRect(-pw / 2, -ph / 2, pw, ph);
      }
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
