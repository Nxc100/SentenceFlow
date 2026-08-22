/**
 * 我的水平(左侧栏一级入口):当前等级 + 一键进入水平测试 +
 * 六张等级卡手动切换 + 上次测试结果详情。
 * 定级测试本体是全屏流程(PlacementTest.tsx),由 Shell 挂载,这里只发起。
 */

import { useEffect, useState } from "react";
import { Button, levelCanDo, levelName, useToast } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { PlacementResult } from "../ipc";
import { ipc } from "../ipc";

export function MyLevelPage({ onStartTest }: { onStartTest: () => void }) {
  const { specs, level, setLevel, sentenceCountFor, topLevelWithContent } = useApp();
  const toast = useToast();
  const [last, setLast] = useState<PlacementResult | null>(null);

  useEffect(() => {
    void ipc.placementLast().then(setLast);
  }, []);

  const spec = specs.find((s) => s.id === level);

  return (
    <div className="page page--mylevel">
      <header className="page__header">
        <h1>我的水平</h1>
      </header>

      <div className="mylevel-current">
        <div className="mylevel-current__info">
          <p className="mylevel-current__eyebrow">当前练习等级</p>
          <p className="mylevel-current__name">{levelName(level)}</p>
          <p className="mylevel-current__cando">能做到:{levelCanDo(level, spec)}</p>
        </div>
        <div className="mylevel-current__action">
          <Button onClick={onStartTest}>
            {last ? "重新测一下我的水平" : "测一测我的水平"}
          </Button>
          <p className="mylevel-current__hint">约 3 分钟:认词 + 打整句 + 选语法</p>
        </div>
      </div>

      {last && (
        <section className="mylevel-last">
          <h2>上次测试</h2>
          <p className="mylevel-last__line">
            推荐「{levelName(last.level)}」 · 词汇量约 {last.vocab_est} 词
            {last.sentence_accuracy > 0 &&
              ` · 整句正确率 ${Math.round(last.sentence_accuracy * 100)}%`}
            {last.grammar_notes.map((n) => ` · ${n}`).join("")}
            {` · ${new Date(last.taken_at * 1000).toLocaleDateString()}`}
          </p>
          {last.low_confidence && (
            <p className="mylevel-last__note">那次作答里猜的成分比较多,结果仅供参考。</p>
          )}
          {/* 推荐级可能还没有句子(出厂内容到 L3):按有内容的最高级封顶,
              与定级结果页同一口径,别让"按推荐切换"变成另一扇通往空库的门 */}
          {(() => {
            const top = topLevelWithContent();
            const rec = last.level > top ? top : last.level;
            return (
              <>
                {rec !== last.level && (
                  <p className="mylevel-last__note">
                    「{levelName(last.level)}」的题库还在补齐中,先按「{levelName(rec)}」练。
                  </p>
                )}
                {rec !== level && (
                  <Button
                    variant="secondary"
                    onClick={() => {
                      void setLevel(rec);
                      toast.show(`已切换到「${levelName(rec)}」`);
                    }}
                  >
                    按推荐切换到「{levelName(rec)}」
                  </Button>
                )}
              </>
            );
          })()}
        </section>
      )}

      <section>
        <h2 className="mylevel-grid__title">也可以自己选</h2>
        <div className="onboarding__grid mylevel-grid">
          {specs.map((s) => (
            <button
              key={s.id}
              type="button"
              className={`onboarding__card${s.id === level ? " onboarding__card--on" : ""}`}
              onClick={() => {
                if (s.id === level) return;
                void setLevel(s.id);
                toast.show(`已切换到「${levelName(s.id)}」`);
              }}
            >
              <span className="onboarding__level">{levelName(s.id)}</span>
              <span className="onboarding__cando">能做到:{levelCanDo(s.id, s)}</span>
              {/* 出厂内容目前到 L3:空库等级如实标出来,别让人切过去才发现没题 */}
              {sentenceCountFor(s.id) === 0 && (
                <span className="onboarding__empty">题库补齐中 · 可用 AI 造句</span>
              )}
              {s.id === level && <span className="mylevel-card__badge">当前</span>}
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}
