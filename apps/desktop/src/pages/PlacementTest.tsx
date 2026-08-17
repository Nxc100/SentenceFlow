/**
 * 定级测试(《英语水平定级测试-实现方案》§2/§3.6):
 * 词汇快筛(认识/不认识)→ 整句实测(重组/打字/听打)→ 语法辨析 → 结果页。
 *
 * 状态机在后端 sf-core(placement_start/answer),本页只做呈现与作答转发;
 * 全程不写 SRS/练习日志、不占试用版每日额度。结果页由用户确认后才改等级
 * (onPick),不静默切换。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  ProgressBar,
  ReorderBoard,
  TypingBoard,
  WebAudioSounds,
  levelCanDo,
  levelName,
} from "@sentenceflow/ui";
import type { LevelId, TypingResult } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { CmdError, PlacementAnswer, PlacementStep } from "../ipc";
import { ipc } from "../ipc";
import { desktopSpeech } from "../speech";

export interface PlacementTestScreenProps {
  /** 关闭(未采纳推荐):首启回等级卡,设置入口回设置页。 */
  onExit: () => void;
  /** 用户确认采纳推荐等级。 */
  onPick: (level: LevelId) => void;
}

export function PlacementTestScreen({ onExit, onPick }: PlacementTestScreenProps) {
  const { settings, specs } = useApp();
  const [step, setStep] = useState<PlacementStep | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);

  const sounds = useMemo(
    () =>
      new WebAudioSounds({
        keySound: settings.sound.key_sound,
        fxVolume: settings.sound.fx_volume,
      }),
    [settings.sound.key_sound, settings.sound.fx_volume],
  );

  // 开始测试;听打题仅在语音可用时启用(不可用自动退化为打字题)
  useEffect(() => {
    const allowListening = typeof window !== "undefined" && "speechSynthesis" in window;
    ipc
      .placementStart(allowListening)
      .then(setStep)
      .catch((e) => setError(String((e as CmdError).message ?? e)));
  }, []);

  const answer = useCallback(async (a: PlacementAnswer) => {
    if (busyRef.current) return;
    busyRef.current = true;
    try {
      setStep(await ipc.placementAnswer(a));
    } catch (e) {
      setError(String((e as CmdError).message ?? e));
    } finally {
      busyRef.current = false;
    }
  }, []);

  const item = step?.item ?? null;
  const result = step?.result ?? null;

  // 听打:题目出现后自动播一遍(与练习页同款交互)
  const sentenceKey = item?.kind === "sentence" ? item.sentence.id : null;
  useEffect(() => {
    if (item?.kind !== "sentence" || item.mode !== "listening") return;
    const timer = window.setTimeout(() => {
      desktopSpeech.speak(item.sentence.en, {
        rate: settings.sound.rate,
        voice: settings.sound.accent,
      });
    }, 400);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sentenceKey]);

  // 词汇 ←/→、语法 1/2 的键盘作答
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!item || e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Escape") {
        onExit();
        return;
      }
      if (item.kind === "vocab") {
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          sounds.key();
          void answer({ kind: "vocab", known: false });
        } else if (e.key === "ArrowRight") {
          e.preventDefault();
          sounds.key();
          void answer({ kind: "vocab", known: true });
        }
      } else if (item.kind === "grammar" && (e.key === "1" || e.key === "2")) {
        e.preventDefault();
        sounds.key();
        void answer({ kind: "grammar", choice: Number(e.key) - 1 });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [item, answer, onExit, sounds]);

  const onSentenceDone = (r: TypingResult) =>
    // 键击/选块错误数作词级错误的代理:0–1 错记通过,≥3 记未通过,
    // 阈值语义与状态机一致(方案 §3.3)。
    void answer({
      kind: "sentence",
      word_errors: r.errors,
      seen_answer: r.seenAnswer,
      dur_ms: r.durMs,
      wpm: r.wpm,
    });

  // 「不会,跳过」:练习组件的宽松校验要求最终打对才能提交(练习语义),
  // 测试语义必须允许放弃——记满错 + 看过答案(必然判未通过)并推进,
  // 否则不会这句的用户会被永远卡住。
  const skipSentence = (wordCount: number) =>
    void answer({
      kind: "sentence",
      word_errors: Math.max(3, wordCount),
      seen_answer: true,
      dur_ms: 0,
      wpm: 0,
    });

  const topBar = (
    <header className="practice-top">
      <button type="button" className="practice-back" onClick={onExit} aria-label="退出测试">
        ‹
      </button>
      <span className="practice-title">水平测试</span>
      <div className="placement-progress">
        <ProgressBar value={step?.progress ?? 0} aria-label="测试进度" />
      </div>
    </header>
  );

  if (error) {
    return (
      <div className="practice-screen">
        {topBar}
        <main className="practice-main practice-empty">
          <div className="practice-empty__icon">😥</div>
          <p className="practice-empty__title">测试无法开始</p>
          <p className="practice-empty__sub">{error}</p>
          <Button onClick={onExit}>返回</Button>
        </main>
      </div>
    );
  }

  if (!step) {
    return (
      <div className="practice-screen">
        {topBar}
        <div className="practice-main" aria-busy="true" />
      </div>
    );
  }

  if (result) {
    const spec = specs.find((s) => s.id === result.level);
    return (
      <div className="practice-screen">
        {topBar}
        <main className="practice-main placement-result">
          <p className="placement-result__eyebrow">适合你的起点是</p>
          <h1 className="placement-result__level">{levelName(result.level)}</h1>
          <p className="placement-result__cando">能做到:{levelCanDo(result.level, spec)}</p>
          <ul className="placement-result__why">
            <li>词汇量约 {result.vocab_est} 词</li>
            {result.sentence_accuracy > 0 && (
              <li>整句正确率 {Math.round(result.sentence_accuracy * 100)}%</li>
            )}
            {result.grammar_notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
          {result.low_confidence && (
            <p className="placement-result__note">
              这次作答里猜的成分比较多,推荐偏保守——练两天觉得简单,随时可以调高等级。
            </p>
          )}
          <div className="placement-result__actions">
            <Button onClick={() => onPick(result.level)}>按这个等级开始练习</Button>
            <Button variant="ghost" onClick={onExit}>
              {settings.level === null ? "自己挑一个等级" : "保持当前等级"}
            </Button>
          </div>
        </main>
      </div>
    );
  }

  return (
    <div className="practice-screen">
      {topBar}
      <main className="practice-main">
        {item?.kind === "vocab" && (
          <div className="placement-vocab">
            <p className="placement-hint">认识这个单词吗?凭直觉,不确定就选「不认识」</p>
            <div className="placement-vocab__word">{item.word}</div>
            <div className="placement-vocab__actions">
              <Button
                variant="secondary"
                onClick={() => {
                  sounds.key();
                  void answer({ kind: "vocab", known: false });
                }}
              >
                ← 不认识
              </Button>
              <Button
                onClick={() => {
                  sounds.key();
                  void answer({ kind: "vocab", known: true });
                }}
              >
                认识 →
              </Button>
            </div>
          </div>
        )}

        {item?.kind === "sentence" && item.mode === "reorder" && (
          <div className="placement-typing">
            <ReorderBoard
              sentence={item.sentence}
              seed={item.sentence.id}
              sounds={sounds}
              onComplete={(r) =>
                void answer({
                  kind: "sentence",
                  word_errors: r.errors,
                  seen_answer: false,
                  dur_ms: r.durMs,
                  wpm: 0,
                })
              }
            />
            <div className="placement-skip">
              <Button variant="ghost" onClick={() => skipSentence(item.sentence.words.length)}>
                这句不会,跳过
              </Button>
            </div>
          </div>
        )}
        {item?.kind === "sentence" && item.mode !== "reorder" && (
          <div className="placement-typing">
            {item.mode === "listening" && (
              <div className="placement-listen">
                <Button
                  variant="ghost"
                  onClick={() =>
                    desktopSpeech.speak(item.sentence.en, {
                      rate: settings.sound.rate,
                      voice: settings.sound.accent,
                    })
                  }
                >
                  🔊 再听一遍
                </Button>
              </div>
            )}
            <TypingBoard
              key={item.sentence.id}
              sentence={item.sentence}
              strict={false}
              hideZh={item.mode === "listening"}
              sounds={sounds}
              speech={desktopSpeech}
              onComplete={onSentenceDone}
            />
            <p className="placement-hint placement-hint--below">
              {item.mode === "listening"
                ? "听录音打出整句 · 空格跳到下一词 · Enter 提交"
                : "打出整句 · 空格跳到下一词 · Enter 提交"}
            </p>
            <div className="placement-skip">
              <Button variant="ghost" onClick={() => skipSentence(item.sentence.words.length)}>
                这句不会,跳过
              </Button>
            </div>
          </div>
        )}

        {item?.kind === "grammar" && (
          <div className="placement-grammar">
            <p className="placement-hint">选出正确的说法(按 1 / 2)</p>
            <p className="placement-grammar__zh">{item.prompt_zh}</p>
            <p className="placement-grammar__stem">{item.stem}</p>
            <div className="placement-grammar__options">
              {item.options.map((opt, i) => (
                <button
                  key={opt}
                  type="button"
                  className="placement-option"
                  onClick={() => {
                    sounds.key();
                    void answer({ kind: "grammar", choice: i });
                  }}
                >
                  <span className="placement-option__num">{i + 1}</span>
                  {opt}
                </button>
              ))}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
