/**
 * opencode 技能面板(智能体模式)——对齐 opencode TUI 的技能选择器
 * (搜索框 + 名称/说明列表 + ↑↓ 选择 + Enter 使用 + Esc 关闭),并补上
 * TUI 没有的三件事:说明完整显示、手动触发型技能也能用、图形化制作。
 *
 * 数据来自后端 skill_catalog(以 `opencode debug skill` 为权威源),
 * 详见 src-tauri/src/skills.rs。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, useToast } from "@sentenceflow/ui";
import { ipc } from "../ipc";
import type { CmdError, SkillCatalog, SkillInfo, SkillState } from "../ipc";

const STATE_TAG: Record<SkillState, { label: string; cls: string; title: string }> = {
  active: { label: "可自动调用", cls: "on", title: "AI 会根据说明自己决定何时使用" },
  unloaded: {
    label: "未加载",
    cls: "warn",
    title: "opencode 没有登记它 —— 点它仍可把内容直接注入这条消息",
  },
};

export function SkillsPanel({
  workdir,
  onClose,
  onUse,
  onAttach,
}: {
  /** 当前智能体会话的工作目录(决定项目级技能;空则只看全局与内置) */
  workdir: string;
  onClose: () => void;
  /** 可自动调用的技能:把「使用 X 技能」写进输入框 */
  onUse: (skill: SkillInfo) => void;
  /** 手动触发型技能:附加到下一条消息(后端注入正文) */
  onAttach: (skill: SkillInfo) => void;
}) {
  const toast = useToast();
  const [catalog, setCatalog] = useState<SkillCatalog | null>(null);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const [editing, setEditing] = useState<{ skill: SkillInfo | null } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setCatalog(null);
    try {
      setCatalog(await ipc.skillCatalog(workdir));
    } catch (e) {
      setCatalog({ skills: [], active_count: 0, warning: String((e as CmdError).message ?? e) });
    }
  }, [workdir]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = catalog?.skills ?? [];
    if (!q) return all;
    return all.filter(
      (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
    );
  }, [catalog, query]);

  useEffect(() => {
    setCursor(0);
  }, [query]);

  const pick = useCallback(
    (s: SkillInfo) => {
      if (s.state === "active") onUse(s);
      else onAttach(s);
    },
    [onUse, onAttach],
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => {
        const next = e.key === "ArrowDown" ? c + 1 : c - 1;
        const clamped = Math.max(0, Math.min(shown.length - 1, next));
        listRef.current?.children[clamped]?.scrollIntoView({ block: "nearest" });
        return clamped;
      });
      return;
    }
    if (e.key === "Enter" && shown[cursor]) {
      e.preventDefault();
      pick(shown[cursor]!);
    }
  };

  const remove = async (s: SkillInfo) => {
    if (!window.confirm(`删除技能「${s.name}」?整个技能文件夹会移入回收站(可找回)。`)) return;
    try {
      await ipc.skillDelete(s.path);
      toast.show(`已删除「${s.name}」(在回收站里)`);
      void load();
    } catch (e) {
      toast.show(String((e as CmdError).message ?? e));
    }
  };

  if (editing) {
    return (
      <SkillEditor
        skill={editing.skill}
        workdir={workdir}
        onCancel={() => setEditing(null)}
        onSaved={() => {
          setEditing(null);
          void load();
        }}
      />
    );
  }

  const brokenCount = (catalog?.skills ?? []).filter((s) => s.state === "unloaded").length;

  return (
    <div className="skills-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="skills-panel" role="dialog" aria-label="技能" onKeyDown={onKeyDown}>
        <header className="skills-panel__head">
          <h2>技能</h2>
          <span className="skills-panel__esc" onClick={onClose} role="button" tabIndex={0}>
            esc
          </span>
        </header>
        <input
          ref={searchRef}
          className="skills-panel__search"
          placeholder="搜索技能…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />

        <div className="skills-panel__list" ref={listRef}>
          {!catalog && <div className="skills-panel__hint">正在读取技能清单…</div>}
          {catalog?.warning && <div className="skills-panel__warn">{catalog.warning}</div>}
          {catalog && shown.length === 0 && !catalog.warning && (
            <div className="skills-panel__hint">
              {query ? "没有匹配的技能" : "还没有技能 —— 点右下角「制作技能」造一个"}
            </div>
          )}
          {shown.map((s, i) => {
            const tag = STATE_TAG[s.state];
            return (
              <div
                key={s.path || s.name}
                className={`skill-row${i === cursor ? " skill-row--on" : ""}`}
                onMouseEnter={() => setCursor(i)}
                onClick={() => pick(s)}
                role="button"
                tabIndex={-1}
              >
                <div className="skill-row__top">
                  <span className="skill-row__name">{s.name}</span>
                  <span className={`skill-row__tag skill-row__tag--${tag.cls}`} title={tag.title}>
                    {tag.label}
                  </span>
                  <span className="skill-row__source" title={s.path || "opencode 内置技能"}>
                    {s.source_label}
                    {s.copies > 1 && (
                      <em
                        className="skill-row__copies"
                        title={`磁盘上有 ${s.copies} 份同名技能,实际生效的是这一份`}
                      >
                        ×{s.copies}
                      </em>
                    )}
                  </span>
                  {s.editable && (
                    <span className="skill-row__acts">
                      <button
                        type="button"
                        title="直接把技能内容注入下一条消息(不靠 AI 自己判断)"
                        aria-label="注入"
                        onClick={(e) => {
                          e.stopPropagation();
                          onAttach(s);
                        }}
                      >
                        ⤵
                      </button>
                      <button
                        type="button"
                        title="编辑这个技能"
                        aria-label="编辑"
                        onClick={(e) => {
                          e.stopPropagation();
                          setEditing({ skill: s });
                        }}
                      >
                        ✎
                      </button>
                      <button
                        type="button"
                        title="删除这个技能"
                        aria-label="删除"
                        onClick={(e) => {
                          e.stopPropagation();
                          void remove(s);
                        }}
                      >
                        🗑
                      </button>
                    </span>
                  )}
                </div>
                <div className="skill-row__desc">
                  {s.description || <em>(没有写「什么时候用它」—— AI 看不见这个技能)</em>}
                </div>
                {s.state === "unloaded" && s.reason && (
                  <div className="skill-row__reason" title="点编辑后保存,通常就能修好">
                    ⚠ {s.reason}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <footer className="skills-panel__foot">
          <span>
            {catalog
              ? `${catalog.active_count} 个可自动调用${brokenCount ? ` · ${brokenCount} 个未加载` : ""}`
              : "…"}
          </span>
          <span className="skills-panel__tip">↑↓ 选择 · Enter 使用 · Esc 关闭</span>
          <Button onClick={() => setEditing({ skill: null })}>＋ 制作技能</Button>
        </footer>
      </div>
    </div>
  );
}

/** 技能制作/编辑表单:填三样东西,生成合规的 SKILL.md */
function SkillEditor({
  skill,
  workdir,
  onCancel,
  onSaved,
}: {
  /** null = 新建 */
  skill: SkillInfo | null;
  workdir: string;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState(skill?.name ?? "");
  const [description, setDescription] = useState(skill?.description ?? "");
  const [body, setBody] = useState("");
  const [scope, setScope] = useState<"global" | "project">("global");
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(!skill);

  useEffect(() => {
    if (!skill) return;
    void ipc
      .skillSource(skill.path)
      .then((src) => {
        setName(src.name || skill.name);
        setDescription(src.description);
        setBody(src.body);
        setLoaded(true);
      })
      .catch((e) => {
        toast.show(String((e as CmdError).message ?? e));
        setLoaded(true);
      });
  }, [skill, toast]);

  const save = async () => {
    setBusy(true);
    try {
      await ipc.skillSave({
        path: skill?.path,
        scope,
        workdir,
        name: name.trim().toLowerCase(),
        description,
        body,
      });
      toast.show(skill ? `已更新「${name}」` : `技能「${name}」已创建,现在就能用了`);
      onSaved();
    } catch (e) {
      toast.show(String((e as CmdError).message ?? e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="skills-overlay" onClick={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="skills-panel skills-panel--editor" role="dialog" aria-label="制作技能">
        <header className="skills-panel__head">
          <h2>{skill ? `编辑技能 · ${skill.name}` : "制作技能"}</h2>
          <span className="skills-panel__esc" onClick={onCancel} role="button" tabIndex={0}>
            esc
          </span>
        </header>

        <div className="skills-panel__form">
          <label className="skill-field">
            <span className="skill-field__label">
              技能名
              <em>小写英文、数字和连字符,如 weekly-report</em>
            </span>
            <input
              value={name}
              disabled={Boolean(skill)}
              placeholder="my-skill"
              onChange={(e) => setName(e.target.value)}
            />
            {skill && <span className="skill-field__note">名字创建后不能改(改名请删掉重建)</span>}
          </label>

          <label className="skill-field">
            <span className="skill-field__label">
              什么时候用它
              <em>AI 就是靠这句话决定要不要用这个技能 —— 写清楚触发场景</em>
            </span>
            <textarea
              rows={3}
              value={description}
              placeholder="当用户要写周报、需要把零散记录整理成汇报材料时使用。"
              onChange={(e) => setDescription(e.target.value)}
            />
          </label>

          <label className="skill-field">
            <span className="skill-field__label">
              技能指令
              <em>AI 用这个技能时要遵守的步骤与规则,支持 Markdown</em>
            </span>
            <textarea
              className="skill-field__body"
              rows={12}
              value={loaded ? body : "读取中…"}
              placeholder={"# 周报写法\n\n1. 先按项目分组\n2. 每项写「做了什么 / 结果 / 下一步」\n3. 结尾附一句风险提示"}
              onChange={(e) => setBody(e.target.value)}
            />
          </label>

          {!skill && (
            <div className="skill-field">
              <span className="skill-field__label">在哪里生效</span>
              <div className="skill-scope">
                {(
                  [
                    ["global", "所有文件夹", "存到 ~/.config/opencode/skills,任何智能体会话都能用"],
                    ["project", "只在这个文件夹", "存到工作目录的 .opencode/skills,跟着项目走"],
                  ] as const
                ).map(([key, label, hint]) => (
                  <button
                    key={key}
                    type="button"
                    className={`skill-scope__opt${scope === key ? " skill-scope__opt--on" : ""}`}
                    disabled={key === "project" && !workdir}
                    title={key === "project" && !workdir ? "先打开一个智能体会话" : hint}
                    onClick={() => setScope(key)}
                  >
                    <b>{label}</b>
                    <span>{hint}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>

        <footer className="skills-panel__foot">
          <span className="skills-panel__tip">
            技能保存后立即生效,下一条消息就能用上。
          </span>
          <Button variant="ghost" onClick={onCancel}>
            取消
          </Button>
          <Button onClick={() => void save()} disabled={busy || !name.trim() || !description.trim() || !body.trim()}>
            {busy ? "保存中…" : "保存技能"}
          </Button>
        </footer>
      </div>
    </div>
  );
}
