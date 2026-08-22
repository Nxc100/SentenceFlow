/**
 * 内容库(§4.3):出厂句库(按等级/场景)· 我的句集 · 错题本 · 收藏。
 * 点句展开解析(无撒花);错题本/收藏一键重练。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, ParseView, levelOptionLabel, useToast } from "@sentenceflow/ui";
import type { LevelId, Sentence } from "@sentenceflow/ui";
import { useApp } from "../appState";
import { ipc } from "../ipc";
import { desktopSpeech } from "../speech";
import type { PracticeLaunch } from "./Practice";

const USER_ID_OFFSET = 1_000_000_000;

/** 每页句数:一屏放得下、翻页成本低(句子多时不再无限长列表) */
const PAGE_SIZE = 20;

type Tab = "factory" | "mine" | "wrongbook" | "favorites";

export function LibraryPage({ onPractice }: { onPractice: (l: PracticeLaunch) => void }) {
  const { level, specs, setLevel, sentenceCountFor } = useApp();
  const toast = useToast();
  const [tab, setTab] = useState<Tab>("factory");
  const [scenes, setScenes] = useState<string[]>([]);
  const [scene, setScene] = useState<string | null>(null);
  const [sentences, setSentences] = useState<Sentence[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [favorites, setFavorites] = useState<Set<number>>(new Set());
  const [importText, setImportText] = useState("");
  const [showImport, setShowImport] = useState(false);
  const [page, setPage] = useState(1);

  // 换页签/等级/场景后回到第 1 页(否则会停在越界页显示空列表)
  useEffect(() => {
    setPage(1);
  }, [tab, level, scene]);

  const pageCount = Math.max(1, Math.ceil(sentences.length / PAGE_SIZE));
  const pageSafe = Math.min(page, pageCount);
  const pageItems = sentences.slice((pageSafe - 1) * PAGE_SIZE, pageSafe * PAGE_SIZE);

  const refresh = useCallback(async () => {
    setExpanded(null);
    const favs = new Set(await ipc.favorites());
    setFavorites(favs);
    if (tab === "factory" || tab === "mine") {
      // 场景签按当前页取对应来源:出厂库固定场景,我的句集用生成工坊的任务场景。
      setScenes(await ipc.listScenes(level, tab === "mine" ? "user" : "factory"));
      const all = await ipc.listSentences(level, scene ?? undefined);
      setSentences(
        all.filter((s) => (tab === "mine" ? s.id >= USER_ID_OFFSET : s.id < USER_ID_OFFSET)),
      );
    } else {
      setScenes([]);
      const ids = tab === "wrongbook" ? await ipc.wrongbook() : [...favs];
      const loaded = await Promise.all(ids.map((id) => ipc.getSentence(id)));
      setSentences(loaded.filter((s): s is Sentence => s !== null));
    }
  }, [tab, level, scene]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const practiceAll = () => {
    if (sentences.length === 0) return;
    onPractice({
      kind: "custom",
      ids: sentences.map((s) => s.id),
      title: tab === "wrongbook" ? "错题重练" : "自由练习",
    });
  };

  return (
    <div className="page page--library">
      <header className="page__header">
        <h1>我的句库</h1>
        <select
          className="level-select"
          value={level}
          onChange={(e) => void setLevel(e.target.value as LevelId)}
        >
          {specs.map((s) => (
            <option key={s.id} value={s.id}>
              {levelOptionLabel(s.id, s)}
              {sentenceCountFor(s.id) === 0 ? "(暂无句子)" : ""}
            </option>
          ))}
        </select>
      </header>

      <div className="library-tabs">
        {(
          [
            ["factory", "出厂句库"],
            ["mine", "我的句集"],
            ["wrongbook", "错题本"],
            ["favorites", "收藏"],
          ] as Array<[Tab, string]>
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            className={`library-tab${tab === key ? " library-tab--on" : ""}`}
            onClick={() => {
              setTab(key);
              setScene(null);
            }}
          >
            {label}
          </button>
        ))}
        <div className="library-tabs__spacer" />
        {tab === "mine" && (
          <Button variant="ghost" onClick={() => setShowImport((v) => !v)}>
            导入(中文 Tab 英文)
          </Button>
        )}
        {sentences.length > 0 && (
          <Button variant="secondary" onClick={practiceAll}>
            {tab === "wrongbook" ? "一键重练" : "练这些句子"}
          </Button>
        )}
      </div>

      {showImport && tab === "mine" && (
        <div className="library-import">
          <textarea
            rows={4}
            placeholder={"每行:中文<TAB>英文\n例如:我很好。\tI am fine."}
            value={importText}
            onChange={(e) => setImportText(e.target.value)}
          />
          <Button
            onClick={async () => {
              const n = await ipc.importTabSentences(level, importText);
              toast.show(`已导入 ${n} 句(无标注,先以打字模式练习)`);
              setImportText("");
              setShowImport(false);
              void refresh();
            }}
          >
            导入
          </Button>
        </div>
      )}

      {(tab === "factory" || tab === "mine") && scenes.length > 0 && (
        <div className="library-scenes">
          <button
            type="button"
            className={`sf-chip${scene === null ? " sf-chip--on" : ""}`}
            onClick={() => setScene(null)}
          >
            全部
          </button>
          {scenes.map((s) => (
            <button
              key={s}
              type="button"
              className={`sf-chip${scene === s ? " sf-chip--on" : ""}`}
              onClick={() => setScene(s)}
            >
              {s}
            </button>
          ))}
          {/* 按场景批量删除:只对「我的句集」开放(出厂句库不可删) */}
          {tab === "mine" && scene !== null && sentences.length > 0 && (
            <button
              type="button"
              className="library-bulk-del"
              onClick={async () => {
                if (
                  !window.confirm(
                    `确定删除「${scene}」场景下的全部 ${sentences.length} 句吗?此操作不可撤销。`,
                  )
                ) {
                  return;
                }
                const n = await ipc.deleteUserSentencesByScene(level, scene);
                toast.show(`已删除「${scene}」的 ${n} 句`);
                setScene(null);
                void refresh();
              }}
            >
              🗑 删除「{scene}」全部 {sentences.length} 句
            </button>
          )}
        </div>
      )}

      <ul className="library-list">
        {pageItems.map((s) => (
          // 展开看解析时独占整行:多列布局下一格太窄放不下成分卡
          <li
            key={s.id}
            className={`library-item${expanded === s.id ? " library-item--open" : ""}`}
          >
            <button
              type="button"
              className="library-item__row"
              onClick={() => setExpanded(expanded === s.id ? null : s.id)}
            >
              <span className="library-item__en">{s.en}</span>
              <span className="library-item__zh">{s.zh}</span>
            </button>
            <span className="library-item__ops">
              <button
                type="button"
                className="library-fav"
                title={favorites.has(s.id) ? "取消收藏" : "收藏"}
                onClick={async () => {
                  await ipc.favoriteToggle(s.id, !favorites.has(s.id));
                  void refresh();
                }}
              >
                {favorites.has(s.id) ? "★" : "☆"}
              </button>
              {s.id >= USER_ID_OFFSET && (
                <button
                  type="button"
                  className="library-del"
                  title="删除"
                  onClick={async () => {
                    await ipc.deleteUserSentence(s.id);
                    toast.show("已删除");
                    void refresh();
                  }}
                >
                  🗑
                </button>
              )}
            </span>
            {expanded === s.id && s.words.length > 0 && (
              <div className="library-item__parse">
                <ParseView sentence={s} speech={desktopSpeech} celebrate={false} />
              </div>
            )}
          </li>
        ))}
        {sentences.length === 0 && <li className="library-empty">这里还没有句子。</li>}
      </ul>

      {/* 分页:句子多时不再是无限长列表(§4.3 体验) */}
      {sentences.length > PAGE_SIZE && (
        <nav className="library-pager" aria-label="分页">
          <button
            type="button"
            className="practice-nav"
            disabled={pageSafe <= 1}
            onClick={() => setPage(pageSafe - 1)}
          >
            ‹ 上一页
          </button>
          <span className="library-pager__info">
            第 {pageSafe} / {pageCount} 页 · 共 {sentences.length} 句
          </span>
          <button
            type="button"
            className="practice-nav"
            disabled={pageSafe >= pageCount}
            onClick={() => setPage(pageSafe + 1)}
          >
            下一页 ›
          </button>
        </nav>
      )}
    </div>
  );
}
