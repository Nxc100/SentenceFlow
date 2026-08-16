// 深色模式语法色派生 (spec §5.2 深色模式):
//   文字 = 亮色文字提亮(HSL: L→0.72, S×1.6 上限 0.92 — 由文档给出的
//          锚点 #B3536E → #E58BA4 反推出的规则)
//   底色 = 提亮后文字色 14% 透明
//   描边 = 提亮后文字色 24% 透明
// 运行: node scripts/derive-dark-colors.mjs  → 输出 tokens.css 深色段落。

const POS = {
  pron: "#C2255C", n: "#C43A2B", v: "#B02890", aux: "#6B3FD1",
  modal: "#5F3DC4", adj: "#8A3FC7", wh: "#2B5BD7", adv: "#1F8A4C",
  prep: "#C46A0C", art: "#0B7F73", conj: "#0B6E99", num: "#A67C00",
  propn: "#9C6E00", part: "#5A6472",
};
const ROLE = {
  subj: "#B3536E", pred: "#6D51B8", link: "#5B62A8", obj: "#3D6BB3",
  comp: "#4A5AB8", advl: "#9A7A17", objc: "#B0642B", marker: "#5A6472",
};

function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function rgbToHsl([r, g, b]) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  switch (max) {
    case r: h = (g - b) / d + (g < b ? 6 : 0); break;
    case g: h = (b - r) / d + 2; break;
    default: h = (r - g) / d + 4;
  }
  return [h / 6, s, l];
}
function hslToRgb([h, s, l]) {
  if (s === 0) { const v = Math.round(l * 255); return [v, v, v]; }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const f = (t) => {
    if (t < 0) t += 1; if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  return [f(h + 1 / 3), f(h), f(h - 1 / 3)].map((v) => Math.round(v * 255));
}
function lighten(hex) {
  const [h, s] = rgbToHsl(hexToRgb(hex));
  return hslToRgb([h, Math.min(s * 1.6, 0.92), 0.72]);
}
const hex = (rgb) => "#" + rgb.map((v) => v.toString(16).padStart(2, "0").toUpperCase()).join("");

for (const [group, map] of [["pos", POS], ["role", ROLE]]) {
  for (const [k, v] of Object.entries(map)) {
    const L = lighten(v);
    const [r, g, b] = L;
    console.log(`  --sf-${group}-${k}-text: ${hex(L)};`);
    console.log(`  --sf-${group}-${k}-bg: rgba(${r}, ${g}, ${b}, 0.14);`);
    if (group === "role") console.log(`  --sf-${group}-${k}-border: rgba(${r}, ${g}, ${b}, 0.24);`);
  }
}
