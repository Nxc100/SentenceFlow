// 把 wasm-pack 产物复制进试用站 public/,使其成为可静态托管的运行时资源。
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const src = "crates/sf-wasm/pkg";
const dest = "apps/web-trial/public/wasm";

if (!existsSync(src)) {
  console.error(`✕ ${src} 不存在 — 先运行 wasm-pack build`);
  process.exit(1);
}
mkdirSync(dest, { recursive: true });
for (const f of ["sf_wasm.js", "sf_wasm_bg.wasm"]) {
  copyFileSync(join(src, f), join(dest, f));
  console.log(`✓ ${join(dest, f)}`);
}
