// 主题对比度体检 — 读 tokens.css 里的真值,逐主题算 WCAG 对比度并断言下限。
//
// 为什么要有这个脚本:规范 §5.2 要求"全部组合对比度 ≥4.5:1",但色值散在
// 四套主题里,人眼审不出来;新增主题时更容易只顾好看、把某一对压到线下。
// 阈值不是凭空定的 —— 取**出厂浅色主题实测值**作基线(见每行末尾注释),
// 新主题只许追平或超过,不许倒退。
//
// 运行: node scripts/check-theme-contrast.mjs   (失败时 exit 1,可挂 CI)

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const TOKENS = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "tokens.css");

/* ---------- 解析 tokens.css ---------- */

/** 取某个选择器的第一段声明块(tokens.css 里每个选择器只出现一次)。 */
function block(css, selector) {
  const at = css.indexOf(selector + " {");
  if (at < 0) throw new Error(`tokens.css 里找不到选择器 ${selector}`);
  const start = css.indexOf("{", at) + 1;
  const end = css.indexOf("\n}", start);
  return css.slice(start, end);
}

/** 声明块 → { "--sf-x": "值" }(注释与空行天然被正则跳过)。 */
function decls(text) {
  const out = {};
  for (const m of text.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

const css = readFileSync(TOKENS, "utf8");
const root = decls(block(css, ":root"));
const THEMES = {
  light: root,
  dark: { ...root, ...decls(block(css, '[data-theme="dark"]')) },
  paper: { ...root, ...decls(block(css, '[data-theme="paper"]')) },
  macaron: { ...root, ...decls(block(css, '[data-theme="macaron"]')) },
};

/* ---------- 颜色 ---------- */

const clamp255 = (v) => Math.max(0, Math.min(255, Math.round(v)));
const parseHex = (h) => {
  const s = h.slice(1);
  const full = s.length === 3 ? [...s].map((c) => c + c).join("") : s;
  const n = parseInt(full, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
};
const parseRgba = (v) => {
  const p = v.slice(v.indexOf("(") + 1, v.lastIndexOf(")")).split(",").map((x) => parseFloat(x));
  return [clamp255(p[0]), clamp255(p[1]), clamp255(p[2]), p.length > 3 ? p[3] : 1];
};

/**
 * 令牌 → [r,g,b,a]。跟随 var() 间接引用;
 * 渐变/none 这类非纯色由调用方给 approx 兜底(见 SOLID_APPROX)。
 */
function color(theme, token, approx = {}) {
  let v = approx[token] ?? THEMES[theme][token];
  if (v === undefined) throw new Error(`${theme} 主题缺令牌 ${token}`);
  let guard = 0;
  while (v.startsWith("var(")) {
    const ref = v.slice(4, v.indexOf(")")).trim();
    v = approx[ref] ?? THEMES[theme][ref];
    if (v === undefined) throw new Error(`${theme} 主题缺令牌 ${ref}`);
    if (++guard > 8) throw new Error(`${token} 的 var() 引用成环`);
  }
  if (v.startsWith("#")) return parseHex(v);
  if (v.startsWith("rgba(") || v.startsWith("rgb(")) return parseRgba(v);
  throw new Error(`${theme}.${token} = "${v}" 不是纯色,请在 SOLID_APPROX 里给最坏情况近似值`);
}

/** 前景带透明度时,先合成到底色上再算亮度。 */
const flatten = ([r, g, b, a], bg) =>
  a >= 1 ? [r, g, b, 1] : [r * a + bg[0] * (1 - a), g * a + bg[1] * (1 - a), b * a + bg[2] * (1 - a), 1];

const luminance = ([r, g, b]) => {
  const [R, G, B] = [r, g, b].map((v) => v / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * R + 0.7152 * G + 0.0722 * B;
};
const contrast = (fg, bg) => {
  const l1 = luminance(flatten(fg, bg));
  const l2 = luminance(bg);
  const [hi, lo] = l1 > l2 ? [l1, l2] : [l2, l1];
  return (hi + 0.05) / (lo + 0.05);
};

/* ---------- 非纯色令牌的最坏情况近似 ---------- */

/**
 * 马卡龙主题的卡面是"白色渐变玻璃"。断言取渐变里**最透明的一端**
 * (0.62)——真实卡面只会比它更实、对比只会更好。
 */
const SOLID_APPROX = {
  macaron: { "--sf-card": "rgba(255, 255, 255, 0.62)" },
};

/**
 * 页面壁纸最饱和处(草莓光斑压在 --sf-bg 上,再留一档余量给光斑重叠区)。
 * 卡片浮在这上面时透出的粉最重,是文字对比的最坏位置。
 */
const WORST_BACKDROP = { macaron: [255, 199, 220, 1] };

/* ---------- 待检查的组合 ---------- */

const white = [255, 255, 255, 1];

/** 教学色板:词性胶囊 14 组 + 成分卡 8 组,每组"字压自己的底"。 */
const POS = "pron n v aux modal adj wh adv prep art conj num propn part".split(" ");
const ROLE = "subj pred link obj comp advl objc marker".split(" ");

/**
 * 阈值来源:出厂浅色主题的实测值(注释里的数字),向下取整留一点余量。
 * 想调阈值先问"浅色主题当年是怎么过的",别为了让新主题过线而放水。
 */
const CHECKS = [
  { label: "正文 / 卡面", fg: "--sf-text", on: "card", min: 10.0 }, // 浅色 15.18
  { label: "正文 / 页面底", fg: "--sf-text", on: "page", min: 10.0 }, // 浅色 14.16
  { label: "次级正文 / 卡面", fg: "--sf-text-2", on: "card", min: 6.0 }, // 浅色 6.64
  { label: "三级文字 / 卡面", fg: "--sf-text-3", on: "card", min: 3.7 }, // 浅色 3.77
  { label: "三级文字 / 页面底", fg: "--sf-text-3", on: "page", min: 3.5 }, // 浅色 3.52
  { label: "占位文字 / 卡面", fg: "--sf-text-placeholder", on: "card", min: 2.2 }, // 浅色 2.26
  { label: "主色 / 卡面", fg: "--sf-primary", on: "card", min: 4.5 }, // 浅色 5.18
  { label: "白字 / 主色(主按钮)", fg: white, on: "--sf-primary", min: 4.5 }, // 浅色 5.18
  { label: "主色 / 主色浅底", fg: "--sf-primary", on: "--sf-primary-soft", min: 4.0 }, // 浅色 4.66
  { label: "错误色 / 卡面", fg: "--sf-error", on: "card", min: 4.5 }, // 浅色 4.83
  { label: "格线 / 卡面(非文字)", fg: "--sf-slot-underline", on: "card", min: 1.4 }, // 浅色 1.58
  ...POS.map((k) => ({ label: `词性 ${k} 字/底`, fg: `--sf-pos-${k}-text`, on: `--sf-pos-${k}-bg`, min: 3.9 })),
  ...ROLE.map((k) => ({ label: `成分 ${k} 字/底`, fg: `--sf-role-${k}-text`, on: `--sf-role-${k}-bg`, min: 3.9 })),
];

/* ---------- 出厂色板的历史欠账 ---------- */

/**
 * 规范 §5.2 写的是"全部组合 ≥4.5:1",但出厂色板实测有几对没做到 ——
 * 这些是 v5 规范自带的色值,改动等于改教学色到语法的映射,属于产品决策,
 * 不在"加一套主题"的范围内。做法:登记在册 + 以实测值为新地板,
 * **允许维持、不允许变差**;新主题不得再添一笔。
 * 键 = `主题/项目`,值 = 登记当日实测比值(2026-08-21)。
 */
const DEBT = {
  // 教学色板本身(浅色/纸色/马卡龙共用同一组值,故三处同账)
  ...Object.fromEntries(
    ["light", "paper", "macaron"].flatMap((t) => [
      [`${t}/词性 adv 字/底`, 3.77],
      [`${t}/词性 prep 字/底`, 3.38],
      [`${t}/词性 num 字/底`, 3.44],
      [`${t}/成分 advl 字/底`, 3.6],
      [`${t}/成分 objc 字/底`, 3.76],
    ]),
  ),
  // 深色主题:主色是 blue-400,压白字偏亮;错误红没有为深底提亮
  "dark/白字 / 主色(主按钮)": 3.59,
  "dark/主色 / 主色浅底": 3.67,
  "dark/错误色 / 卡面": 3.37,
  // 纸色主题:底色偏暖偏深,灰阶文字跟着损失一点
  "paper/三级文字 / 卡面": 3.61,
  "paper/三级文字 / 页面底": 3.38,
  "paper/占位文字 / 卡面": 2.17,
};

/* ---------- 跑 ---------- */

let failed = 0;
let debts = 0;
const width = 26;

for (const theme of Object.keys(THEMES)) {
  const approx = SOLID_APPROX[theme] ?? {};
  const page = color(theme, "--sf-bg", approx);
  const card = flatten(color(theme, "--sf-card", approx), page);
  const backdrops = { page, card };

  console.log(`\n主题 ${theme}`);
  for (const c of CHECKS) {
    const fg = Array.isArray(c.fg) ? c.fg : color(theme, c.fg, approx);
    // 底色本身可能带透明度(如主色浅底),先压到卡面上
    const bg = backdrops[c.on] ?? flatten(color(theme, c.on, approx), card);
    const r = contrast(fg, bg);
    const debt = DEBT[`${theme}/${c.label}`];
    if (r >= c.min - 0.005) {
      console.log(`  ✓ ${c.label.padEnd(width)} ${r.toFixed(2).padStart(6)}  下限 ${c.min.toFixed(2)}`);
    } else if (debt !== undefined && r >= debt - 0.005) {
      debts++;
      console.log(`  ⚠ ${c.label.padEnd(width)} ${r.toFixed(2).padStart(6)}  下限 ${c.min.toFixed(2)}(历史欠账,已登记)`);
    } else {
      failed++;
      const why = debt !== undefined ? `低于登记值 ${debt.toFixed(2)}` : `下限 ${c.min.toFixed(2)}`;
      console.log(`  ✗ ${c.label.padEnd(width)} ${r.toFixed(2).padStart(6)}  ${why}`);
    }
  }

  // 壁纸最重处:卡片是半透明的,文字实际压在"卡 + 光斑"上
  const worst = WORST_BACKDROP[theme];
  if (worst) {
    const cardOnBlob = flatten(color(theme, "--sf-card", approx), worst);
    for (const [label, tok, min] of [
      ["正文 / 卡面压光斑", "--sf-text", 10.0],
      ["三级文字 / 卡面压光斑", "--sf-text-3", 3.5],
      ["主色 / 卡面压光斑", "--sf-primary", 4.5],
    ]) {
      const r = contrast(color(theme, tok, approx), cardOnBlob);
      const ok = r >= min - 0.005;
      if (!ok) failed++;
      console.log(`  ${ok ? "✓" : "✗"} ${label.padEnd(width)} ${r.toFixed(2).padStart(6)}  下限 ${min.toFixed(2)}`);
    }
  }
}

/* ---------- §5.1 第 2 条:装饰色不得挪用教学色板 ---------- */

const TEACHING = new Set();
for (const k of POS) for (const s of ["text", "bg"]) TEACHING.add(`--sf-pos-${k}-${s}`);
for (const k of ROLE) for (const s of ["text", "bg", "border"]) TEACHING.add(`--sf-role-${k}-${s}`);

console.log("\n教学色板独占性(§5.1 第 2 条:粉彩色板专属词性/成分,禁止挪用)");
for (const theme of Object.keys(THEMES)) {
  const t = THEMES[theme];
  const teachingValues = new Map();
  for (const k of TEACHING) if (t[k]?.startsWith("#")) teachingValues.set(t[k].toUpperCase(), k);
  const clashes = [];
  for (const [k, v] of Object.entries(t)) {
    if (TEACHING.has(k) || !v.startsWith("#")) continue;
    const hit = teachingValues.get(v.toUpperCase());
    if (hit) clashes.push(`${k} 与 ${hit} 同值 ${v}`);
  }
  if (clashes.length) {
    failed += clashes.length;
    console.log(`  ✗ ${theme}: ${clashes.join("; ")}`);
  } else {
    console.log(`  ✓ ${theme}: 装饰/语义色与教学色板零重合`);
  }
}

const tail = debts ? `(另有 ${debts} 项历史欠账,维持登记值未变差)` : "";
console.log(failed ? `\n${failed} 项未达标 ${tail}` : `\n全部达标 ${tail}`);
process.exit(failed ? 1 : 0);
