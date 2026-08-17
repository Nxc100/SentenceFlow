/**
 * 试用版主应用(§7.9):首页(两节选择) → 练习流(重组→打字→签名时刻)
 * → 节完成页(统计 + 进度导出 + 桌面版 CTA)。无账号无网络依赖。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  CompletionPage,
  ParseView,
  ProgressBar,
  ReorderBoard,
  TypingBoard,
  WebAudioSounds,
  levelName,
} from "@sentenceflow/ui";
import type {
  CoreEngine,
  LevelSpec,
  Outcome,
  Sentence,
  SrsState,
  TypingResult,
} from "@sentenceflow/ui";
import trialContent from "./data/trial-content.json";
import { loadEngine } from "./engine/wasmEngine";
import { webSpeech } from "./speech";
import { appendLog, downloadProgress, exportProgress, saveSrs } from "./storage";
import { applyReducedMotion, applyTheme, getReducedMotion, getThemePref } from "./theme";
import type { ThemePref } from "./theme";

/** 购买链接 — 渠道定价为开放问题(§10),接入发卡平台后替换 */
const PURCHASE_URL = "https://example.com/sentenceflow#buy";

interface Section {
  level: string;
  title: string;
  spec: LevelSpec;
  sentences: Sentence[];
}

const SECTIONS = (trialContent as { sections: Section[] }).sections;

type Screen =
  | { kind: "home" }
  | { kind: "practice"; section: Section; index: number; phase: "reorder" | "typing" | "parse" }
  | { kind: "done"; section: Section };

interface SessionTally {
  sentences: number;
  errors: number;
  wpmSum: number;
  wpmCount: number;
  durMs: number;
}

const EMPTY_TALLY: SessionTally = { sentences: 0, errors: 0, wpmSum: 0, wpmCount: 0, durMs: 0 };

