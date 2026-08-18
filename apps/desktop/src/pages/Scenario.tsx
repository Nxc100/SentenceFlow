/**
 * 场景练习(《场景练习模块-实现方案》§2.3):按分类展示场景包
 * (出厂包 + 生成工坊产出的「我生成的」),点开看对话预览,一键开练。
 * 不分等级——参考难度只作小字提示。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, levelName, useToast } from "@sentenceflow/ui";
import type { Sentence } from "@sentenceflow/ui";
import type { CmdError, ScenePackInfo } from "../ipc";
import { ipc } from "../ipc";
import type { ScenarioLaunch } from "./ScenarioPractice";

export function ScenarioPage({
  onPractice,
  onGenerate,
}: {
  onPractice: (launch: ScenarioLaunch) => void;
  /** 去生成工坊(场景对话模式)自己造一个场景 */
  onGenerate: () => void;
}) {
  const toast = useToast();
  const [packs, setPacks] = useState<ScenePackInfo[] | null>(null);
  const [preview, setPreview] = useState<{ pack: ScenePackInfo; lines: Sentence[] } | null>(null);

  const refresh = useCallback(async () => {
    setPacks(await ipc.listScenePacks());
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openPreview = async (pack: ScenePackInfo) => {
    const lines = await ipc.listPackSentences(pack.pack);
    setPreview({ pack, lines });
  };

  if (!packs) {
    return <div className="page page--scenario" aria-busy="true" />;
  }

  // 分类分组(保持内容包给出的顺序,「我生成的」永远排最后)
  const categories: string[] = [];
  for (const p of packs) {
    if (!categories.includes(p.category)) categories.push(p.category);
  }
  categories.sort((a, b) => Number(a === "我生成的") - Number(b === "我生成的"));

  return (
    <div className="page page--scenario">
      <header className="page__header">
        <h1>场景</h1>
        <Button variant="ghost" onClick={onGenerate}>
          生成我的场景 →
        </Button>
      </header>
      <p className="scenario-intro">
        围绕真实生活场景的整段对话练习,不分等级——要用哪个场景就练哪个。
      </p>

      {packs.length === 0 && (
        <div className="scenario-empty">
          <p>还没有场景包。去生成工坊选「场景对话」,描述一个场景就能生成。</p>
          <Button onClick={onGenerate}>去生成我的场景</Button>
        </div>
      )}

      {categories.map((cat) => (
        <section key={cat} className="scenario-group">
          <h2 className="scenario-group__title">{cat}</h2>
          <div className="scenario-grid">
            {packs
              .filter((p) => p.category === cat)
              .map((p) => (
                <button
                  key={p.pack}
                  type="button"
                  className="scenario-card"
                  onClick={() => void openPreview(p)}
                >
                  <span className="scenario-card__name">
                    {p.name}
                    {p.practiced && <span className="scenario-card__done" title="练过">✓</span>}
                  </span>
                  {p.intro && <span className="scenario-card__intro">{p.intro}</span>}
                  <span className="scenario-card__meta">
                    {p.sentence_count} 句对话
                    {p.reference_level ? ` · 参考难度 ${levelName(p.reference_level)}` : ""}
                  </span>
                </button>
              ))}
          </div>
        </section>
      ))}

      {preview && (
        <div className="scenario-preview" role="dialog" aria-label={`${preview.pack.name} 预览`}>
          <div className="scenario-preview__panel">
            <header className="scenario-preview__head">
              <h2>{preview.pack.name}</h2>
              <button
                type="button"
                className="scenario-preview__close"
                onClick={() => setPreview(null)}
                aria-label="关闭"
              >
                ✕
              </button>
            </header>
            {preview.pack.intro && <p className="scenario-preview__intro">{preview.pack.intro}</p>}
            <div className="scenario-preview__lines">
              {preview.lines.map((s) => (
                <div
                  key={s.id}
                  className={`scenario-bubble scenario-bubble--${s.func === "B" ? "b" : "a"}`}
                >
                  <span className="scenario-bubble__en">{s.en}</span>
                  <span className="scenario-bubble__zh">{s.zh}</span>
                </div>
              ))}
            </div>
            <footer className="scenario-preview__foot">
              <Button
                onClick={() => {
                  const launch = { pack: preview.pack.pack, title: preview.pack.name };
                  setPreview(null);
                  onPractice(launch);
                }}
              >
                开始练习 →
              </Button>
              {preview.pack.from_user && (
                <Button
                  variant="ghost"
                  onClick={async () => {
                    try {
                      await ipc.deleteUserScenePack(preview.pack.pack);
                      toast.show("已删除这个场景包");
                      setPreview(null);
                      void refresh();
                    } catch (e) {
                      toast.show(String((e as CmdError).message ?? e));
                    }
                  }}
                >
                  删除
                </Button>
              )}
            </footer>
          </div>
        </div>
      )}
    </div>
  );
}
