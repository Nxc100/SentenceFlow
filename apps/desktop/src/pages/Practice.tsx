/**
 * 练习全屏模态(§4.1/§6.2/§6.5):打字 / 重组 / 听打 / 默写 → 解析 → 完成页。
 * 交互对齐参照原型 example/句子打字练习.html:
 * - 解析页 空格/Enter → 下一题(动效定格后生效),←→ → 朗读整句;
 * - 底栏常驻 ‹上一题/下一题› + 随视图切换的快捷键提示条;
 * - 任何加载/空态都保留顶栏返回按钮 —— 用户永远出得去。
 * 快捷键:Ctrl 朗读 · Ctrl+M 掌握 · Ctrl+Q 不熟悉 · Esc 暂停 · Shift+←→ 切题。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  CompletionPage,
  Modal,
  ParseView,
  ProgressBar,
  ReorderBoard,
  TypingBoard,
  WebAudioSounds,
  useToast,
} from "@sentenceflow/ui";
import type {
  LevelId,
  Mode,
  Sentence,
  SessionItem,
  TypingResult,
  Verdict,
  WordVerdict,
} from "@sentenceflow/ui";
import type { ReactNode } from "react";
import { useApp } from "../appState";
import { ipc, tzOffsetSecs } from "../ipc";
import type { CmdError } from "../ipc";
import { desktopSpeech } from "../speech";
import { AskAiDrawer } from "./AskAiDrawer";

export type PracticeLaunch =
  | { kind: "daily"; level: LevelId }
  | { kind: "custom"; ids: number[]; title: string };

interface Tally {
  sentences: number;
  errors: number;
  wpmSum: number;
  wpmCount: number;
  durMs: number;
}

type Phase = "reorder" | "input" | "parse";

const MODE_LABEL: Record<Mode, string> = {
  typing: "打字",
  reorder: "重组",
  listening: "听打",
  dictation: "默写",
};

/** 无标注句(导入句)兜底:按空白分词供打字,标点保留句末(§4.3 导入句) */
function tokenizeFallback(s: Sentence): Sentence {
  const tokens = s.en
    .split(/\s+/)
    .map((t) => t.replace(/^[^a-zA-Z0-9'’-]+|[^a-zA-Z0-9'’-]+$/g, ""))
    .filter(Boolean);
  const tail = /[.!?,;:]+$/.exec(s.en.trim())?.[0] ?? "";
  return {
    ...s,
    punct: s.punct || tail,
    words: tokens.map((w) => ({ w, ipa: "", pos: "n" as const })),
  };
}