export function App() {
  const [engine, setEngine] = useState<CoreEngine | null>(null);
  const [screen, setScreen] = useState<Screen>({ kind: "home" });
  const [tally, setTally] = useState<SessionTally>(EMPTY_TALLY);
  const [theme, setTheme] = useState<ThemePref>(getThemePref());
  const [reduced, setReduced] = useState(getReducedMotion());
  const sounds = useMemo(() => new WebAudioSounds(), []);
  /** 解析动效定格后才允许空格/Enter 进下一句(与桌面端一致) */
  const parseSettledRef = useRef(false);

  useEffect(() => {
    loadEngine().then(setEngine);
  }, []);

  // 解析页键盘流(参照原型):空格/Enter 下一句(动效定格后),←→ 朗读
  useEffect(() => {
    if (screen.kind !== "practice" || screen.phase !== "parse") return;
    const { section, index } = screen;
    const handler = (e: KeyboardEvent) => {
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        if (!parseSettledRef.current) return;
        parseSettledRef.current = false;
        const nextIndex = index + 1;
        const reorderFirst = section.spec.practice.flow === "reorder_then_typing";
        setScreen(
          nextIndex >= section.sentences.length
            ? { kind: "done", section }
            : {
                kind: "practice",
                section,
                index: nextIndex,
                phase: reorderFirst ? "reorder" : "typing",
              },
        );
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        webSpeech.speak(section.sentences[index]?.en ?? "");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [screen]);

  const changeTheme = (t: ThemePref) => {
    setTheme(t);
    applyTheme(t);
  };
  const changeReduced = (on: boolean) => {
    setReduced(on);
    applyReducedMotion(on);
  };

  const startSection = (section: Section) => {
    setTally(EMPTY_TALLY);
    const first = section.sentences[0];
    if (!first) return;
    const reorderFirst = section.spec.practice.flow === "reorder_then_typing";
    setScreen({
      kind: "practice",
      section,
      index: 0,
      phase: reorderFirst ? "reorder" : "typing",
    });
  };

  const recordOutcome = useCallback(
    async (sentence: Sentence, spec: LevelSpec, result: TypingResult) => {
      if (!engine) return;
      const now = Math.floor(Date.now() / 1000);
      const outcome: Outcome = { kind: "correct", seen_answer: result.seenAnswer };
      const prev: SrsState = await engine.newSrsState(now);
      const next = await engine.applyOutcome(prev, outcome, "typing", spec, now);
      await saveSrs(sentence.en, next);
      await appendLog({
        ts: now,
        sentence_id: sentence.id,
        mode: "typing",
        result: "correct",
        dur_ms: result.durMs,
        errors: result.errors,
        wpm: result.wpm,
        seen_answer: result.seenAnswer,
        error_tags: [],
      });
    },
    [engine],
  );

  if (screen.kind === "home") {
    return (
      <div className="trial-shell">
        <header className="trial-hero">
          <img src="/logo.png" alt="句流 SentenceFlow" className="trial-hero__logo" />
          <h1>句流 SentenceFlow</h1>
          <p className="trial-tagline">
            看中文、打英文 —— 答对瞬间,句子自动展开成
            <strong>音标 + 词性 + 成分</strong>的彩色解析。
          </p>
          <p className="trial-sub">在线试用版 · 无需注册 · 进度只存在你的浏览器里</p>
        </header>
        <div className="trial-sections">
          {SECTIONS.map((section) => (
            <button
              key={section.level}
              type="button"
              className="trial-section-card"
              onClick={() => startSection(section)}
              disabled={!engine}
            >
              <span className="trial-section-level">{levelName(section.level)}</span>
              <span className="trial-section-title">{levelName(section.level)}体验节</span>
              <span className="trial-section-cando">
                {section.spec.can_do.join(" · ")} · {section.sentences.length} 句
              </span>
            </button>
          ))}
        </div>
        <ThemeBar theme={theme} onTheme={changeTheme} reduced={reduced} onReduced={changeReduced} />
        <FooterCta />
        <LocalBadge />
      </div>
    );
  }

  if (screen.kind === "done") {
    const avgWpm = tally.wpmCount > 0 ? tally.wpmSum / tally.wpmCount : 0;
    const total = tally.sentences;
    const accuracy = total > 0 ? Math.max(0, 1 - tally.errors / Math.max(1, total * 5)) : 0;
    return (
      <div className="trial-shell">
        <CompletionPage
          title={`${levelName(screen.section.level)}体验节 · 完成!`}
          stats={{ sentences: total, accuracy, avgWpm, durMs: tally.durMs }}
          actions={
            <>
              <Button onClick={() => setScreen({ kind: "home" })}>回到首页</Button>
              <Button
                variant="secondary"
                onClick={async () => downloadProgress(await exportProgress())}
              >
                导出学习进度
              </Button>
            </>
          }
        >
          <p className="trial-export-hint">
            导出的 JSON 可在桌面完整版「设置 → 数据」中导入,试用进度带得走。
          </p>
        </CompletionPage>
        <FooterCta />
        <LocalBadge />
      </div>
    );
  }

  // practice
  const { section, index, phase } = screen;
  const sentence = section.sentences[index];
  if (!sentence) {
    // advance() 已保证 index 不越界;此分支仅防御数据异常
    return null;
  }
  const reorderFirst = section.spec.practice.flow === "reorder_then_typing";
  const progress = index / section.sentences.length;

  const advance = () => {
    const nextIndex = index + 1;
    parseSettledRef.current = false;
    if (nextIndex >= section.sentences.length) {
      setScreen({ kind: "done", section });
    } else {
      setScreen({
        kind: "practice",
        section,
        index: nextIndex,
        phase: reorderFirst ? "reorder" : "typing",
      });
    }
  };

  return (
    <div className="trial-shell trial-shell--practice">
      <header className="trial-practice-top">
        <button
          type="button"
          className="trial-back"
          onClick={() => setScreen({ kind: "home" })}
          aria-label="返回"
        >
          ‹
        </button>
        <span className="trial-practice-title">
          {levelName(section.level)} · {sentence.scene} ({index + 1}/{section.sentences.length})
        </span>
      </header>
      <ProgressBar value={progress} aria-label="本节进度" />
      <main className="trial-practice-main">
        {phase === "reorder" && (
          <ReorderBoard
            sentence={sentence}
            seed={sentence.id}
            sounds={sounds}
            onComplete={() => setScreen({ ...screen, phase: "typing" })}
          />
        )}
        {phase === "typing" && (
          <TypingBoard
            sentence={sentence}
            strict={section.spec.practice.judge.strict}
            speech={webSpeech}
            sounds={sounds}
            onComplete={(result) => {
              parseSettledRef.current = false;
              setTally((t) => ({
                sentences: t.sentences + 1,
                errors: t.errors + result.errors,
                wpmSum: t.wpmSum + (result.wpm > 0 ? result.wpm : 0),
                wpmCount: t.wpmCount + (result.wpm > 0 ? 1 : 0),
                durMs: t.durMs + result.durMs,
              }));
              void recordOutcome(sentence, section.spec, result);
              setScreen({ ...screen, phase: "parse" });
            }}
          />
        )}
        {phase === "parse" && (
          <div className="trial-parse-wrap">
            <ParseView
              sentence={sentence}
              speech={webSpeech}
              sounds={sounds}
              celebrate
              onSettled={() => {
                parseSettledRef.current = true;
              }}
            />
            <div className="trial-parse-next">
              <Button onClick={advance}>
                {index + 1 >= section.sentences.length ? "完成本节" : "下一句 →"}
              </Button>
              <span className="trial-parse-hint">空格 / Enter 下一句</span>
            </div>
          </div>
        )}
      </main>
      <LocalBadge />
    </div>
  );
}

function ThemeBar({
  theme,
  onTheme,
  reduced,
  onReduced,
}: {
  theme: ThemePref;
  onTheme: (t: ThemePref) => void;
  reduced: boolean;
  onReduced: (on: boolean) => void;
}) {
  return (
    <div className="trial-themebar">
      <span>外观:</span>
      {(["light", "system", "dark"] as const).map((t) => (
        <button
          key={t}
          type="button"
          className={`trial-chip${theme === t ? " trial-chip--on" : ""}`}
          onClick={() => onTheme(t)}
        >
          {t === "light" ? "浅色" : t === "dark" ? "深色" : "跟随系统"}
        </button>
      ))}
      <button
        type="button"
        className={`trial-chip${reduced ? " trial-chip--on" : ""}`}
        onClick={() => onReduced(!reduced)}
      >
        减少动效
      </button>
    </div>
  );
}

function FooterCta() {
  return (
    <footer className="trial-footer">
      <a className="trial-cta" href={PURCHASE_URL} target="_blank" rel="noreferrer">
        桌面完整版:六级句库 · 听打默写 · AI 生成工坊 → 购买
      </a>
    </footer>
  );
}

/** 本地徽标(§5.5):数据何时出机永远可见 — 试用版数据从不出机 */
function LocalBadge() {
  return (
    <div className="trial-localbadge" title="所有学习数据仅存于此浏览器,不上传任何服务器">
      🛡 数据仅存本机
    </div>
  );
}
