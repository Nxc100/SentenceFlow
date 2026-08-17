/**
 * 首启定级(§6.3):六张等级卡(各带 can-do)或「帮我测一下」——
 * 后者进入真实的水平测试(词汇快筛 + 整句实测 + 语法辨析,约 3 分钟,
 * 见《英语水平定级测试-实现方案》),替代早期的三问自报熟悉度。
 * 零配置,无通道内容。
 */

import { Button, levelCanDo, levelName } from "@sentenceflow/ui";
import logoUrl from "../assets/logo.png";
import { useApp } from "../appState";

export function Onboarding({ onStartTest }: { onStartTest: () => void }) {
  const { specs, setLevel } = useApp();

  return (
    <div className="onboarding">
      <img src={logoUrl} alt="句流 SentenceFlow" className="onboarding__logo" />
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
            <span className="onboarding__level">{levelName(spec.id)}</span>
            <span className="onboarding__cando">能做到:{levelCanDo(spec.id, spec)}</span>
          </button>
        ))}
      </div>
      <Button variant="ghost" onClick={onStartTest}>
        不确定?帮我测一下(约 3 分钟)
      </Button>
    </div>
  );
}