export function PracticeScreen({
  launch,
  onExit,
}: {
  launch: PracticeLaunch;
  onExit: () => void;
}) {
  const { settings, specFor } = useApp();
  const toast = useToast();
  const [items, setItems] = useState<SessionItem[] | null>(null);
  const [index, setIndex] = useState(0);
  const [phase, setPhase] = useState<Phase>("input");
  const [sentence, setSentence] = useState<Sentence | null>(null);
  const [paused, setPaused] = useState(false);
  const [done, setDone] = useState(false);
  const [tally, setTally] = useState<Tally>({
    sentences: 0,
    errors: 0,
    wpmSum: 0,
    wpmCount: 0,
    durMs: 0,
  });
  const [askOpen, setAskOpen] = useState(false);
  /** 解析动效已定格 → 空格/Enter 允许进下一题 */
  const parseSettledRef = useRef(false);
  const ctrlComboRef = useRef(false);
  /** 当前句无标注(导入句):完成后跳过解析直接下一题 */
  const unannotatedRef = useRef(false);

  // 音效:随设置实时更新(§6.5)
  const sounds = useMemo(() => new WebAudioSounds(), []);
  useEffect(() => {
    sounds.setSettings({
      keySound:
        settings.sound.key_sound === "mechanical"
          ? "mechanical"
          : settings.sound.key_sound === "off"
            ? "off"
            : "soft",
      fxVolume: settings.sound.fx_volume,
    });
  }, [sounds, settings.sound.key_sound, settings.sound.fx_volume]);

  // 载入队列
  useEffect(() => {
    const load =
      launch.kind === "daily" ? ipc.startSession(launch.level) : ipc.startCustomSession(launch.ids);
    void load
      .then((session) => {
        setItems(session.items);
        if (session.overflow_reviews > 0) {
          toast.show(`到期复习较多,今天先安排 ${session.items.length} 句`);
        }
      })
      .catch((e) => {
        toast.show(String((e as CmdError).message ?? e));
        setItems([]);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const item = items?.[index] ?? null;

  const advance = useCallback(() => {
    if (!items) return;
    if (index + 1 >= items.length) {
      setDone(true);
    } else {
      setIndex((i) => i + 1);
    }
  }, [items, index]);

  // 载入当前句
  useEffect(() => {
    if (!item) return;
    setSentence(null);
    parseSettledRef.current = false;
    void ipc.getSentence(item.sentence_id).then((s) => {
      if (!s) {
        toast.show("句子加载失败,已跳过");
        advance();
        return;
      }
      unannotatedRef.current = s.words.length === 0;
      setSentence(unannotatedRef.current ? tokenizeFallback(s) : s);
      setPhase(item.reorder_first ? "reorder" : "input");
      if (item.mode === "listening") {
        // 听打:自动播 1 遍(§4.1)
        window.setTimeout(
          () =>
            desktopSpeech.speak(s.en, {
              rate: settings.sound.rate,
              voice: settings.sound.accent,
            }),
          400,
        );
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item?.sentence_id]);

  const submitOutcome = useCallback(
    async (mode: Mode, result: TypingResult, sentenceId: number, errorWords: Sentence["words"]) => {
      try {
        await ipc.submitAttempt({
          sentence_id: sentenceId,
          mode,
          outcome: { kind: "correct", seen_answer: result.seenAnswer },
          dur_ms: result.durMs,
          errors: result.errors,
          wpm: result.wpm,
          error_tags: errorWords.map((w) => ({ t: "pos", v: w.pos })),
          tz_offset_secs: tzOffsetSecs(),
        });
      } catch (e) {
        const err = e as CmdError;
        if (err.code === "trial_limit") {
          toast.show(err.message);
          onExit();
        }
      }
    },
    [toast, onExit],
  );

  const markOutcome = useCallback(
    (kind: "mark_mastered" | "mark_unfamiliar") => {
      if (!sentence || !item) return;
      void ipc
        .submitAttempt({
          sentence_id: sentence.id,
          mode: item.mode,
          outcome: { kind },
          dur_ms: 0,
          errors: 0,
          wpm: 0,
          error_tags: [],
          tz_offset_secs: tzOffsetSecs(),
        })
        .then(() => {
          if (kind === "mark_mastered") {
            sounds.master();
            toast.show("已标记为掌握,直入盒 5");
          } else {
            toast.show("已收录错题本");
          }
          advance();
        });
    },
    [sentence, item, advance, toast, sounds],
  );

  // 快捷键(§6.5;Ctrl 朗读用 keyup 无组合模式,参照原型)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPaused((p) => !p);
        return;
      }
      if (askOpen || paused) return;
      if (e.ctrlKey || e.metaKey) {
        if (e.key !== "Control" && e.key !== "Meta") ctrlComboRef.current = true;
        const k = e.key.toLowerCase();
        if (k === "m") {
          e.preventDefault();
          markOutcome("mark_mastered");
        } else if (k === "q") {
          e.preventDefault();
          markOutcome("mark_unfamiliar");
        }
        return;
      }
      if (e.key === "Control") {
        ctrlComboRef.current = false;
        return;
      }
      if (e.shiftKey && e.key === "ArrowRight") {
        e.preventDefault();
        advance();
        return;
      }
      if (e.shiftKey && e.key === "ArrowLeft" && index > 0) {
        e.preventDefault();
        setIndex((i) => i - 1);
        return;
      }
      // 解析视图:空格/Enter 下一题,←→ 朗读(参照原型)
      if (phase === "parse" && sentence) {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          if (parseSettledRef.current) advance();
        } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
          e.preventDefault();
          desktopSpeech.speak(sentence.en, {
            rate: settings.sound.rate,
            voice: settings.sound.accent,
          });
        }
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      // 解析/重组等无输入焦点的视图下,Ctrl 单击朗读
      if (
        e.key === "Control" &&
        !ctrlComboRef.current &&
        phase !== "input" &&
        sentence &&
        !paused &&
        !askOpen
      ) {
        desktopSpeech.speak(sentence.en, {
          rate: settings.sound.rate,
          voice: settings.sound.accent,
        });
      }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [sentence, item, advance, index, phase, paused, askOpen, markOutcome, settings.sound]);

  const speech = useMemo(
    () => ({
      speak: (text: string) =>
        desktopSpeech.speak(text, { rate: settings.sound.rate, voice: settings.sound.accent }),
      stop: () => desktopSpeech.stop(),
    }),
    [settings.sound.rate, settings.sound.accent],
  );

  /** 顶栏(所有状态常驻 —— 返回永远可用) */
  const topBar = (title: ReactNode) => (
    <header className="practice-top">
      <button type="button" className="practice-back" onClick={onExit} aria-label="退出练习">
        ‹
      </button>
      <span className="practice-title">{title}</span>
    </header>
  );

  if (done) {
    const avgWpm = tally.wpmCount > 0 ? tally.wpmSum / tally.wpmCount : 0;
    const accuracy =
      tally.sentences > 0 ? Math.max(0, 1 - tally.errors / Math.max(1, tally.sentences * 5)) : 0;
    return (
      <div className="practice-screen">
        {topBar("练习完成")}
        <CompletionPage
          stats={{ sentences: tally.sentences, accuracy, avgWpm, durMs: tally.durMs }}
          actions={<Button onClick={onExit}>回到今日</Button>}
        >
          <p className="practice-rest-hint">练得不错 —— 记得休息一下眼睛。</p>
        </CompletionPage>
      </div>
    );
  }

  // 状态顺序:未加载 → 加载中;已加载但为空 → 空态;两者都保留返回(修复:
  // 此前 items=[] 会一直落在"加载中"的空白页,没有任何出口)
  if (items === null) {
    return (
      <div className="practice-screen">
        {topBar("加载中…")}
        <div className="practice-main" aria-busy="true" />
      </div>
    );
  }
  if (items.length === 0 || !item) {
    return (
      <div className="practice-screen">
        {topBar("练习")}
        <main className="practice-main practice-empty">
          <div className="practice-empty__icon">🌱</div>
          <p className="practice-empty__title">这个等级还没有可练的句子</p>
          <p className="practice-empty__sub">
            出厂句库正在成长中 —— 可以先切换到其他等级,或在生成工坊为这个等级生成专属句集。
          </p>
          <Button onClick={onExit}>返回</Button>
        </main>
      </div>
    );
  }

  const spec = sentence ? specFor(sentence.level) : undefined;
  const strict = settings.practice.strict_typing && (spec?.practice.judge.strict ?? true);
  const progress = index / items.length;

  return (
    <div className="practice-screen">
      {topBar(
        <>
          {sentence?.level} · {sentence?.scene ?? "…"} ({index + 1}/{items.length})
          {item.is_review && <span className="practice-review-dot" title="复习句" />}
          <span className="practice-mode-chip">{MODE_LABEL[item.mode]}</span>
        </>,
      )}
      <ProgressBar value={progress} aria-label="进度" />

      <main className="practice-main">
        {sentence && phase === "reorder" && (
          <ReorderBoard
            sentence={sentence}
            seed={sentence.id}
            sounds={sounds}
            onComplete={() => setPhase("input")}
          />
        )}
        {sentence && phase === "input" && item.mode !== "dictation" && (
          <TypingBoard
            sentence={sentence}
            strict={strict}
            hideZh={item.mode === "listening" || settings.practice.hide_chinese}
            speech={speech}
            sounds={sounds}
            onComplete={(result) => {
              setTally((t) => ({
                sentences: t.sentences + 1,
                errors: t.errors + result.errors,
                wpmSum: t.wpmSum + (result.wpm > 0 ? result.wpm : 0),
                wpmCount: t.wpmCount + (result.wpm > 0 ? 1 : 0),
                durMs: t.durMs + result.durMs,
              }));
              void submitOutcome(
                item.mode,
                result,
                sentence.id,
                result.errors > 0 && !unannotatedRef.current ? sentence.words : [],
              );
              // 无标注句(导入句)没有成分/音标可展示:跳过解析直接下一题
              if (unannotatedRef.current) {
                sounds.correct();
                advance();
              } else {
                setPhase("parse");
              }
            }}
          />
        )}
        {sentence && phase === "input" && item.mode === "dictation" && (
          <DictationBoard
            sentence={sentence}
            onCorrect={(durMs) => {
              setTally((t) => ({ ...t, sentences: t.sentences + 1, durMs: t.durMs + durMs }));
              void submitOutcome(
                "dictation",
                { errors: 0, seenAnswer: false, durMs, wpm: 0 },
                sentence.id,
                [],
              );
              setPhase("parse");
            }}
            onWrong={() => {
              sounds.error();
              void ipc.submitAttempt({
                sentence_id: sentence.id,
                mode: "dictation",
                outcome: { kind: "wrong" },
                dur_ms: 0,
                errors: 1,
                wpm: 0,
                error_tags: [],
                tz_offset_secs: tzOffsetSecs(),
              });
            }}
          />
        )}
        {sentence && phase === "parse" && (
          <div className="practice-parse">
            <ParseView
              sentence={sentence}
              speech={speech}
              sounds={sounds}
              celebrate
              onSettled={() => {
                parseSettledRef.current = true;
              }}
            />
            <div className="practice-parse-actions">
              <Button variant="ghost" onClick={() => setAskOpen(true)}>
                问 AI
              </Button>
            </div>
          </div>
        )}
      </main>

      <footer className="practice-footer">
        <button
          type="button"
          className="practice-nav"
          disabled={index === 0}
          onClick={() => setIndex((i) => Math.max(0, i - 1))}
        >
          ‹ 上一题
        </button>
        <span className="practice-shortcuts">
          {phase === "parse"
            ? "空格 / Enter 下一题 · ←→ 朗读 · Ctrl+M 掌握 · Ctrl+Q 不熟悉 · Esc 暂停"
            : "空格 跳格 · Enter 提交 · ↓↑ 答案 · Ctrl 朗读 · Ctrl+M 掌握 · Ctrl+Q 不熟悉 · Esc 暂停"}
        </span>
        <button type="button" className="practice-nav practice-nav--primary" onClick={advance}>
          {index + 1 >= items.length ? "完成 ›" : "下一题 ›"}
        </button>
      </footer>

      <Modal open={paused} title="已暂停" onClose={() => setPaused(false)}>
        <div className="practice-pause">
          <Button onClick={() => setPaused(false)}>继续</Button>
          <Button variant="ghost" onClick={onExit}>
            退出练习
          </Button>
        </div>
      </Modal>

      {sentence && (
        <AskAiDrawer open={askOpen} sentence={sentence} onClose={() => setAskOpen(false)} />
      )}
    </div>
  );
}

/** 默写(§4.1):只显中文,自由输入,Enter 逐词 diff。 */
function DictationBoard({
  sentence,
  onCorrect,
  onWrong,
}: {
  sentence: Sentence;
  onCorrect: (durMs: number) => void;
  onWrong: () => void;
}) {
  const [input, setInput] = useState("");
  const [verdict, setVerdict] = useState<Verdict | null>(null);
  const startRef = useRef<number | null>(null);

  const check = async () => {
    const v = await ipc.judgeText(sentence.id, input);
    setVerdict(v);
    if (v.correct) {
      onCorrect(startRef.current ? Math.round(performance.now() - startRef.current) : 0);
    } else {
      onWrong();
    }
  };

  return (
    <div className="dictation">
      <div className="dictation__zh">{sentence.zh}</div>
      <textarea
        className="dictation__input"
        value={input}
        placeholder="根据中文默写整句,Enter 校验"
        onChange={(e) => {
          if (startRef.current === null) startRef.current = performance.now();
          setInput(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void check();
          }
        }}
        rows={2}
        autoFocus
      />
      {verdict && !verdict.correct && (
        <div className="dictation__diff">
          {verdict.words.map((w, i) => (
            <DiffWord key={i} verdict={w} />
          ))}
        </div>
      )}
    </div>
  );
}

function DiffWord({ verdict }: { verdict: WordVerdict }) {
  switch (verdict.kind) {
    case "correct":
      return <span className="diff diff--ok">{verdict.word}</span>;
    case "wrong":
      return (
        <span className="diff diff--wrong">
          <del>{verdict.got}</del> {verdict.expected}
        </span>
      );
    case "missing":
      return <span className="diff diff--missing">{verdict.expected}</span>;
    case "extra":
      return (
        <span className="diff diff--extra">
          <del>{verdict.got}</del>
        </span>
      );
  }
}
