import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

const page = (name: string) => fileURLToPath(new URL(`${name}.html`, import.meta.url));

// devUrl 5174 与 src-tauri/tauri.conf.json 对齐。
// 双入口:index = 主窗(React),pet = AI 萌宠的透明桌宠窗(零框架 Canvas)。
export default defineConfig({
  plugins: [react()],
  server: { port: 5174, strictPort: true },
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 1024,
    rollupOptions: {
      input: { main: page("index"), pet: page("pet") },
    },
  },
  clearScreen: false,
});
