/**
 * 今日页(§2.2):任务卡(到期复习 n + 新学 m)· 火苗 · 开始练习。
 * 无红点、无促销 — 心流优先(§1.4)。
 */

import { useEffect, useState } from "react";
import { Button, StatCard, levelOptionLabel } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { TodayOverview } from "../ipc";
import { ipc } from "../ipc";
import type { PracticeLaunch } from "./Practice";

export function TodayPage({ onStart }: { onStart: (launch: PracticeLaunch) => void }) {
  const { level, specs, setLevel } = useApp();
  const [overview, setOverview] = useState<TodayOverview | null>(null);

  useEffect(() => {
    void ipc.todayOverview(level).then(setOverview);
  }, [level]);

  return (
    <div className="page page--today">
      <header className="page__header">
        <h1>今日</h1>
        <select
          className="level-select"
          value={level}
          onChange={(e) => void setLevel(e.target.value as typeof level)}
          aria-label="当前等级"
        >
          {specs.map((s) => (
            <option key={s.id} value={s.id}>
              {levelOptionLabel(s.id, s)}
            </option>
          ))}
        </select>
      </header>

      {overview && (
        <>
          <div className="today-grid">
            <StatCard value={String(overview.due_count)} label="到期复习" />
            <StatCard value={String(Math.min(overview.new_available, 50))} label="可学新句" />
            <StatCard value={`${overview.streak_days} 天`} label="连续打卡 🔥" />
            <StatCard value={String(overview.practiced_today)} label="今日已练" />
          </div>
          <div className="today-action">
            <Button
              onClick={() => onStart({ kind: "daily", level })}
              disabled={overview.due_count === 0 && overview.new_available === 0}
            >
              {overview.practiced_today > 0 ? "继续练习 →" : "开始今日练习 →"}
            </Button>
            {overview.due_count === 0 && overview.new_available === 0 && (
              <p className="today-empty">
                今天没有待练句子 —— 可切换等级、去内容库自由练,或在生成工坊生成新句集。
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}
