/**
 * AI 聊天(doc/AI聊天模块-实现方案.md):三种模式一页承载。
 * - 自由聊天:英语陪聊 + 难度自适应 + ⟦fix⟧纠错小卡(可关);
 * - 角色扮演:角色卡墙 + AI 开场白 + 自定义角色 + 情景对话「实战演练」入口;
 * - 智能体:本机 opencode CLI 的美观外壳(工作目录显式选择,工具活动可视化)。
 * 打字机流式沿用答疑抽屉的缓冲模式(60 字/秒);会话与消息存 progress.db。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Markdown, useToast } from "@sentenceflow/ui";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { events, ipc } from "../ipc";
import type {
  ChatDoneEvent,
  ChatMode,
  ChatThreadInfo,
  CmdError,
  FixCard,
} from "../ipc";
import { useApp } from "../appState";
import { desktopSpeech } from "../speech";
import { ROLE_CARDS } from "./aichatRoles";
import type { RoleCard } from "./aichatRoles";

/** 从情景对话页跳入的「实战演练」预填(§3.3 联动) */
export interface AiChatPrefill {
  title: string;
  roleSystem: string;
}

interface UiMsg {
  role: "user" | "ai";
  text: string;
  fix: FixCard | null;
  /** 智能体本轮完成的工具活动(仅当次会话内存,不落库) */
  tools: string[];
  /** 被停止/超时截断的部分回复 */
  partial?: boolean;
}

const MODES: Array<{ key: ChatMode; label: string; hint: string }> = [
  { key: "free", label: "自由聊天", hint: "随便聊,AI 顺手帮你润色" },
  { key: "roleplay", label: "角色扮演", hint: "面试官、店员、房东…实战开口" },
  { key: "agent", label: "智能体", hint: "让 AI 在你的文件夹里干活" },
];

/** 自由聊天冷启动话题(点一下即代发,§3.2) */
const TOPIC_CHIPS: Array<{ label: string; send: string }> = [
  { label: "聊聊我的周末", send: "Let's talk about my weekend." },
  { label: "介绍我的工作/学习", send: "Can I tell you about my work and study?" },
  { label: "最喜欢的食物", send: "Let's talk about my favorite food." },
];

/** 打字机:60 字/秒 ≈ 每 33ms 吐 2 字(与答疑抽屉一致) */
const TICK_MS = 33;
const CHARS_PER_TICK = 2;

const FIX_TOGGLE_KEY = "sf-chat-fix";

/** 流式展示时藏起纠错标记之后的内容(完整正文由 done 事件给出) */
const stripFix = (t: string) => {
  const i = t.indexOf("⟦");
  return i === -1 ? t : t.slice(0, i);
};

const threadDate = (ts: number) => {
  const d = new Date(ts * 1000);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  return sameDay
    ? d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
};

