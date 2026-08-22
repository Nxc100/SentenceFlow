/**
 * 今日页(§2.2):任务卡(到期复习 n + 新学 m)· 火苗 · 开始练习。
 * 无红点、无促销 — 心流优先(§1.4)。
 *
 * 空态分两种,措辞与出路完全不同(实测发现的坑:定级把用户推荐到「高级」,
 * 而出厂内容目前只到「中级」,新用户一进来四张卡全 0 —— 此时说"今天没有待练
 * 句子"是答非所问,他要的是"这一级还没有句子,先去哪儿")。
 */

import { useEffect, useState } from "react";
import { Button, StatCard, levelName, levelOptionLabel } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { NavKey } from "../App";
import type { TodayOverview } from "../ipc";
import { ipc } from "../ipc";
import type { PracticeLaunch } from "./Practice";

export function TodayPage({
  onStart,
  onNav,
}: {
  onStart: (launch: PracticeLaunch) => void;
  onNav: (key: NavKey) => void;
}) {
  const { level, specs, setLevel, sentenceCountFor, topLevelWithContent } = useApp();
  const [overview, setOverview] = useState<TodayOverview | null>(null);

  useEffect(() => {
    void ipc.todayOverview(level).then(setOverview);
  }, [level]);

  const nothingToday = overview !== null && overview.due_count === 0 && overview.new_available === 0;
  const levelEmpty = sentenceCountFor(level) === 0;
  const fallback = topLevelWithContent();

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
              {sentenceCountFor(s.id) === 0 ? "(暂无句子)" : ""}
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
            <Button onClick={() => onStart({ kind: "daily", level })} disabled={nothingToday}>
              {overview.practiced_today > 0 ? "继续练习 →" : "开始今日练习 →"}
            </Button>

            {nothingToday && levelEmpty && (
              <div className="today-empty">
                <p>
                  「{levelName(level)}」的出厂句库还在补齐中 —— 目前出厂内容到「
                  {levelName(fallback)}」。
                </p>
                <div className="today-empty__acts">
                  {fallback !== level && (
                    <Button variant="secondary" onClick={() => void setLevel(fallback)}>
                      切到「{levelName(fallback)}」开始练
                    </Button>
                  )}
                  <Button variant="ghost" onClick={() => onNav("workshop")}>
                    用 AI 为这一级造句
                  </Button>
                </div>
              </div>
            )}

            {nothingToday && !levelEmpty && (
              <div className="today-empty">
                <p>今天的都练完了 👏</p>
                <div className="today-empty__acts">
                  <Button variant="secondary" onClick={() => onNav("library")}>
                    去我的句库自由练
                  </Button>
                  <Button variant="ghost" onClick={() => onNav("workshop")}>
                    用 AI 加练一组
                  </Button>
                </div>
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
