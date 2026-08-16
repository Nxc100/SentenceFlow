/**
 * 内容库(§4.3):出厂句库(按等级/场景)· 我的句集 · 错题本 · 收藏。
 * 点句展开解析(无撒花);错题本/收藏一键重练。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, ParseView, useToast } from "@sentenceflow/ui";
import type { LevelId, Sentence } from "@sentenceflow/ui";
import { useApp } from "../appState";
import { ipc } from "../ipc";
import { desktopSpeech } from "../speech";
import type { PracticeLaunch } from "./Practice";

const USER_ID_OFFSET = 1_000_000_000;

type Tab = "factory" | "mine" | "wrongbook" | "favorites";

export function LibraryPage({ onPractice }: { onPractice: (l: PracticeLaunch) => void }) {
  const { level, specs, setLevel } = useApp();
  const toast = useToast();
  const [tab, setTab] = useState<Tab>("factory");
  const [scenes, setScenes] = useState<string[]>([]);
  const [scene, setScene] = useState<string | null>(null);
  const [sentences, setSentences] = useState<Sentence[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [favorites, setFavorites] = useState<Set<number>>(new Set());
  const [importText, setImportText] = useState("");
  const [showImport, setShowImport] = useState(false);

  const refresh = useCallback(async () => {
    setExpanded(null);
    const favs = new Set(await ipc.favorites());
    setFavorites(favs);
    if (tab === "factory" || tab === "mine") {
      const all = await ipc.listSentences(level, scene ?? undefined);
      setSentences(
        all.filter((s) => (tab === "mine" ? s.id >= USER_ID_OFFSET : s.id < USER_ID_OFFSET)),
      );
    } else {
      const ids = tab === "wrongbook" ? await ipc.wrongbook() : [...favs];
      const loaded = await Promise.all(ids.map((id) => ipc.getSentence(id)));
      setSentences(loaded.filter((s): s is Sentence => s !== null));
    }
  }, [tab, level, scene]);

  useEffect(() => {
    void ipc.listScenes(level).then(setScenes);
  }, [level]);

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
        <h1>内容库</h1>
        <select
          className="level-select"
          value={level}
          onChange={(e) => void setLevel(e.target.value as LevelId)}
        >
          {specs.map((s) => (
            <option key={s.id} value={s.id}>
              {s.id}
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

      {tab === "factory" && (
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
        </div>
      )}

      <ul className="library-list">
        {sentences.map((s) => (
          <li key={s.id} className="library-item">
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
    </div>
  );
}
