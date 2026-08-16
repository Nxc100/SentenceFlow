/**
 * 练习全屏模态(§4.1/§6.2/§6.5):打字 / 重组 / 听打 / 默写 → 解析 → 完成页。
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
  const [tally, setTally] = useState<Tally>({ sentences: 0, errors: 0, wpmSum: 0, wpmCount: 0, durMs: 0 });
  const [askOpen, setAskOpen] = useState(false);
  const lastResult = useRef<TypingResult | null>(null);

  // 载入队列
  useEffect(() => {
    const load =
      launch.kind === "daily"
        ? ipc.startSession(launch.level)
        : ipc.startCustomSession(launch.ids);
    void load.then((session) => {
      setItems(session.items);
      if (session.overflow_reviews > 0) {
        toast.show(`到期复习较多,今天先安排 ${session.items.length} 句`);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const item = items?.[index] ?? null;

  // 载入当前句
  useEffect(() => {
    if (!item) return;
    setSentence(null);
    void ipc.getSentence(item.sentence_id).then((s) => {
      if (!s) {
        toast.show("句子加载失败,已跳过");
        advance();
        return;
      }
      setSentence(s);
      setPhase(item.reorder_first ? "reorder" : "input");
      if (item.mode === "listening") {
        // 听打:自动播 1 遍(§4.1)
        window.setTimeout(() => desktopSpeech.speak(s.en, { rate: settings.sound.rate, voice: settings.sound.accent }), 400);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item?.sentence_id]);

  const advance = useCallback(() => {
    if (!items) return;
    if (index + 1 >= items.length) {
      setDone(true);
    } else {
      setIndex((i) => i + 1);
    }
  }, [items, index]);

  const submitOutcome = useCallback(
    async (mode: Mode, result: TypingResult, sentenceId: number, errorTags: Sentence["words"]) => {
      try {
        await ipc.submitAttempt({
          sentence_id: sentenceId,
          mode,
          outcome: { kind: "correct", seen_answer: result.seenAnswer },
          dur_ms: result.durMs,
          errors: result.errors,
          wpm: result.wpm,
          error_tags: errorTags.map((w) => ({ t: "pos", v: w.pos })),
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

  // 快捷键(§6.5)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPaused((p) => !p);
        return;
      }
      if (!sentence || !item) return;
      if (e.ctrlKey && (e.key === "m" || e.key === "M")) {
        e.preventDefault();
        void ipc
          .submitAttempt({
            sentence_id: sentence.id,
            mode: item.mode,
            outcome: { kind: "mark_mastered" },
            dur_ms: 0,
            errors: 0,
            wpm: 0,
            error_tags: [],
            tz_offset_secs: tzOffsetSecs(),
          })
          .then(() => {
            toast.show("已标记为掌握,直入盒 5");
            advance();
          });
        return;
      }
      if (e.ctrlKey && (e.key === "q" || e.key === "Q")) {
        e.preventDefault();
        void ipc
          .submitAttempt({
            sentence_id: sentence.id,
            mode: item.mode,
            outcome: { kind: "mark_unfamiliar" },
            dur_ms: 0,
            errors: 0,
            wpm: 0,
            error_tags: [],
            tz_offset_secs: tzOffsetSecs(),
          })
          .then(() => {
            toast.show("已收录错题本");
            advance();
          });
        return;
      }
      if (e.shiftKey && e.key === "ArrowRight") {
        e.preventDefault();
        advance();
      }
      if (e.shiftKey && e.key === "ArrowLeft" && index > 0) {
        e.preventDefault();
        setIndex((i) => i - 1);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [sentence, item, advance, index, toast]);

  const speech = useMemo(
    () => ({
      speak: (text: string) =>
        desktopSpeech.speak(text, { rate: settings.sound.rate, voice: settings.sound.accent }),
      stop: () => desktopSpeech.stop(),
    }),
    [settings.sound.rate, settings.sound.accent],
  );

  if (done) {
    const avgWpm = tally.wpmCount > 0 ? tally.wpmSum / tally.wpmCount : 0;
    const accuracy =
      tally.sentences > 0 ? Math.max(0, 1 - tally.errors / Math.max(1, tally.sentences * 5)) : 0;
    return (
      <div className="practice-screen">
        <CompletionPage
          stats={{ sentences: tally.sentences, accuracy, avgWpm, durMs: tally.durMs }}
          actions={<Button onClick={onExit}>回到今日</Button>}
        >
          <p className="practice-rest-hint">练得不错 —— 记得休息一下眼睛。</p>
        </CompletionPage>
      </div>
    );
  }

  if (!items || !item) {
    return <div className="practice-screen" aria-busy="true" />;
  }
  if (items.length === 0) {
    return (
      <div className="practice-screen practice-screen--empty">
        <p>今天没有待练句子。</p>
        <Button onClick={onExit}>返回</Button>
      </div>
    );
  }

  const spec = sentence ? specFor(sentence.level) : undefined;
  const strict = settings.practice.strict_typing && (spec?.practice.judge.strict ?? true);
  const progress = index / items.length;
  const modeLabel: Record<Mode, string> = {
    typing: "打字",
    reorder: "重组",
    listening: "听打",
    dictation: "默写",
  };

  return (
    <div className="practice-screen">
      <header className="practice-top">
        <button type="button" className="practice-back" onClick={onExit} aria-label="退出练习">
          ‹
        </button>
        <span className="practice-title">
          {sentence?.level} · {sentence?.scene ?? "…"} ({index + 1}/{items.length})
          {item.is_review && <span className="practice-review-dot" title="复习句" />}
          <span className="practice-mode-chip">{modeLabel[item.mode]}</span>
        </span>
      </header>
      <ProgressBar value={progress} aria-label="进度" />

      <main className="practice-main">
        {sentence && phase === "reorder" && (
          <ReorderBoard
            sentence={sentence}
            seed={sentence.id}
            onComplete={() => setPhase("input")}
          />
        )}
        {sentence && phase === "input" && item.mode !== "dictation" && (
          <TypingBoard
            sentence={sentence}
            strict={strict}
            hideZh={item.mode === "listening" || settings.practice.hide_chinese}
            speech={speech}
            onRevealAnswer={() => undefined}
            onComplete={(result) => {
              lastResult.current = result;
              setTally((t) => ({
                sentences: t.sentences + 1,
                errors: t.errors + result.errors,
                wpmSum: t.wpmSum + (result.wpm > 0 ? result.wpm : 0),
                wpmCount: t.wpmCount + (result.wpm > 0 ? 1 : 0),
                durMs: t.durMs + result.durMs,
              }));
              void submitOutcome(item.mode, result, sentence.id, result.errors > 0 ? sentence.words : []);
              setPhase("parse");
            }}
          />
        )}
        {sentence && phase === "input" && item.mode === "dictation" && (
          <DictationBoard
            sentence={sentence}
            onCorrect={(durMs) => {
              lastResult.current = { errors: 0, seenAnswer: false, durMs, wpm: 0 };
              setTally((t) => ({ ...t, sentences: t.sentences + 1, durMs: t.durMs + durMs }));
              void submitOutcome("dictation", { errors: 0, seenAnswer: false, durMs, wpm: 0 }, sentence.id, []);
              setPhase("parse");
            }}
            onWrong={() => {
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
            <ParseView sentence={sentence} speech={speech} celebrate />
            <div className="practice-parse-actions">
              <Button onClick={advance}>
                {index + 1 >= items.length ? "完成" : "下一题 →"}
              </Button>
              <Button variant="ghost" onClick={() => setAskOpen(true)}>
                问 AI
              </Button>
            </div>
          </div>
        )}
      </main>

      <footer className="practice-shortcuts">
        空格 跳格 · Enter 提交 · ↓ 显示答案 · Ctrl 朗读 · Ctrl+M 掌握 · Ctrl+Q 不熟悉 · Esc 暂停
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
