/**
 * 场景对话练习(《场景练习模块-实现方案》§2.4):
 * 按对话顺序逐句练,上方是已完成句的 A/B 气泡流(上下文即意义),
 * 下方用打字板打当前句。不洗牌、不写 SRS(只记练习日志)、不撒花。
 *
 * 复用:TypingBoard(打字)、CompletionPage(完成页)、desktopSpeech(朗读)。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  CompletionPage,
  ProgressBar,
  TypingBoard,
  WebAudioSounds,
  useToast,
} from "@sentenceflow/ui";
import type { Sentence, SessionItem, TypingResult } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { CmdError } from "../ipc";
import { ipc, tzOffsetSecs } from "../ipc";
import { desktopSpeech } from "../speech";

export interface ScenarioLaunch {
  pack: string;
  title: string;
}

interface Turn {
  speaker: string;
  en: string;
  zh: string;
}

/** 无标注句兜底(与练习页同款):按空白分词供打字 */
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

export function ScenarioPracticeScreen({
  launch,
  onExit,
}: {
  launch: ScenarioLaunch;
  onExit: () => void;
}) {
  const { settings } = useApp();
  const toast = useToast();
  const [items, setItems] = useState<SessionItem[] | null>(null);
  const [index, setIndex] = useState(0);
  const [sentence, setSentence] = useState<Sentence | null>(null);
  const [history, setHistory] = useState<Turn[]>([]);
  const [done, setDone] = useState(false);
  const [tally, setTally] = useState({ sentences: 0, errors: 0, wpmSum: 0, wpmCount: 0, durMs: 0 });
  const unannotatedRef = useRef(false);
  const historyRef = useRef<HTMLDivElement>(null);

  const sounds = useMemo(
    () =>
      new WebAudioSounds({
        keySound: settings.sound.key_sound,
        fxVolume: settings.sound.fx_volume,
      }),
    [settings.sound.key_sound, settings.sound.fx_volume],
  );

  // 载入整包会话(后端按对话顺序返回)
  useEffect(() => {
    ipc
      .startScenarioSession(launch.pack)
      .then((session) => setItems(session.items))
      .catch((e) => {
        toast.show(String((e as CmdError).message ?? e));
        onExit();
      });
  }, [launch.pack, toast, onExit]);

  const item = items?.[index] ?? null;

  // 载入当前句
  useEffect(() => {
    if (!item) return;
    setSentence(null);
    void ipc.getSentence(item.sentence_id).then((s) => {
      if (!s) return;
      unannotatedRef.current = s.words.length === 0;
      setSentence(unannotatedRef.current ? tokenizeFallback(s) : s);
    });
  }, [item]);

  // 气泡流自动滚到底
  useEffect(() => {
    const el = historyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [history.length]);

  const finishTurn = useCallback(
    async (result: TypingResult) => {
      if (!sentence || !items) return;
      sounds.correct();
      setHistory((h) => [
        ...h,
        { speaker: sentence.func || "", en: sentence.en, zh: sentence.zh },
      ]);
      setTally((t) => ({
        sentences: t.sentences + 1,
        errors: t.errors + result.errors,
        wpmSum: t.wpmSum + (result.wpm > 0 ? result.wpm : 0),
        wpmCount: t.wpmCount + (result.wpm > 0 ? 1 : 0),
        durMs: t.durMs + result.durMs,
      }));
      // 场景练习只记日志、不进等级复习队列(skip_srs)
      try {
        await ipc.submitAttempt({
          sentence_id: sentence.id,
          mode: "typing",
          outcome: { kind: "correct", seen_answer: result.seenAnswer },
          dur_ms: result.durMs,
          errors: result.errors,
          wpm: result.wpm,
          error_tags: [],
          tz_offset_secs: tzOffsetSecs(),
          skip_srs: true,
        });
      } catch (e) {
        const err = e as CmdError;
        if (err.code === "trial_limit") {
          toast.show(err.message);
          onExit();
          return;
        }
      }
      if (settings.practice.auto_speak_answer) {
        desktopSpeech.speak(sentence.en, {
          rate: settings.sound.rate,
          voice: settings.sound.accent,
        });
      }
      if (index + 1 >= items.length) {
        setDone(true);
      } else {
        setIndex((i) => i + 1);
      }
    },
    [sentence, items, index, sounds, toast, onExit, settings],
  );

  const topBar = (
    <header className="practice-top">
      <button type="button" className="practice-back" onClick={onExit} aria-label="退出练习">
        ‹
      </button>
      <span className="practice-title">
        {launch.title}
        {items && !done && ` (${index + 1}/${items.length})`}
      </span>
    </header>
  );

  if (done) {
    const avgWpm = tally.wpmCount > 0 ? tally.wpmSum / tally.wpmCount : 0;
    const accuracy =
      tally.sentences > 0 ? Math.max(0, 1 - tally.errors / Math.max(1, tally.sentences * 5)) : 0;
    return (
      <div className="practice-screen">
        {topBar}
        <main className="practice-main scenario-done">
          <CompletionPage
            title={`${launch.title} · 完成`}
            stats={{ sentences: tally.sentences, accuracy, avgWpm, durMs: tally.durMs }}
            actions={<Button onClick={onExit}>返回场景列表</Button>}
          >
            <div className="scenario-replay">
              {history.map((t, i) => (
                <div
                  key={i}
                  className={`scenario-bubble scenario-bubble--${t.speaker === "B" ? "b" : "a"}`}
                >
                  <span className="scenario-bubble__en">{t.en}</span>
                  <span className="scenario-bubble__zh">{t.zh}</span>
                </div>
              ))}
            </div>
          </CompletionPage>
        </main>
      </div>
    );
  }

  if (!items || !sentence) {
    return (
      <div className="practice-screen">
        {topBar}
        <div className="practice-main" aria-busy="true" />
      </div>
    );
  }

  return (
    <div className="practice-screen">
      {topBar}
      <ProgressBar value={index / items.length} aria-label="对话进度" />
      <main className="practice-main scenario-practice">
        {history.length > 0 && (
          <div className="scenario-history" ref={historyRef}>
            {history.map((t, i) => (
              <div
                key={i}
                className={`scenario-bubble scenario-bubble--${t.speaker === "B" ? "b" : "a"}`}
              >
                <span className="scenario-bubble__en">{t.en}</span>
                <span className="scenario-bubble__zh">{t.zh}</span>
              </div>
            ))}
          </div>
        )}
        <div className="scenario-current">
          {sentence.func && (
            <span className={`scenario-speaker scenario-speaker--${sentence.func === "B" ? "b" : "a"}`}>
              {sentence.func === "B" ? "你说" : "对方说"}
            </span>
          )}
          <TypingBoard
            key={sentence.id}
            sentence={sentence}
            strict={false}
            sounds={sounds}
            speech={desktopSpeech}
            onComplete={(r) => void finishTurn(r)}
          />
        </div>
      </main>
      <footer className="practice-footer">
        <span className="practice-shortcuts">
          打出这句 · 空格跳到下一词 · Enter 提交 · ↓ 显示答案 · Ctrl 朗读
        </span>
      </footer>
    </div>
  );
}
