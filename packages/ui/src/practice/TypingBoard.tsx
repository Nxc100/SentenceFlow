/**
 * TypingBoard — 打字模式 (§4.1 / §6.2 微交互;交互细节对齐参照原型
 * example/句子打字练习.html)。
 *
 * 严格模式:错字不上屏(抖动 + 红字闪现);宽松模式:上屏标红,Enter 整句校验。
 * 细节:词满后继续敲键自动落入下一未完成词;点击词格定位;↓ 在格内以浅灰
 * 揭示答案(记 seen_answer);Enter 在未完成时定位到第一个错/未完成词;
 * Ctrl 单击(keyup 无组合键)朗读整句;按键/错误音效经 SoundPlayer 注入。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { SpeechService } from "../engine";
import { silentSounds } from "../sounds";
import type { SoundPlayer } from "../sounds";
import type { Sentence } from "../types";

export interface TypingResult {
  errors: number;
  seenAnswer: boolean;
  durMs: number;
  wpm: number;
}

export interface TypingBoardProps {
  sentence: Sentence;
  strict: boolean;
  /** 隐藏中文题面(设置项,§4.8) */
  hideZh?: boolean;
  onComplete: (result: TypingResult) => void;
  onRevealAnswer?: () => void;
  speech?: SpeechService;
  sounds?: SoundPlayer;
}

interface Ghost {
  wordIdx: number;
  slotIdx: number;
  ch: string;
  key: number;
}

/**
 * 可上屏的字符,必须与后端分词口径一致 —— `sf_pipeline::validate::tokenize_en`
 * 保留 `is_ascii_alphanumeric() || '\'' || '-'`。
 * 数字曾漏在白名单外:含数字的词(航班号 BA208、座位 14A)会永远差几格打不满,
 * 用户只能跳过 —— 出厂情景包「机场值机与安检」的第 1、6 句就是这样卡死的。
 */
