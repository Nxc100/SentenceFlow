/**
 * AI 答疑抽屉(§4.2/§6.3):底部抽屉,预填模板,打字机流式回答(60 字/秒)。
 * - 会话记录:同一句内多轮问答以气泡累积,近 3 轮随新问题回传做上下文;
 *   进入下一题(句子切换)自动清空;
 * - 回答用轻量 Markdown 排版(加粗/列表/行内代码);
 * - 顶部拖拽条可上下调整抽屉高度(记忆到 localStorage)。
 * 组件在练习页常驻挂载,open 仅控制显隐 —— 关闭再打开不丢当前句的会话。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { Button, Markdown } from "@sentenceflow/ui";
import type { Sentence } from "@sentenceflow/ui";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { events, ipc } from "../ipc";
import type { CmdError } from "../ipc";

interface ChatMsg {
  role: "user" | "ai";
  text: string;
}

/** 打字机速率:60 字/秒(§6.3) ≈ 每 33ms 吐 2 字 */
const TICK_MS = 33;
const CHARS_PER_TICK = 2;

const HEIGHT_KEY = "sf-ask-height";
const MIN_HEIGHT = 240;

const clampHeight = (h: number) => Math.min(Math.round(window.innerHeight * 0.85), Math.max(MIN_HEIGHT, h));

export function AskAiDrawer({
  open,
  sentence,
  onClose,
}: {
  open: boolean;
  sentence: Sentence;
  onClose: () => void;
}) {
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [question, setQuestion] = useState("为什么这里用 __?");
  const [streamText, setStreamText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [height, setHeight] = useState(() =>
    clampHeight(Number(localStorage.getItem(HEIGHT_KEY)) || Math.round(window.innerHeight * 0.4)),
  );

  const bufferRef = useRef("");
  const shownRef = useRef("");
  const doneRef = useRef(false);
  const timerRef = useRef<number | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const unlistens = useRef<UnlistenFn[]>([]);
  const sentenceIdRef = useRef(sentence.id);

  const stopTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const scrollToBottom = useCallback(() => {
    const el = bodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  /** 结束一轮:把已显示文本落入会话记录 */
  const finalize = useCallback(() => {
    stopTimer();
    const text = shownRef.current.trim();
    if (text) setMessages((m) => [...m, { role: "ai", text }]);
    shownRef.current = "";
    bufferRef.current = "";
    setStreamText("");
    setBusy(false);
  }, [stopTimer]);

  /** 打字机:从缓冲匀速吐字;缓冲空且流已结束 → 收尾 */
  const startTimer = useCallback(() => {
    stopTimer();
    timerRef.current = window.setInterval(() => {
      if (bufferRef.current.length > 0) {
        shownRef.current += bufferRef.current.slice(0, CHARS_PER_TICK);
        bufferRef.current = bufferRef.current.slice(CHARS_PER_TICK);
        setStreamText(shownRef.current);
        scrollToBottom();
      } else if (doneRef.current) {
        finalize();
      }
    }, TICK_MS);
  }, [stopTimer, finalize, scrollToBottom]);

  // 句子切换 → 清空本句会话(需求:进入下一题后清除)
  useEffect(() => {
    if (sentenceIdRef.current === sentence.id) return;
    sentenceIdRef.current = sentence.id;
    stopTimer();
    setMessages([]);
    setStreamText("");
    setError(null);
    setBusy(false);
    shownRef.current = "";
    bufferRef.current = "";
    doneRef.current = true;
    setQuestion("为什么这里用 __?");
  }, [sentence.id, stopTimer]);

  // 流事件订阅(常驻;chunk 进缓冲,由打字机匀速消费)
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const u1 = await events.onAskChunk((text) => {
        if (!cancelled) bufferRef.current += text;
      });
      const u2 = await events.onAskDone(() => {
        doneRef.current = true;
      });
      const u3 = await events.onAskError((msg) => {
        doneRef.current = true;
        setError(msg);
      });
      unlistens.current = [u1, u2, u3];
    })();
    return () => {
      cancelled = true;
      unlistens.current.forEach((u) => u());
      unlistens.current = [];
      stopTimer();
    };
  }, [stopTimer]);

  const ask = async () => {
    const q = question.trim();
    if (!q || busy) return;
    // 历史 = 已完成的 user/ai 相邻对
    const history: Array<{ q: string; a: string }> = [];
    for (let i = 0; i + 1 < messages.length; i++) {
      if (messages[i]!.role === "user" && messages[i + 1]!.role === "ai") {
        history.push({ q: messages[i]!.text, a: messages[i + 1]!.text });
      }
    }
    setMessages((m) => [...m, { role: "user", text: q }]);
    setQuestion("");
    setError(null);
    setBusy(true);
    shownRef.current = "";
    bufferRef.current = "";
    doneRef.current = false;
    setStreamText("");
    startTimer();
    scrollToBottom();
    try {
      await ipc.askAi(sentence.id, q, history);
    } catch (e) {
      stopTimer();
      setBusy(false);
      const err = e as CmdError;
      setError(
        err.code === "no_channel" ? "尚未配置 AI 通道 — 在设置 · AI 接入里选择一个" : err.message,
      );
    }
  };

  /** 顶部拖拽条:上下拖动调整高度 */
  const onGripPointerDown = (e: ReactPointerEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startHeight = height;
    const onMove = (ev: PointerEvent) => {
      setHeight(clampHeight(startHeight + (startY - ev.clientY)));
    };
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      localStorage.setItem(HEIGHT_KEY, String(clampHeight(startHeight + (startY - ev.clientY))));
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  if (!open) return null;

  return (
    <div className="ask-drawer" role="dialog" aria-label="AI 答疑" style={{ height }}>
      <div
        className="ask-drawer__grip"
        onPointerDown={onGripPointerDown}
        title="拖动调整高度"
        aria-label="拖动调整高度"
      >
        <span className="ask-drawer__gripbar" />
      </div>
      <div className="ask-drawer__head">
        <span className="ask-drawer__title">问 AI · {sentence.en}</span>
        <button type="button" className="ask-drawer__close" onClick={onClose} aria-label="关闭">
          ✕
        </button>
      </div>
      <div className="ask-drawer__body" ref={bodyRef}>
        {messages.length === 0 && !busy && !error && (
          <div className="ask-drawer__empty">就这句随便问 —— 语法、用法、换种说法都可以。</div>
        )}
        {messages.map((m, i) =>
          m.role === "user" ? (
            <div key={i} className="ask-msg ask-msg--user">
              {m.text}
            </div>
          ) : (
            <div key={i} className="ask-msg ask-msg--ai">
              <Markdown text={m.text} />
            </div>
          ),
        )}
        {busy && (
          <div className="ask-msg ask-msg--ai">
            <Markdown text={streamText} />
            <span className="ask-caret" aria-hidden />
          </div>
        )}
        {error && <div className="ask-drawer__error">{error}</div>}
      </div>
      <div className="ask-drawer__inputrow">
        <input
          className="ask-drawer__input"
          value={question}
          placeholder="继续追问这句…"
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !busy) void ask();
            e.stopPropagation(); // 不让练习页快捷键截获输入
          }}
        />
        <Button onClick={() => void ask()} disabled={busy || !question.trim()}>
          {busy ? "回答中…" : "提问"}
        </Button>
      </div>
    </div>
  );
}
