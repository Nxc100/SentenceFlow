/**
 * 报告页(§4.5):今日卡 · 热力日历 · WPM/正确率曲线 · 薄弱分析([AI 加练])
 * · 周报导出(打印为 PDF — 系统打印对话框选“另存为 PDF”,A4 一页)。
 */

import { useEffect, useMemo, useState } from "react";
import { Button, HeatmapCalendar, POS_ZH, ROLE_ZH, StatCard } from "@sentenceflow/ui";
import type { StatsSummary } from "@sentenceflow/ui";
import { ipc, tzOffsetSecs } from "../ipc";

export function ReportPage({ onDrill }: { onDrill: (scene: string) => void }) {
  const [stats, setStats] = useState<StatsSummary | null>(null);

  useEffect(() => {
    void ipc.getStats().then(setStats);
  }, []);

  const today = useMemo(() => {
    if (!stats) return null;
    const dayIdx = Math.floor((Date.now() / 1000 + tzOffsetSecs()) / 86_400);
    return stats.days[String(dayIdx)] ?? null;
  }, [stats]);

  const curve = useMemo(() => {
    if (!stats) return [];
    return Object.entries(stats.days)
      .map(([day, d]) => ({ day: Number(day), ...d }))
      .sort((a, b) => a.day - b.day)
      .slice(-30);
  }, [stats]);

  const heatCounts = useMemo(() => {
    if (!stats) return {};
    const out: Record<string, number> = {};
    for (const [day, d] of Object.entries(stats.days)) out[day] = d.attempts;
    return out;
  }, [stats]);

  if (!stats) return <div className="page" aria-busy="true" />;

  return (
    <div className="page page--report" id="report-print-root">
      <header className="page__header">
        <h1>报告</h1>
        <Button variant="ghost" onClick={() => window.print()}>
          导出周报 PDF
        </Button>
      </header>

      <div className="report-grid">
        <StatCard value={String(today?.attempts ?? 0)} label="今日句数" />
        <StatCard
          value={today ? `${Math.round(today.accuracy * 100)}%` : "–"}
          label="今日正确率"
        />
        <StatCard
          value={today && today.avg_wpm > 0 ? today.avg_wpm.toFixed(0) : "–"}
          label="打字速度(词/分)"
        />
        <StatCard value={`${stats.streak_days} 天`} label="连续打卡 🔥" />
      </div>

      <section className="report-section">
        <h2>练习热力</h2>
        <HeatmapCalendar
          counts={heatCounts}
          endDay={Math.floor((Date.now() / 1000 + tzOffsetSecs()) / 86_400)}
        />
      </section>

      {curve.length > 1 && (
        <section className="report-section">
          <h2>近 30 天曲线</h2>
          <Sparkline
            label="打字速度"
            values={curve.map((c) => c.avg_wpm)}
            color="var(--sf-blue-500)"
          />
          <Sparkline
            label="正确率"
            values={curve.map((c) => c.accuracy * 100)}
            color="var(--sf-success)"
          />
        </section>
      )}

      <section className="report-section">
        <h2>薄弱分析</h2>
        {stats.weak_pos.length === 0 && stats.weak_roles.length === 0 && (
          <p className="report-empty">还没有足够的错误数据 —— 这是好事。</p>
        )}
        <ul className="report-weak">
          {stats.weak_pos.slice(0, 3).map((w) => (
            <li key={w.tag}>
              <span>
                {POS_ZH[w.tag]}错误率 {Math.round(w.share * 100)}% —— 值得针对性加练。
              </span>
              <Button variant="ghost" onClick={() => onDrill(`${POS_ZH[w.tag]}专项练习`)}>
                AI 加练
              </Button>
            </li>
          ))}
          {stats.weak_roles.slice(0, 2).map((w) => (
            <li key={w.tag}>
              <span>
                {ROLE_ZH[w.tag]}相关错误占 {Math.round(w.share * 100)}%。
              </span>
              <Button variant="ghost" onClick={() => onDrill(`${ROLE_ZH[w.tag]}强化句`)}>
                AI 加练
              </Button>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

function Sparkline({ label, values, color }: { label: string; values: number[]; color: string }) {
  const w = 560;
  const h = 64;
  const max = Math.max(...values, 1);
  const pts = values
    .map((v, i) => `${(i / Math.max(1, values.length - 1)) * w},${h - (v / max) * (h - 8) - 4}`)
    .join(" ");
  return (
    <div className="sparkline">
      <span className="sparkline__label">{label}</span>
      <svg width={w} height={h} role="img" aria-label={`${label} 曲线`}>
        <polyline points={pts} fill="none" stroke={color} strokeWidth="2" strokeLinejoin="round" />
      </svg>
    </div>
  );
}
