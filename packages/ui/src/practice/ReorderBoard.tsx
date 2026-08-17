/**
 * ReorderBoard — 拆句重组 (§4.1):词块乱序拼句,点错抖动,支持重复词。
 *
 * 全键盘操作(无需鼠标):
 * - 数字键 1–9/0 直接选对应角标的词块;
 * - ←→ 在未用词块间移动高亮,空格/Enter 选中高亮词块;
 * - 选错同样抖动 + 错误音;鼠标点击仍然可用。
 * Shift/Ctrl 组合键不拦截(切题、掌握等页面级快捷键照常工作)。
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { silentSounds } from "../sounds";
import type { SoundPlayer } from "../sounds";
import type { Sentence } from "../types";

export interface ReorderResult {
  errors: number;
  durMs: number;
}

export interface ReorderBoardProps {
  sentence: Sentence;
  onComplete: (result: ReorderResult) => void;
  /** 洗牌种子(缺省用句 id,保证可复现) */
  seed?: number;
  sounds?: SoundPlayer;
}

interface ChipItem {
  /** 池中唯一 id(= 原词序号) */
  id: number;
  word: string;
}

/** 简单可复现洗牌(mulberry32) */
function shuffled<T>(arr: T[], seed: number): T[] {
  let s = seed >>> 0 || 1;
  const rand = () => {
    s |= 0;
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const out = [...arr];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [out[i], out[j]] = [out[j]!, out[i]!];
  }
  return out;
}

/** 词块角标:1–9 之后第 10 个用 0(对应键盘 0 键);再往后不标 */
function chipBadge(poolIndex: number): string | null {
  if (poolIndex < 9) return String(poolIndex + 1);
  if (poolIndex === 9) return "0";
  return null;
}

export function ReorderBoard({
  sentence,
  onComplete,
  seed,
  sounds = silentSounds,
}: ReorderBoardProps) {
  const chips = useMemo<ChipItem[]>(() => {
    const items = sentence.words.map((w, i) => ({ id: i, word: w.w }));
    const mixed = shuffled(items, seed ?? sentence.id);
    // 保证乱序(短句时洗牌可能恰好原序)
    const inOrder = mixed.every((c, i) => c.id === i);
    return inOrder && mixed.length > 1 ? [...mixed].reverse() : mixed;
  }, [sentence, seed]);

  const [placedIds, setPlacedIds] = useState<number[]>([]);
  const [shakeId, setShakeId] = useState<{ id: number; key: number } | null>(null);
  /** 键盘高亮的池中词块索引 */
  const [focusIdx, setFocusIdx] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const errorsRef = useRef(0);
  const startRef = useRef<number | null>(null);
  const doneRef = useRef(false);
  const shakeKeyRef = useRef(0);

  useEffect(() => {
    setPlacedIds([]);
    setFocusIdx(0);
    errorsRef.current = 0;
    startRef.current = null;
    doneRef.current = false;
    containerRef.current?.focus();
  }, [sentence]);

  useEffect(() => {
    containerRef.current?.focus();
  }, []);

  const nextIndex = placedIds.length;
  const isUsed = (chip: ChipItem) => placedIds.includes(chip.id);

  /** 从 from 起沿 dir 方向找下一个未用词块(回卷) */
  const seekUnused = (from: number, dir: 1 | -1): number => {
    const n = chips.length;
    for (let step = 1; step <= n; step++) {
      const idx = (from + dir * step + n * step) % n;
      if (!isUsed(chips[idx]!)) return idx;
    }
    return from;
  };

  const pickChip = (chip: ChipItem, poolIndex: number) => {
    if (doneRef.current || isUsed(chip)) return;
    if (startRef.current === null) startRef.current = performance.now();
    const expected = sentence.words[nextIndex]?.w;
    // 支持重复词:比对词面而非固定位置 id
    if (expected !== undefined && chip.word.toLowerCase() === expected.toLowerCase()) {
      sounds.key();
      const placed = [...placedIds, chip.id];
      setPlacedIds(placed);
      if (placed.length === sentence.words.length) {
        doneRef.current = true;
        const durMs = startRef.current ? Math.round(performance.now() - startRef.current) : 0;
        window.setTimeout(() => onComplete({ errors: errorsRef.current, durMs }), 260);
      } else {
        // 高亮顺移到下一个未用词块(键盘连选不断流)
        const stillUnused = (idx: number) =>
          idx >= 0 && idx < chips.length && !placed.includes(chips[idx]!.id);
        setFocusIdx(stillUnused(poolIndex) ? poolIndex : seekNextAfterPick(poolIndex, placed));
      }
    } else {
      errorsRef.current += 1;
      sounds.error();
      shakeKeyRef.current += 1;
      setShakeId({ id: chip.id, key: shakeKeyRef.current });
      setFocusIdx(poolIndex);
    }
  };

  const seekNextAfterPick = (from: number, placed: number[]): number => {
    const n = chips.length;
    for (let step = 1; step <= n; step++) {
      const idx = (from + step) % n;
      if (!placed.includes(chips[idx]!.id)) return idx;
    }
    return from;
  };

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    // 页面级组合键(Shift+←→ 切题、Ctrl+M/Q 等)不拦截
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
    const key = e.key;
    // 数字直选:1–9 → 角标 1–9,0 → 角标 0(第 10 块)
    if (/^[0-9]$/.test(key)) {
      e.preventDefault();
      const poolIndex = key === "0" ? 9 : Number(key) - 1;
      const chip = chips[poolIndex];
      if (chip && !isUsed(chip)) pickChip(chip, poolIndex);
      return;
    }
    if (key === "ArrowRight" || key === "ArrowLeft") {
      e.preventDefault();
      setFocusIdx((f) => seekUnused(f, key === "ArrowRight" ? 1 : -1));
      return;
    }
    if (key === " " || key === "Enter") {
      e.preventDefault();
      const chip = chips[focusIdx];
      if (chip && !isUsed(chip)) pickChip(chip, focusIdx);
    }
  };

  return (
    <div
      ref={containerRef}
      className="sf-reorder"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      role="application"
      aria-label="拆句重组练习"
    >
      <div className="sf-reorder__zh">{sentence.zh}</div>
      <div className="sf-reorder__answer">
        {placedIds.map((id, i) => (
          <span key={`${id}-${i}`} className="sf-chip sf-chip--placed">
            {chips.find((c) => c.id === id)?.word}
          </span>
        ))}
        {sentence.punct && placedIds.length === sentence.words.length && (
          <span className="sf-typing__punct">{sentence.punct}</span>
        )}
      </div>
      <div className="sf-reorder__pool">
        {chips.map((chip, poolIndex) => {
          const used = isUsed(chip);
          const shaking = shakeId?.id === chip.id;
          const badge = chipBadge(poolIndex);
          return (
            <button
              key={`${chip.id}-${shaking ? shakeId.key : "s"}`}
              type="button"
              className={[
                "sf-chip",
                used ? "sf-chip--used" : "",
                shaking ? "sf-chip--shake" : "",
                !used && poolIndex === focusIdx ? "sf-chip--focus" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onClick={() => {
                pickChip(chip, poolIndex);
                containerRef.current?.focus();
              }}
              disabled={used}
              tabIndex={-1}
            >
              {badge !== null && !used && <span className="sf-chip__num">{badge}</span>}
              {chip.word}
            </button>
          );
        })}
      </div>
      <div className="sf-reorder__hint">按数字键或点击选词 · ←→ 移动 · 空格/Enter 选中</div>
    </div>
  );
}
