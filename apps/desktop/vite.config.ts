import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// devUrl 5174 与 src-tauri/tauri.conf.json 对齐
export default defineConfig({
  plugins: [react()],
  server: { port: 5174, strictPort: true },
  build: { target: "es2022", chunkSizeWarningLimit: 1024 },
  clearScreen: false,
});
