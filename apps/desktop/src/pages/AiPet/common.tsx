/** 「AI 萌宠」页内共用的小部件:落区、校验报告、警告行、进度条。 */

import { useRef } from "react";
import type { ReactNode, RefObject } from "react";
import type { Report } from "../../pet/types";
import { useDropZone } from "./useDropZone";

/** 拖放落区。点击走 `onPick`(rfd 文件选择),拖入走同一个 `onFiles`。 */
export function DropZone({
  icon,
  title,
  hint,
  small,
  busy,
  onFiles,
  onPick,
}: {
  icon: string;
  title: string;
  hint?: string;
  small?: boolean;
  busy?: boolean;
  onFiles: (paths: string[]) => void;
  onPick: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useDropZone(ref as RefObject<HTMLElement | null>, onFiles, !busy);
  return (
    <div
      ref={ref}
      className={`aipet-drop${small ? " aipet-drop--sm" : ""}${busy ? " aipet-drop--busy" : ""}`}
      role="button"
      tabIndex={0}
      onClick={() => !busy && onPick()}
      onKeyDown={(e) => {
        if (!busy && (e.key === "Enter" || e.key === " ")) onPick();
      }}
    >
      <div className="aipet-drop__icon" aria-hidden>
        {icon}
      </div>
      <div className="aipet-drop__title">{title}</div>
      {hint && <div className="aipet-drop__hint">{hint}</div>}
    </div>
  );
}

/** 校验报告:全通过给一条绿色确认,否则逐条列出。 */
export function ReportView({ report }: { report: Report | null }) {
  if (!report) return null;
  if (report.findings.length === 0) {
    return <div className="aipet-note aipet-note--ok">✅ 全部校验通过</div>;
  }
  return (
    <div className="aipet-notes">
      {report.findings.map((f, i) => (
        <div
          key={`${f.code}-${i}`}
          className={`aipet-note aipet-note--${f.level === "error" ? "err" : "warn"}`}
        >
          {f.level === "error" ? "❌" : "⚠"} {f.message}
        </div>
      ))}
    </div>
  );
}

export function Warnings({ items }: { items: string[] }) {
  if (items.length === 0) return null;
  return (
    <div className="aipet-notes">
      {items.map((w, i) => (
        <div key={i} className="aipet-note aipet-note--warn">
          ⚠ {w}
        </div>
      ))}
    </div>
  );
}

/** 细进度条(视频抠像 / 出生视频编码)。 */
export function Progress({ pct, label }: { pct: number; label: string }) {
  return (
    <div className="aipet-progress">
      <div className="aipet-progress__track">
        <div className="aipet-progress__fill" style={{ width: `${Math.round(pct)}%` }} />
      </div>
      <span className="aipet-progress__label">{label}</span>
    </div>
  );
}

/** 分区小标题:图标徽章 + 标题 + 可选说明。 */
export function SectionHead({
  icon,
  title,
  desc,
  right,
}: {
  icon: string;
  title: string;
  desc?: string;
  right?: ReactNode;
}) {
  return (
    <div className="aipet-head">
      <span className="aipet-head__ico" aria-hidden>
        {icon}
      </span>
      <div className="aipet-head__text">
        <h3>{title}</h3>
        {desc && <p>{desc}</p>}
      </div>
      {right && <div className="aipet-head__right">{right}</div>}
    </div>
  );
}

/** 背景判定的人话。 */
export function bgName(bg: string): string {
  return (
    ({ chroma: "绿幕", transparent: "透明", solid: "纯色", busy: "复杂（智能抠图）" } as const)[
      bg as "chroma" | "transparent" | "solid" | "busy"
    ] ?? bg
  );
}

export const IMG_EXTS = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
export const VIDEO_EXTS = ["mp4", "mov", "webm"];

/**
 * 复制文本到剪贴板,失败时用一次性 textarea 兜底。
 *
 * 「复制咒语」是这个模块最要紧的一个动作 —— WebView 的
 * `navigator.clipboard` 在个别环境里会直接抛(权限/非安全上下文),
 * 不能让用户卡在这里,得留一条老路。
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.cssText = "position:fixed;top:-1000px;opacity:0";
      document.body.append(ta);
      ta.select();
      const ok = document.execCommand("copy");
      ta.remove();
      return ok;
    } catch {
      return false;
    }
  }
}
