/**
 * AI 答疑抽屉(§4.2/§6.3):底部 40% 高,预填模板,流式回答。
 * 无通道时入口不出现(由 Practice 侧控制;这里兜底提示)。
 */

import { useEffect, useRef, useState } from "react";
import { Button } from "@sentenceflow/ui";
import type { Sentence } from "@sentenceflow/ui";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { events, ipc } from "../ipc";
import type { CmdError } from "../ipc";

export function AskAiDrawer({
  open,
  sentence,
  onClose,
}: {
  open: boolean;
  sentence: Sentence;
  onClose: () => void;
}) {
  const [question, setQuestion] = useState("为什么这里用 __?");
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const unlistens = useRef<UnlistenFn[]>([]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void (async () => {
      const u1 = await events.onAskChunk((text) => {
        if (!cancelled) setAnswer((a) => a + text);
      });
      const u2 = await events.onAskDone(() => setBusy(false));
      const u3 = await events.onAskError((msg) => {
        setBusy(false);
        setError(msg);
      });
      unlistens.current = [u1, u2, u3];
    })();
    return () => {
      cancelled = true;
      unlistens.current.forEach((u) => u());
      unlistens.current = [];
    };
  }, [open]);

  if (!open) return null;

  const ask = async () => {
    setAnswer("");
    setError(null);
    setBusy(true);
    try {
      await ipc.askAi(sentence.id, question);
    } catch (e) {
      setBusy(false);
      const err = e as CmdError;
      setError(err.code === "no_channel" ? "尚未配置 AI 通道 — 在设置 · AI 接入里选择一个" : err.message);
    }
  };

  return (
    <div className="ask-drawer" role="dialog" aria-label="AI 答疑">
      <div className="ask-drawer__head">
        <span className="ask-drawer__title">问 AI · {sentence.en}</span>
        <button type="button" className="ask-drawer__close" onClick={onClose} aria-label="关闭">
          ✕
        </button>
      </div>
      <div className="ask-drawer__body">
        {answer && <div className="ask-drawer__answer">{answer}</div>}
        {error && <div className="ask-drawer__error">{error}</div>}
      </div>
      <div className="ask-drawer__inputrow">
        <input
          className="ask-drawer__input"
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !busy) void ask();
          }}
        />
        <Button onClick={() => void ask()} disabled={busy}>
          {busy ? "回答中…" : "提问"}
        </Button>
      </div>
    </div>
  );
}