export function AiChatPage({
  prefill,
  onConsumedPrefill,
}: {
  prefill: AiChatPrefill | null;
  onConsumedPrefill: () => void;
}) {
  const { settings } = useApp();
  const toast = useToast();
  const channel = settings.ai.channel;

  const [mode, setMode] = useState<ChatMode>("free");
  const [threads, setThreads] = useState<ChatThreadInfo[]>([]);
  const [active, setActive] = useState<ChatThreadInfo | null>(null);
  const [messages, setMessages] = useState<UiMsg[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [streamText, setStreamText] = useState("");
  const [activity, setActivity] = useState<string | null>(null);
  const [toolLog, setToolLog] = useState<string[]>([]);
  const [fixEnabled, setFixEnabled] = useState(
    () => localStorage.getItem(FIX_TOGGLE_KEY) !== "0",
  );
  /** roleplay 的角色选择视图 / agent 的目录选择视图 */
  const [pickingRole, setPickingRole] = useState(false);
  const [customRole, setCustomRole] = useState("");
  const [agentSetup, setAgentSetup] = useState(false);
  const [agentDir, setAgentDir] = useState<string | null>(null);

  const bufferRef = useRef("");
  const shownRef = useRef("");
  const doneRef = useRef(false);
  const timerRef = useRef<number | null>(null);
  const pendingDoneRef = useRef<ChatDoneEvent | null>(null);
  const toolsRef = useRef<string[]>([]);
  const activeIdRef = useRef<number | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const consumedPrefillRef = useRef(false);

  activeIdRef.current = active?.id ?? null;

  const scrollToBottom = useCallback(() => {
    const el = bodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  const stopTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  /** 结束一轮:以 done 事件的干净正文落地(打字机只是过场) */
  const finalize = useCallback(() => {
    stopTimer();
    const done = pendingDoneRef.current;
    const text = done ? done.text : stripFix(shownRef.current).trim();
    const tools = toolsRef.current;
    if (text || tools.length > 0) {
      setMessages((m) => [
        ...m,
        { role: "ai", text, fix: done?.fix ?? null, tools, partial: done?.partial },
      ]);
    }
    pendingDoneRef.current = null;
    toolsRef.current = [];
    shownRef.current = "";
    bufferRef.current = "";
    setStreamText("");
    setToolLog([]);
    setActivity(null);
    setBusy(false);
  }, [stopTimer]);

  const startTimer = useCallback(() => {
    stopTimer();
    timerRef.current = window.setInterval(() => {
      if (bufferRef.current.length > 0) {
        shownRef.current += bufferRef.current.slice(0, CHARS_PER_TICK);
        bufferRef.current = bufferRef.current.slice(CHARS_PER_TICK);
        setStreamText(stripFix(shownRef.current));
        scrollToBottom();
      } else if (doneRef.current) {
        finalize();
      }
    }, TICK_MS);
  }, [stopTimer, finalize, scrollToBottom]);

  /** 本地流状态全清(切换/删除会话、开新对话时用;不动 messages) */
  const resetStream = useCallback(() => {
    stopTimer();
    shownRef.current = "";
    bufferRef.current = "";
    toolsRef.current = [];
    pendingDoneRef.current = null;
    doneRef.current = true;
    setStreamText("");
    setToolLog([]);
    setActivity(null);
    setBusy(false);
    setError(null);
  }, [stopTimer]);

  const refreshThreads = useCallback(async () => {
    try {
      setThreads(await ipc.chatThreads());
    } catch {
      /* 列表失败不阻塞页面 */
    }
  }, []);

  useEffect(() => {
    void refreshThreads();
  }, [refreshThreads]);

  // 流事件订阅(常驻,按 thread_id 过滤)
  useEffect(() => {
    let cancelled = false;
    const unlistens: UnlistenFn[] = [];
    void (async () => {
      unlistens.push(
        await events.onChatChunk((p) => {
          if (cancelled || p.thread_id !== activeIdRef.current) return;
          bufferRef.current += p.text;
        }),
        await events.onChatTool((p) => {
          if (cancelled || p.thread_id !== activeIdRef.current) return;
          if (p.status === "completed") {
            toolsRef.current = [...toolsRef.current, p.label];
            setToolLog(toolsRef.current);
            setActivity(null);
          } else if (p.status === "error") {
            setActivity(null);
          } else {
            setActivity(p.label);
          }
          scrollToBottom();
        }),
        await events.onChatDone((p) => {
          if (cancelled || p.thread_id !== activeIdRef.current) return;
          pendingDoneRef.current = p;
          doneRef.current = true;
        }),
        await events.onChatError((p) => {
          if (cancelled || p.thread_id !== activeIdRef.current) return;
          doneRef.current = true;
          setError(
            p.retry_after_secs !== null
              ? `触发限速,等 ${p.retry_after_secs} 秒再发一次`
              : p.message,
          );
        }),
      );
    })();
    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
      stopTimer();
    };
  }, [stopTimer, scrollToBottom]);

  /** 打开一个会话(读历史) */
  const openThread = useCallback(
    async (t: ChatThreadInfo) => {
      resetStream();
      setActive(t);
      setPickingRole(false);
      setAgentSetup(false);
      try {
        const history = await ipc.chatHistory(t.id);
        setMessages(
          history.map((m) => ({
            role: m.role === "user" ? "user" : "ai",
            text: m.text,
            fix: m.fix,
            tools: [],
          })),
        );
        window.setTimeout(scrollToBottom, 0);
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [resetStream, scrollToBottom, toast],
  );

  // 首次载入:自动打开当前模式最近的会话(prefill 流程自己接管)。
  // 用户已经开始交互(有活跃会话/正在流式/在建会话)就绝不接管——
  // 否则首条消息建线程刷新列表时会被这里重置吞掉流(实测踩坑)。
  const initializedRef = useRef(false);
  useEffect(() => {
    if (initializedRef.current || prefill) return;
    if (active || busy || pickingRole || agentSetup) {
      initializedRef.current = true;
      return;
    }
    if (threads.length === 0) return;
    initializedRef.current = true;
    const latest = threads.find((t) => t.mode === mode);
    if (latest) void openThread(latest);
  }, [threads, mode, prefill, active, busy, pickingRole, agentSetup, openThread]);

  /** 模式切换:自动打开该模式最近的会话 */
  const switchMode = useCallback(
    (next: ChatMode) => {
      if (busy) void ipc.chatStop();
      resetStream();
      setMode(next);
      setPickingRole(false);
      setAgentSetup(false);
      const latest = threads.find((t) => t.mode === next);
      if (latest) {
        void openThread(latest);
      } else {
        setActive(null);
        setMessages([]);
        if (next === "roleplay") setPickingRole(true);
        if (next === "agent") setAgentSetup(true);
      }
    },
    [busy, threads, openThread, resetStream],
  );

  /** 新对话入口(按模式进入对应的创建流) */
  const newThread = useCallback(() => {
    if (busy) void ipc.chatStop();
    resetStream();
    setActive(null);
    setMessages([]);
    setPickingRole(mode === "roleplay");
    setAgentSetup(mode === "agent");
    setAgentDir(null);
  }, [busy, mode, resetStream]);

  const createRoleplayThread = useCallback(
    async (card: Pick<RoleCard, "id" | "name" | "system" | "opener">) => {
      try {
        const t = await ipc.chatThreadCreate({
          mode: "roleplay",
          title: card.name,
          roleId: card.id,
          roleSystem: card.system,
          opener: card.opener,
        });
        await refreshThreads();
        await openThread(t);
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [refreshThreads, openThread, toast],
  );

  // 情景对话「实战演练」预填:直接建角色扮演会话进入
  useEffect(() => {
    if (!prefill || consumedPrefillRef.current) return;
    consumedPrefillRef.current = true;
    setMode("roleplay");
    void createRoleplayThread({
      id: "scenario",
      name: prefill.title,
      system: prefill.roleSystem,
      opener: "",
    }).then(() => onConsumedPrefill());
  }, [prefill, createRoleplayThread, onConsumedPrefill]);

  const beginStream = useCallback(() => {
    setError(null);
    setBusy(true);
    shownRef.current = "";
    bufferRef.current = "";
    doneRef.current = false;
    pendingDoneRef.current = null;
    toolsRef.current = [];
    setToolLog([]);
    setActivity(null);
    setStreamText("");
    startTimer();
    window.setTimeout(scrollToBottom, 0);
  }, [startTimer, scrollToBottom]);

  const send = useCallback(
    async (raw?: string) => {
      const text = (raw ?? input).trim();
      if (!text || busy) return;
      let thread = active;
      try {
        if (!thread) {
          if (mode !== "free") return; // roleplay/agent 必先建会话
          thread = await ipc.chatThreadCreate({
            mode: "free",
            title: text.slice(0, 20),
          });
          setActive(thread);
          activeIdRef.current = thread.id;
          void refreshThreads();
        }
        setMessages((m) => [...m, { role: "user", text, fix: null, tools: [] }]);
        setInput("");
        beginStream();
        if (mode === "agent") {
          await ipc.agentSend(thread.id, text);
        } else {
          await ipc.chatSend(thread.id, text, fixEnabled);
        }
      } catch (e) {
        stopTimer();
        setBusy(false);
        const err = e as CmdError;
        setError(
          err.code === "no_channel"
            ? "尚未配置 AI 通道 — 在「设置 · AI 接入」里选择一个"
            : err.code === "not_installed"
              ? "未找到 opencode — 到「设置 · AI 接入」一键安装后再来"
              : String(err.message ?? e),
        );
      }
    },
    [input, busy, active, mode, fixEnabled, beginStream, stopTimer, refreshThreads],
  );

  const stop = useCallback(() => {
    void ipc.chatStop();
  }, []);

  const deleteThread = useCallback(
    async (t: ChatThreadInfo) => {
      if (!window.confirm(`删除「${t.title}」?聊天记录将一并清除。`)) return;
      try {
        await ipc.chatThreadDelete(t.id);
        if (active?.id === t.id) {
          void ipc.chatStop();
          resetStream();
          setActive(null);
          setMessages([]);
        }
        void refreshThreads();
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [active, refreshThreads, toast, resetStream],
  );

  const pickAgentDir = useCallback(async () => {
    try {
      const dir = await ipc.pickFolder("选择智能体的工作目录");
      if (dir) setAgentDir(dir);
    } catch (e) {
      toast.show(String((e as CmdError).message ?? e));
    }
  }, [toast]);

  const startAgentThread = useCallback(async () => {
    if (!agentDir) return;
    const base = agentDir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? agentDir;
    try {
      const t = await ipc.chatThreadCreate({
        mode: "agent",
        title: `📁 ${base}`,
        workdir: agentDir,
      });
      await refreshThreads();
      await openThread(t);
    } catch (e) {
      toast.show(String((e as CmdError).message ?? e));
    }
  }, [agentDir, refreshThreads, openThread, toast]);

  const toggleFix = useCallback(() => {
    setFixEnabled((v) => {
      localStorage.setItem(FIX_TOGGLE_KEY, v ? "0" : "1");
      return !v;
    });
  }, []);

  const modeThreads = threads.filter((t) => t.mode === mode);
  const needChannel = mode !== "agent" && !channel;

  return (
    <div className="page page--aichat">
      <header className="page__header">
        <h1>AI 聊天</h1>
        {channel && (
          <span className="workshop-channel">
            AI:{settings.ai.model_label ?? settings.ai.model?.split("/").pop() ?? channel}
          </span>
        )}
      </header>

      <div className="aichat-tabs">
        {MODES.map((m) => (
          <button
            key={m.key}
            type="button"
            className={`workshop-mode${mode === m.key ? " workshop-mode--on" : ""}`}
            onClick={() => switchMode(m.key)}
          >
            <span className="workshop-mode__label">{m.label}</span>
            <span className="workshop-mode__hint">{m.hint}</span>
          </button>
        ))}
      </div>

      {needChannel ? (
        <ChannelGuide />
      ) : (
        <div className="aichat-body">
          <aside className="aichat-threads">
            <button type="button" className="aichat-threads__new" onClick={newThread}>
              ＋ 新对话
            </button>
            <div className="aichat-threads__list">
              {modeThreads.length === 0 && (
                <div className="aichat-threads__empty">还没有对话</div>
              )}
              {modeThreads.map((t) => (
                <div
                  key={t.id}
                  className={`aichat-thread${active?.id === t.id ? " aichat-thread--on" : ""}`}
                  onClick={() => void openThread(t)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void openThread(t);
                  }}
                >
                  <span className="aichat-thread__title" title={t.workdir || t.title}>
                    {roleEmoji(t)} {t.title}
                  </span>
                  <span className="aichat-thread__meta">{threadDate(t.updated_at)}</span>
                  <button
                    type="button"
                    className="aichat-thread__del"
                    aria-label="删除对话"
                    title="删除对话"
                    onClick={(e) => {
                      e.stopPropagation();
                      void deleteThread(t);
                    }}
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          </aside>

          <section className="aichat-main">
            {pickingRole ? (
              <RolePicker
                custom={customRole}
                onCustomChange={setCustomRole}
                onPick={(card) => void createRoleplayThread(card)}
                onCustomStart={() => {
                  const desc = customRole.trim();
                  if (!desc) return;
                  void createRoleplayThread({
                    id: "custom",
                    name: desc.slice(0, 16),
                    system: desc,
                    opener: "",
                  });
                  setCustomRole("");
                }}
              />
            ) : agentSetup ? (
              <AgentSetup
                dir={agentDir}
                onPick={() => void pickAgentDir()}
                onStart={() => void startAgentThread()}
              />
            ) : (
              <>
                {mode === "agent" && active && (
                  <div className="aichat-workdir" title={active.workdir}>
                    📁 {active.workdir}
                    <span className="aichat-workdir__warn">
                      AI 可在该文件夹读写文件、执行命令
                    </span>
                  </div>
                )}
                <div className="aichat-msgs" ref={bodyRef}>
                  {messages.length === 0 && !busy && (
                    <div className="aichat-empty">
                      {mode === "free" && (
                        <>
                          <p>想到什么就用英文说什么 —— 说错了 AI 会顺手帮你润色。</p>
                          <div className="aichat-chips">
                            {TOPIC_CHIPS.map((c) => (
                              <button
                                key={c.label}
                                type="button"
                                className="aichat-chip"
                                onClick={() => void send(c.send)}
                              >
                                {c.label}
                              </button>
                            ))}
                          </div>
                        </>
                      )}
                      {mode === "agent" && (
                        <p>
                          用中文或英文吩咐它:整理文件、写脚本、查资料……它会在上面的文件夹里干活。
                        </p>
                      )}
                      {mode === "roleplay" && <p>对方正在等你开口 —— 用英文说点什么吧。</p>}
                    </div>
                  )}
                  {messages.map((m, i) => (
                    <MsgBubble
                      key={i}
                      msg={m}
                      original={m.fix ? lastUserTextBefore(messages, i) : ""}
                      accent={settings.sound.accent}
                    />
                  ))}
                  {busy && (
                    <div className="aichat-msg aichat-msg--ai">
                      {toolLog.map((t, i) => (
                        <div key={i} className="aichat-tool aichat-tool--done">
                          ⚙ {t}
                        </div>
                      ))}
                      {activity && <div className="aichat-tool">⚙ {activity}…</div>}
                      <Markdown text={streamText} />
                      <span className="ask-caret" aria-hidden />
                    </div>
                  )}
                  {error && <div className="aichat-error">{error}</div>}
                </div>

                <div className="aichat-composer">
                  {mode !== "agent" && (
                    <label className="aichat-fixtoggle" title="AI 在每条回复下方指出你这句英文可以怎么说更好">
                      <input type="checkbox" checked={fixEnabled} onChange={toggleFix} />
                      帮我纠错
                    </label>
                  )}
                  <textarea
                    className="aichat-input"
                    rows={2}
                    value={input}
                    placeholder={
                      mode === "agent" ? "让它帮你做点什么…(Enter 发送)" : "用英文说点什么…(Enter 发送,Shift+Enter 换行)"
                    }
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        void send();
                      }
                    }}
                  />
                  {busy ? (
                    <Button variant="ghost" onClick={stop}>
                      停止
                    </Button>
                  ) : (
                    <Button onClick={() => void send()} disabled={!input.trim()}>
                      发送
                    </Button>
                  )}
                </div>
              </>
            )}
          </section>
        </div>
      )}
    </div>
  );
}

function roleEmoji(t: ChatThreadInfo): string {
  if (t.mode === "agent") return "";
  if (t.mode === "roleplay") {
    return ROLE_CARDS.find((c) => c.id === t.role_id)?.emoji ?? "🎭";
  }
  return "";
}

function lastUserTextBefore(messages: UiMsg[], index: number): string {
  for (let i = index - 1; i >= 0; i--) {
    if (messages[i]!.role === "user") return messages[i]!.text;
  }
  return "";
}

function MsgBubble({
  msg,
  original,
  accent,
}: {
  msg: UiMsg;
  original: string;
  accent: "gb" | "us";
}) {
  if (msg.role === "user") {
    return <div className="aichat-msg aichat-msg--user">{msg.text}</div>;
  }
  return (
    <div className="aichat-msg aichat-msg--ai">
      {msg.tools.map((t, i) => (
        <div key={i} className="aichat-tool aichat-tool--done">
          ⚙ {t}
        </div>
      ))}
      <Markdown text={msg.text} />
      {msg.partial && <div className="aichat-partial">(已停止,回复不完整)</div>}
      {msg.fix && (
        <div className="aichat-fix">
          <div className="aichat-fix__head">✏️ 帮你润色</div>
          {original && <div className="aichat-fix__orig">{original}</div>}
          <div className="aichat-fix__better">
            {msg.fix.better}
            <button
              type="button"
              className="aichat-fix__speak"
              title="朗读这句"
              aria-label="朗读这句"
              onClick={() =>
                desktopSpeech.speak(msg.fix!.better, { voice: accent === "us" ? "us" : "gb" })
              }
            >
              🔊
            </button>
          </div>
          {msg.fix.why && <div className="aichat-fix__why">{msg.fix.why}</div>}
        </div>
      )}
    </div>
  );
}

function RolePicker({
  custom,
  onCustomChange,
  onPick,
  onCustomStart,
}: {
  custom: string;
  onCustomChange: (v: string) => void;
  onPick: (card: RoleCard) => void;
  onCustomStart: () => void;
}) {
  return (
    <div className="aichat-roles">
      <h2>选一个对手戏角色</h2>
      <p className="aichat-roles__hint">AI 会先开口,你用英文接住就行。</p>
      <div className="aichat-roles__grid">
        {ROLE_CARDS.map((c) => (
          <button key={c.id} type="button" className="aichat-role" onClick={() => onPick(c)}>
            <span className="aichat-role__emoji" aria-hidden>
              {c.emoji}
            </span>
            <span className="aichat-role__name">{c.name}</span>
            <span className="aichat-role__desc">{c.desc}</span>
          </button>
        ))}
      </div>
      <div className="aichat-custom">
        <input
          className="aichat-custom__input"
          value={custom}
          placeholder="自定义角色,如:一位健身教练,督促我用英文汇报锻炼"
          onChange={(e) => onCustomChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onCustomStart();
          }}
        />
        <Button onClick={onCustomStart} disabled={!custom.trim()}>
          开演
        </Button>
      </div>
    </div>
  );
}

function AgentSetup({
  dir,
  onPick,
  onStart,
}: {
  dir: string | null;
  onPick: () => void;
  onStart: () => void;
}) {
  return (
    <div className="aichat-agentsetup">
      <h2>选择智能体的工作目录</h2>
      <p>
        智能体由本机 opencode 驱动,只在你选定的文件夹里工作:读写文件、执行命令、查资料都行。
      </p>
      <div className="aichat-agentsetup__row">
        <span className="aichat-agentsetup__dir" title={dir ?? undefined}>
          {dir ?? "还没有选择文件夹"}
        </span>
        <Button variant="ghost" onClick={onPick}>
          选择文件夹…
        </Button>
      </div>
      <p className="aichat-agentsetup__warn">
        ⚠ AI 将可以在该文件夹内<b>读写文件、执行命令</b>。请为它单独准备一个文件夹,不要选桌面、系统盘根目录或含重要资料的目录。
      </p>
      <Button onClick={onStart} disabled={!dir}>
        开始会话
      </Button>
    </div>
  );
}

function ChannelGuide() {
  return (
    <div className="workshop-guide">
      <h2>先接入一个 AI 通道</h2>
      <p>聊天模式和角色扮演需要任一 AI 通道;智能体只需本机装有 opencode。</p>
      <table className="workshop-guide__table">
        <tbody>
          <tr>
            <td>opencode 本地</td>
            <td>一键安装即免费用(限速,无需登录)</td>
          </tr>
          <tr>
            <td>DeepSeek 官方</td>
            <td>自己的 Key,质量稳定,按量计费</td>
          </tr>
          <tr>
            <td>Zen 直连</td>
            <td>不装 CLI 也能用免费模型</td>
          </tr>
          <tr>
            <td>Ollama 本地</td>
            <td>全离线,本机模型</td>
          </tr>
        </tbody>
      </table>
      <p className="workshop-guide__hint">
        在「设置 · AI 接入」选择通道。<b>不接入也不影响任何学习功能。</b>
      </p>
    </div>
  );
}