const CHAR_RE = /^[a-zA-Z0-9'-]$/;

export function TypingBoard({
  sentence,
  strict,
  hideZh,
  onComplete,
  onRevealAnswer,
  speech,
  sounds = silentSounds,
}: TypingBoardProps) {
  const targets = useMemo(() => sentence.words.map((w) => w.w), [sentence]);

  const [typed, setTyped] = useState<string[][]>(() => targets.map(() => []));
  const [current, setCurrent] = useState(0);
  const [shake, setShake] = useState<{ wordIdx: number; key: number } | null>(null);
  const [ghost, setGhost] = useState<Ghost | null>(null);
  const [revealed, setRevealed] = useState(false);

  const containerRef = useRef<HTMLDivElement>(null);
  const errorsRef = useRef(0);
  const seenAnswerRef = useRef(false);
  const startRef = useRef<number | null>(null);
  const submittedRef = useRef(false);
  const ghostKeyRef = useRef(0);
  const composingRef = useRef(false);
  /** Ctrl 是否参与了组合键(参照原型:keyup 无组合才朗读) */
  const ctrlComboRef = useRef(false);

  // 换句时全量重置
  useEffect(() => {
    setTyped(targets.map(() => []));
    setCurrent(0);
    setShake(null);
    setGhost(null);
    setRevealed(false);
    errorsRef.current = 0;
    seenAnswerRef.current = false;
    startRef.current = null;
    submittedRef.current = false;
    containerRef.current?.focus();
  }, [targets]);

  useEffect(() => {
    containerRef.current?.focus();
  }, []);

  const submit = useCallback(() => {
    if (submittedRef.current) return;
    submittedRef.current = true;
    const durMs = startRef.current ? Math.round(performance.now() - startRef.current) : 0;
    const minutes = durMs / 60_000;
    const wpm = minutes > 0 ? targets.length / minutes : 0;
    onComplete({
      errors: errorsRef.current,
      seenAnswer: seenAnswerRef.current,
      durMs,
      wpm: Math.round(wpm * 10) / 10,
    });
  }, [onComplete, targets.length]);

  /** 下一个未完成词(从 from+1 向后,末尾回卷) */
  const nextIncomplete = useCallback(
    (state: string[][], from: number): number | null => {
      const n = targets.length;
      for (let step = 1; step <= n; step++) {
        const idx = (from + step) % n;
        if (state[idx]!.length < targets[idx]!.length) return idx;
      }
      return null;
    },
    [targets],
  );

  const allFull = useCallback(
    (state: string[][]) => state.every((letters, i) => letters.length >= targets[i]!.length),
    [targets],
  );

  const wordRight = useCallback(
    (state: string[][], wi: number) =>
      state[wi]!.length >= targets[wi]!.length &&
      state[wi]!.every((ch, li) => ch.toLowerCase() === targets[wi]![li]!.toLowerCase()),
    [targets],
  );

  // 严格模式:全部正确即自动提交 (§4.1)
  useEffect(() => {
    if (strict && allFull(typed) && !submittedRef.current) {
      submit();
    }
  }, [typed, strict, allFull, submit]);

  const shakeWord = useCallback((wordIdx: number) => {
    ghostKeyRef.current += 1;
    setShake({ wordIdx, key: ghostKeyRef.current });
  }, []);

  /** 输入一个字母;当前词已满时自动落入下一未完成词(参照原型) */
  const typeChar = useCallback(
    (key: string) => {
      if (startRef.current === null) startRef.current = performance.now();
      let target = current;
      if (typed[target]!.length >= targets[target]!.length) {
        const next = nextIncomplete(typed, target);
        if (next === null) return;
        target = next;
      }
      const word = targets[target]!;
      const pos = typed[target]!.length;
      const matches = word[pos]!.toLowerCase() === key.toLowerCase();

      if (strict && !matches) {
        // 错字不上屏:抖动 240ms + 红字原位闪现 200ms (§6.2)
        errorsRef.current += 1;
        sounds.error();
        shakeWord(target);
        setGhost({ wordIdx: target, slotIdx: pos, ch: key, key: ghostKeyRef.current });
        if (target !== current) setCurrent(target);
        return;
      }
      if (!strict && !matches) {
        errorsRef.current += 1;
      }
      sounds.key();
      const copy = typed.map((w) => [...w]);
      copy[target]!.push(key);
      let cursor = target;
      // 词满自动跳下一未完成词
      if (copy[target]!.length >= word.length) {
        const next = nextIncomplete(copy, target);
        if (next !== null) cursor = next;
      }
      setTyped(copy);
      setCurrent(cursor);
    },
    [current, typed, targets, strict, sounds, nextIncomplete, shakeWord],
  );

  /** Enter:严格模式定位到第一个未完成/错误词(参照原型 submit()) */
  const locateFirstProblem = useCallback(() => {
    for (let wi = 0; wi < targets.length; wi++) {
      if (!wordRight(typed, wi)) {
        setCurrent(wi);
        shakeWord(wi);
        return;
      }
    }
  }, [targets.length, typed, wordRight, shakeWord]);

  /** 宽松模式 Enter 整句校验:错误词清空重打 (§4.1) */
  const lenientValidate = useCallback(() => {
    let wrongCount = 0;
    const cleaned = typed.map((letters, i) => {
      const got = letters.join("").toLowerCase();
      const want = targets[i]!.toLowerCase();
      if (got !== want) {
        wrongCount += 1;
        return [];
      }
      return letters;
    });
    if (wrongCount === 0) {
      submit();
    } else {
      sounds.error();
      setTyped(cleaned);
      const firstWrong = cleaned.findIndex((l) => l.length === 0);
      if (firstWrong >= 0) {
        setCurrent(firstWrong);
        shakeWord(firstWrong);
      }
    }
  }, [typed, targets, submit, sounds, shakeWord]);

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    // 拦截输入法 composition (§4.1 验收)
    if (composingRef.current || e.nativeEvent.isComposing || e.keyCode === 229) {
      e.preventDefault();
      return;
    }
    const key = e.key;

    // Ctrl 组合键(掌握/不熟悉等)交给页面层,且不当作输入
    if (e.ctrlKey || e.metaKey) {
      if (key !== "Control" && key !== "Meta") ctrlComboRef.current = true;
      return;
    }
    if (key === "Control") {
      ctrlComboRef.current = false;
      return;
    }
    if (key === "ArrowDown") {
      e.preventDefault();
      if (!revealed) {
        setRevealed(true);
        seenAnswerRef.current = true; // 记"看过答案",不计连对
        onRevealAnswer?.();
      }
      return;
    }
    if (key === "ArrowUp") {
      e.preventDefault();
      setRevealed(false);
      return;
    }
    if (key === "ArrowRight" || key === "ArrowLeft") {
      if (e.shiftKey) return; // Shift+←→ 切题在页面层
      e.preventDefault();
      const n = targets.length;
      setCurrent((c) => (c + (key === "ArrowRight" ? 1 : -1) + n) % n);
      return;
    }
    if (key === " ") {
      e.preventDefault();
      if (allFull(typed)) {
        if (strict) return; // 严格模式全对已自动提交
        lenientValidate();
        return;
      }
      // 当前词为空时空格无动作(防双跳,§4.1)
      if (typed[current]!.length === 0) return;
      const next = nextIncomplete(typed, current);
      if (next !== null) setCurrent(next);
      return;
    }
    if (key === "Enter") {
      e.preventDefault();
      if (allFull(typed)) {
        if (!strict) lenientValidate();
      } else if (strict) {
        // 未写完:定位到第一处问题并抖动提示(参照原型)
        locateFirstProblem();
      } else {
        lenientValidate();
      }
      return;
    }
    if (key === "Backspace") {
      e.preventDefault();
      setTyped((prev) => {
        const copy = prev.map((w) => [...w]);
        let idx = current;
        if (copy[idx]!.length === 0) {
          // 跨词退格
          if (idx === 0) return prev;
          idx = idx - 1;
          setCurrent(idx);
        }
        copy[idx]!.pop();
        return copy;
      });
      return;
    }
    if (CHAR_RE.test(key)) {
      e.preventDefault();
      typeChar(key);
    }
  };

  // Ctrl 单击朗读:keyup 且未参与组合键(参照原型,避免 Ctrl+M 误触发)
  const handleKeyUp = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Control" && !ctrlComboRef.current) {
      speech?.speak(sentence.en);
    }
  };

  return (
    <div
      ref={containerRef}
      className="sf-typing"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onKeyUp={handleKeyUp}
      onCompositionStart={() => {
        composingRef.current = true;
      }}
      onCompositionEnd={() => {
        composingRef.current = false;
      }}
      role="application"
      aria-label="整句打字练习"
    >
      {!hideZh && <div className="sf-typing__zh">{sentence.zh}</div>}
      <div className="sf-typing__line">
        {targets.map((word, wi) => {
          const letters = typed[wi] ?? [];
          const isCurrent = wi === current;
          const shaking = shake?.wordIdx === wi;
          return (
            <span
              key={`${wi}-${shaking ? shake.key : "s"}`}
              className={[
                "sf-typing__word",
                isCurrent ? "sf-typing__word--current" : "",
                shaking ? "sf-typing__word--shake" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onClick={() => {
                setCurrent(wi);
                containerRef.current?.focus();
              }}
            >
              {Array.from(word).map((expected, li) => {
                const typedCh = letters[li];
                const isCursor = isCurrent && li === letters.length;
                const wrong =
                  !strict &&
                  typedCh !== undefined &&
                  typedCh.toLowerCase() !== expected.toLowerCase();
                return (
                  <span
                    key={li}
                    className={`sf-typing__slot${isCursor ? " sf-typing__slot--cursor" : ""}`}
                  >
                    {typedCh !== undefined && (
                      <span
                        className={`sf-typing__letter${wrong ? " sf-typing__letter--wrong" : ""}`}
                      >
                        {typedCh}
                      </span>
                    )}
                    {ghost && ghost.wordIdx === wi && ghost.slotIdx === li && (
                      <span key={ghost.key} className="sf-typing__ghost">
                        {ghost.ch}
                      </span>
                    )}
                  </span>
                );
              })}
            </span>
          );
        })}
        {sentence.punct && <span className="sf-typing__punct">{sentence.punct}</span>}
      </div>
      {revealed && (
        <div className="sf-typing__answer" aria-label="答案">
          {sentence.en}
        </div>
      )}
      <div className="sf-typing__hint">
        {revealed ? "↑ 收起答案 · 已记为看过答案" : "直接敲键盘输入 · 大小写不限 · ↓ 显示答案"}
      </div>
    </div>
  );
}
