/**
 * ParseView — 签名时刻(§6.2,产品灵魂):
 * t0 下划线淡出 → +160 成分聚拢 → +300 词性胶囊 stagger → +380 音标
 * → +420 撒花 + 正确音"叮" → +500 朗读 → +700 中文与解析入口。
 * 任意按键跳终态;跳过动效的那一次按键不向页面层透传(stopPropagation),
 * 因此"空格进下一题"永远作用于已定格的画面。
 * 动效自然结束或被跳过时触发 onSettled(页面层据此启用空格/Enter 下一题)。
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type * as React from "react";
import { playConfetti } from "../confetti";
import type { ConfettiHandle } from "../confetti";
import type { SpeechService } from "../engine";
import { silentSounds } from "../sounds";
import type { SoundPlayer } from "../sounds";
import { ROLE_ZH, roleVars } from "../grammar";
import { PosCapsule } from "../components/PosCapsule";
import type { Sentence } from "../types";

export interface ParseViewProps {
  sentence: Sentence;
  speech?: SpeechService;
  sounds?: SoundPlayer;
  /** 撒花与正确音开关(签名时刻专属;库内浏览等场景传 false) */
  celebrate?: boolean;
  /** 动效自然完成或被按键跳过后回调(仅一次) */
  onSettled?: () => void;
  /** 展开"句子解析"抽屉的回调(句型公式 + note) */
  onExpandExplain?: () => void;
}

type Stage = "start" | "grouped" | "pos" | "ipa" | "footer";

const STAGE_TIMES: Array<[Stage, number]> = [
  ["grouped", 160],
  ["pos", 300],
  ["ipa", 380],
  ["footer", 700],
];

function reducedMotion(): boolean {
  return (
    window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
    document.documentElement.dataset.motion === "reduced"
  );
}

export function ParseView({
  sentence,
  speech,
  sounds = silentSounds,
  celebrate = true,
  onSettled,
  onExpandExplain,
}: ParseViewProps) {
  const [stage, setStage] = useState<Stage>("start");
  const [showNote, setShowNote] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const confettiRef = useRef<ConfettiHandle | null>(null);
  const timersRef = useRef<number[]>([]);
  const doneRef = useRef(false);
  const settledRef = useRef(false);

  const stageIdx = (s: Stage) =>
    s === "start" ? 0 : STAGE_TIMES.findIndex(([name]) => name === s) + 1;
  const reached = (s: Stage) => stageIdx(stage) >= stageIdx(s);

  useEffect(() => {
    doneRef.current = false;
    settledRef.current = false;
    setStage("start");
    setShowNote(false);
    const reduced = reducedMotion();

    const settle = () => {
      if (settledRef.current) return;
      settledRef.current = true;
      onSettled?.();
    };

    if (reduced) {
      // 降级:直接终态,无撒花 (§6.1)
      setStage("footer");
      if (celebrate) sounds.correct();
      speech?.speak(sentence.en);
      doneRef.current = true;
      settle();
      return;
    }

    for (const [name, t] of STAGE_TIMES) {
      timersRef.current.push(window.setTimeout(() => setStage(name), t));
    }
    if (celebrate) {
      timersRef.current.push(
        window.setTimeout(() => {
          sounds.correct();
          if (canvasRef.current) confettiRef.current = playConfetti(canvasRef.current);
        }, 420),
      );
    }
    timersRef.current.push(
      window.setTimeout(() => {
        speech?.speak(sentence.en);
      }, 500),
    );
    timersRef.current.push(
      window.setTimeout(() => {
        doneRef.current = true;
        settle();
      }, 720),
    );

    const skip = (e: KeyboardEvent) => {
      if (doneRef.current) return;
      doneRef.current = true;
      // 这次按键只负责"跳到终态",不再向页面层冒泡(否则会立刻切题)
      e.stopPropagation();
      timersRef.current.forEach(clearTimeout);
      confettiRef.current?.cancel();
      setStage("footer");
      settle();
    };
    window.addEventListener("keydown", skip, { capture: true });

    return () => {
      timersRef.current.forEach(clearTimeout);
      timersRef.current = [];
      confettiRef.current?.cancel();
      window.removeEventListener("keydown", skip, { capture: true });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sentence.id]);

  const chunks = useMemo(() => {
    return sentence.chunks.map((chunk, ci) => ({
      key: ci,
      role: chunk.r,
      words: chunk.i
        .filter((i) => i >= 0 && i < sentence.words.length)
        .map((i) => ({ idx: i, ...sentence.words[i]! })),
    }));
  }, [sentence]);

  const cls = [
    "sf-parse",
    reached("grouped") ? "sf-parse--grouped" : "",
    reached("pos") ? "sf-parse--pos" : "",
    reached("ipa") ? "sf-parse--ipa" : "",
    reached("footer") ? "sf-parse--footer" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cls}>
      <div className="sf-parse__chunks">
        {chunks.map((chunk) => (
          <div key={chunk.key} className="sf-parse__chunk" style={roleVars(chunk.role)}>
            <div className="sf-parse__words">
              {chunk.words.map((word) => (
                <div key={word.idx} className="sf-parse__wordcol">
                  <span className="sf-parse__ipa">{word.ipa ? `/${word.ipa}/` : ""}</span>
                  <span
                    className="sf-parse__word"
                    onClick={() => speech?.speak(word.w)}
                    style={{ cursor: speech ? "pointer" : undefined }}
                  >
                    {word.w}
                  </span>
                  <span
                    className="sf-parse__pos"
                    style={{ "--sf-stagger": `${word.idx * 40}ms` } as React.CSSProperties}
                  >
                    <PosCapsule pos={word.pos} />
                  </span>
                </div>
              ))}
            </div>
            <span className="sf-parse__rolename">{ROLE_ZH[chunk.role]}</span>
          </div>
        ))}
      </div>

      <div className="sf-parse__footer">
        <div className="sf-parse__zh">{sentence.zh}</div>
        <button
          type="button"
          className="sf-btn sf-btn--ghost"
          onClick={() => {
            setShowNote((v) => !v);
            onExpandExplain?.();
          }}
        >
          句子解析
        </button>
        {showNote && (
          <div className="sf-parse__note">
            {sentence.chunks.length > 0 && (
              <div className="sf-parse__pattern">
                {sentence.chunks.map((c, i) => (
                  <span key={i} className="sf-parse__patwrap">
                    {i > 0 && <span className="sf-parse__patplus">+</span>}
                    <span className="sf-parse__patchip" style={roleVars(c.r)}>
                      {ROLE_ZH[c.r]}
                    </span>
                  </span>
                ))}
              </div>
            )}
            {sentence.pattern && <div>句型:{sentence.pattern}</div>}
            {sentence.note && <div>{sentence.note}</div>}
          </div>
        )}
      </div>

      {celebrate && <canvas ref={canvasRef} className="sf-parse__confetti" />}
    </div>
  );
}
