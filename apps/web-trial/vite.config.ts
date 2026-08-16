import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: { port: 5173 },
  build: {
    target: "es2022",
    // wasm-pack 产物经动态 import 载入;体积预算见 §7.9(静态站)
    chunkSizeWarningLimit: 1024,
  },
});
