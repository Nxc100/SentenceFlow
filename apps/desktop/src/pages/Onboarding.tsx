/**
 * 首启定级(§6.3):六张等级卡(各带 can-do)或"帮我测一下"
 * (自适应 3 句定级的轻量近似:自报熟悉度三问)。零配置,无通道内容。
 */

import { useState } from "react";
import { Button } from "@sentenceflow/ui";
import type { LevelId } from "@sentenceflow/ui";
import { useApp } from "../appState";

const QUIZ: Array<{ q: string; levels: [LevelId, LevelId, LevelId] }> = [
  { q: "看到 “Could you tell me where the station is?” 你的感觉是?", levels: ["L1", "L3", "L5"] },
  { q: "用英语打电话约时间,你能做到吗?", levels: ["L2", "L4", "L5"] },
  { q: "描述上周末做了什么(过去时),流畅吗?", levels: ["L1", "L3", "L6"] },
];

export function Onboarding() {
  const { specs, setLevel } = useApp();
  const [quiz, setQuiz] = useState<number | null>(null);
  const [picks, setPicks] = useState<LevelId[]>([]);

  if (quiz !== null) {
    const item = QUIZ[quiz];
    if (!item) {
      // 取三次自报的中位
      const order: LevelId[] = ["L1", "L2", "L3", "L4", "L5", "L6"];
      const sorted = [...picks].sort((a, b) => order.indexOf(a) - order.indexOf(b));
      const mid = sorted[Math.floor(sorted.length / 2)] ?? "L1";
      void setLevel(mid);
      return null;
    }
    return (
      <div className="onboarding">
        <h2>{item.q}</h2>
        <div className="onboarding__quiz">
          {(["完全陌生", "大致可以", "轻松搞定"] as const).map((label, i) => (
            <Button
              key={label}
              variant="secondary"
              onClick={() => {
                setPicks((p) => [...p, item.levels[i as 0 | 1 | 2]]);
                setQuiz(quiz + 1);
              }}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="onboarding">
      <h1>选择你的起点</h1>
      <p className="onboarding__sub">随时可在设置里调整;30 秒后你就会打出第一句。</p>
      <div className="onboarding__grid">
        {specs.map((spec) => (
          <button
            key={spec.id}
            type="button"
            className="onboarding__card"
            onClick={() => void setLevel(spec.id)}
          >
            <span className="onboarding__level">{spec.id}</span>
            <span className="onboarding__cefr">≈{spec.cefr}</span>
            <span className="onboarding__cando">{spec.can_do.join(" · ")}</span>
          </button>
        ))}
      </div>
      <Button variant="ghost" onClick={() => setQuiz(0)}>
        不确定?帮我测一下
      </Button>
    </div>
  );
}
