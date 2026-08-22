import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ToastProvider } from "@sentenceflow/ui";
import { App } from "./App";
import "./app.css";
// 主题层在布局之后加载:靠顺序 + 特异性覆盖外壳默认表现
import "./theme-macaron.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ToastProvider>
      <App />
    </ToastProvider>
  </StrictMode>,
);
