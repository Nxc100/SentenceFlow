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
  onRoleplay,
}: {
  onPractice: (launch: ScenarioLaunch) => void;
  /** 去生成工坊(场景对话模式)自己造一个场景 */
  onGenerate: () => void;
  /** 去「AI 聊天 · 角色扮演」把这个场景即兴演一遍(AI 演对方) */
  onRoleplay: (prefill: { title: string; roleSystem: string }) => void;
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

  // Esc 关预览(与项目 Modal 组件手感一致)
  useEffect(() => {
    if (!preview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreview(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview]);

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
        <h1>情景对话</h1>
        <Button variant="ghost" onClick={onGenerate}>
          用 AI 生成新场景 →
        </Button>
      </header>
      <p className="scenario-intro">
        照着真实生活里的整段对话练:点餐、值机、看病……不分等级,要用哪个练哪个。
      </p>

      {packs.length === 0 && (
        <div className="scenario-empty">
          <p>还没有对话场景。去「AI 造句」选「场景对话」,描述一个场景就能生成。</p>
          <Button onClick={onGenerate}>去生成我的场景</Button>
        </div>
      )}

      {categories.map((cat) => {
        const deletable = packs.filter((p) => p.category === cat && p.from_user);
        return (
        <section key={cat} className="scenario-group">
          <h2 className="scenario-group__title">
            {cat}
            {/* 批量删除:只清自建场景,出厂场景不受影响 */}
            {deletable.length > 0 && (
              <button
                type="button"
                className="scenario-group__bulk"
                onClick={async () => {
                  const total = deletable.reduce((n, p) => n + p.sentence_count, 0);
                  if (
                    !window.confirm(
                      `确定删除「${cat}」下我生成的 ${deletable.length} 个场景(共 ${total} 句)吗?此操作不可撤销。`,
                    )
                  ) {
                    return;
                  }
                  try {
                    const n = await ipc.deleteUserScenePacks(deletable.map((p) => p.pack));
                    toast.show(`已删除 ${deletable.length} 个场景、${n} 句`);
                    void refresh();
                  } catch (e) {
                    toast.show(String((e as CmdError).message ?? e));
                  }
                }}
              >
                🗑 清空我生成的({deletable.length})
              </button>
            )}
          </h2>
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
                    {p.practiced_count >= p.sentence_count && p.sentence_count > 0 && (
                      <span className="scenario-card__done" title="已练完">
                        ✓
                      </span>
                    )}
                  </span>
                  {p.intro && <span className="scenario-card__intro">{p.intro}</span>}
                  <span className="scenario-card__meta">
                    {p.sentence_count} 句对话
                    {p.reference_level ? ` · 参考难度 ${levelName(p.reference_level)}` : ""}
                  </span>
                  {/* 断点提示:练到一半的包一眼看见进度 */}
                  {p.practiced_count > 0 && p.practiced_count < p.sentence_count && (
                    <span className="scenario-card__progress">
                      练到第 {p.practiced_count} 句
                    </span>
                  )}
                </button>
              ))}
          </div>
        </section>
        );
      })}

      {preview && (
        <div
          className="scenario-preview"
          role="dialog"
          aria-label={`${preview.pack.name} 预览`}
          onClick={() => setPreview(null)}
        >
          {/* 点遮罩关闭;点面板本身不关 */}
          <div className="scenario-preview__panel" onClick={(e) => e.stopPropagation()}>
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
              {/* 断点续练:练到一半时主按钮变「继续」,并保留「从头开始」 */}
              {preview.pack.practiced_count > 0 &&
                preview.pack.practiced_count < preview.pack.sentence_count && (
                  <Button
                    variant="ghost"
                    onClick={() => {
                      const launch = { pack: preview.pack.pack, title: preview.pack.name };
                      setPreview(null);
                      onPractice(launch);
                    }}
                  >
                    从头开始
                  </Button>
                )}
              <Button
                onClick={() => {
                  const resume =
                    preview.pack.practiced_count < preview.pack.sentence_count
                      ? preview.pack.practiced_count
                      : 0;
                  const launch = {
                    pack: preview.pack.pack,
                    title: preview.pack.name,
                    startIndex: resume,
                  };
                  setPreview(null);
                  onPractice(launch);
                }}
              >
                {preview.pack.practiced_count > 0 &&
                preview.pack.practiced_count < preview.pack.sentence_count
                  ? `继续练习(第 ${preview.pack.practiced_count + 1} 句)→`
                  : preview.pack.practiced_count >= preview.pack.sentence_count &&
                      preview.pack.sentence_count > 0
                    ? "再练一遍 →"
                    : "开始练习 →"}
              </Button>
              {/* 照剧本练完 → 去角色扮演即兴用出来(方案 §3.3 联动) */}
              <Button
                variant="ghost"
                title="AI 演对方,你即兴发挥——把背下来的对话真正用出来"
                onClick={() => {
                  const p = preview.pack;
                  const roleSystem = `The scene: "${p.name}"${p.intro ? ` — ${p.intro}` : ""}. You play the counterpart (the staff / the other side) of this scene; the learner plays themselves. Improvise a realistic conversation within this scene, one short turn at a time.`;
                  setPreview(null);
                  onRoleplay({ title: `${p.name} · 实战`, roleSystem });
                }}
              >
                和 AI 实战演练
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
