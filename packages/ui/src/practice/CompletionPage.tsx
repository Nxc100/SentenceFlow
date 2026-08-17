/**
 * CompletionPage — 完成页统计卡(§5.5 核心组件 11)。
 * 无红点无促销;CTA 由宿主注入(试用版页脚 CTA / 桌面端安静卡片,§6.3)。
 */

import type { ReactNode } from "react";
import { StatCard } from "../components/StatCard";

export interface CompletionStats {
  sentences: number;
  accuracy: number; // 0..1
  avgWpm: number;
  durMs: number;
}

export interface CompletionPageProps {
  title?: string;
  stats: CompletionStats;
  /** 宿主注入的行动区(继续/下一节/购买 CTA…) */
  actions?: ReactNode;
  children?: ReactNode;
}

function formatDuration(ms: number): string {
  const totalSec = Math.round(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return m > 0 ? `${m} 分 ${s} 秒` : `${s} 秒`;
}

export function CompletionPage({ title = "本节完成", stats, actions, children }: CompletionPageProps) {
  return (
    <div className="sf-completion">
      <h2 className="sf-completion__title">{title}</h2>
      <div className="sf-completion__grid">
        <StatCard value={String(stats.sentences)} label="句子" />
        <StatCard value={`${Math.round(stats.accuracy * 100)}%`} label="正确率" />
        <StatCard value={stats.avgWpm > 0 ? stats.avgWpm.toFixed(0) : "–"} label="速度(词/分)" />
        <StatCard value={formatDuration(stats.durMs)} label="用时" />
      </div>
      {children}
      {actions && <div className="sf-completion__actions">{actions}</div>}
    </div>
  );
}
