/**
 * ReorderBoard — 拆句重组 (§4.1):词块乱序点击拼句,点错抖动,支持重复词。
 */

import { useEffect, useMemo, useRef, useState } from "react";
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
  /** 池中唯一 id */
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
  const errorsRef = useRef(0);
  const startRef = useRef<number | null>(null);
  const doneRef = useRef(false);
  const shakeKeyRef = useRef(0);

  useEffect(() => {
    setPlacedIds([]);
    errorsRef.current = 0;
    startRef.current = null;
    doneRef.current = false;
  }, [sentence]);

  const nextIndex = placedIds.length;

  const clickChip = (chip: ChipItem) => {
    if (doneRef.current) return;
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
      }
    } else {
      errorsRef.current += 1;
      sounds.error();
      shakeKeyRef.current += 1;
      setShakeId({ id: chip.id, key: shakeKeyRef.current });
    }
  };

  return (
    <div className="sf-reorder" aria-label="拆句重组练习">
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
        {chips.map((chip) => {
          const used = placedIds.includes(chip.id);
          const shaking = shakeId?.id === chip.id;
          return (
            <button
              key={`${chip.id}-${shaking ? shakeId.key : "s"}`}
              type="button"
              className={[
                "sf-chip",
                used ? "sf-chip--used" : "",
                shaking ? "sf-chip--shake" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onClick={() => clickChip(chip)}
              disabled={used}
            >
              {chip.word}
            </button>
          );
        })}
      </div>
    </div>
  );
}
