/**
 * TypingBoard — 打字模式 (§4.1 / §6.2 微交互)。
 * 严格模式:错字不上屏(抖动 + 红字闪现);宽松模式:上屏标红,Enter 整句校验。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { SpeechService } from "../engine";
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
}

interface Ghost {
  wordIdx: number;
  slotIdx: number;
  ch: string;
  key: number;
}

const CHAR_RE = /^[a-zA-Z'-]$/;

export function TypingBoard({
  sentence,
  strict,
  hideZh,
  onComplete,
  onRevealAnswer,
  speech,
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

  // 严格模式:全部正确即自动提交 (§4.1)
  useEffect(() => {
    if (strict && allFull(typed) && !submittedRef.current) {
      submit();
    }
  }, [typed, strict, allFull, submit]);

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    // 拦截输入法 composition (§4.1 验收)
    if (composingRef.current || e.nativeEvent.isComposing || e.keyCode === 229) {
      e.preventDefault();
      return;
    }
    const key = e.key;

    if (key === "Control" && !e.repeat) {
      speech?.speak(sentence.en);
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
    if (key === " ") {
      e.preventDefault();
      if (allFull(typed)) {
        if (strict) return; // 自动提交已处理
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
      if (!strict && allFull(typed)) lenientValidate();
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
      if (startRef.current === null) startRef.current = performance.now();
      const word = targets[current]!;
      const pos = typed[current]!.length;
      if (pos >= word.length) return; // 满词等待跳转
      const expected = word[pos]!;
      const matches = expected.toLowerCase() === key.toLowerCase();

      if (strict && !matches) {
        // 错字不上屏:抖动 240ms + 红字原位闪现 200ms (§6.2)
        errorsRef.current += 1;
        ghostKeyRef.current += 1;
        setShake({ wordIdx: current, key: ghostKeyRef.current });
        setGhost({ wordIdx: current, slotIdx: pos, ch: key, key: ghostKeyRef.current });
        return;
      }
      if (!strict && !matches) {
        errorsRef.current += 1;
      }
      setTyped((prev) => {
        const copy = prev.map((w) => [...w]);
        copy[current]!.push(key);
        // 词满自动跳下一未完成词
        if (copy[current]!.length >= word.length) {
          const next = nextIncomplete(copy, current);
          if (next !== null) setCurrent(next);
        }
        return copy;
      });
    }
  };

  /** 宽松模式 Enter 整句校验:错误词清空重打 (§4.1) */
  const lenientValidate = () => {
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
      setTyped(cleaned);
      const firstWrong = cleaned.findIndex((l) => l.length === 0);
      if (firstWrong >= 0) setCurrent(firstWrong);
      ghostKeyRef.current += 1;
      setShake({ wordIdx: firstWrong, key: ghostKeyRef.current });
    }
  };

  return (
    <div
      ref={containerRef}
      className="sf-typing"
      tabIndex={0}
      onKeyDown={handleKeyDown}
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
            >
              {Array.from(word).map((expected, li) => {
                const typedCh = letters[li];
                const isCursor = isCurrent && li === letters.length;
                const wrong =
                  !strict && typedCh !== undefined &&
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
      {revealed ? (
        <div className="sf-typing__answer">{sentence.en}</div>
      ) : (
        <div className="sf-typing__hint">直接敲键盘输入 · 大小写不限 · ↓ 显示答案</div>
      )}
    </div>
  );
}
