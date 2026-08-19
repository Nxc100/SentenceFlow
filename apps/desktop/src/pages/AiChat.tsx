/**
 * AI 聊天(doc/AI聊天模块-实现方案.md):三种模式一页承载。
 * - 自由聊天:英语陪聊 + 难度自适应 + ⟦fix⟧纠错小卡(可关);
 * - 角色扮演:角色卡墙 + AI 开场白 + 自定义角色 + 情景对话「实战演练」入口;
 * - 智能体:本机 opencode CLI 的美观外壳(工作目录显式选择,工具活动可视化)。
 *
 * 流式状态按会话 id 分桶(streamsRef):多个会话可同时生成,切走再切回来
 * 照样看到"正在思考";[停止] 也只掐当前这一个会话。打字机速率沿用答疑
 * 抽屉的 60 字/秒缓冲模式。
 *
 * 每个会话可单独切模型(把 opencode 的 /model 包成可视化操作):选择存进
 * chat_thread,下一条消息起生效;opencode 会话换模型后仍沿用同一 -s 会话。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Markdown, Modal, useToast } from "@sentenceflow/ui";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { events, ipc } from "../ipc";
import type {
  ChannelId,
  ChannelStatus,
  ChatDoneEvent,
  ChatMode,
  ChatThreadInfo,
  CmdError,
  FixCard,
  ModelInfo,
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

/** 一个会话正在进行的流(按 thread id 分桶,切换会话不丢) */
interface ThreadStream {
  /** 已收到、还没吐完的字 */
  buffer: string;
  /** 已吐出的字 */
  shown: string;
  /** 已完成的工具活动 */
  tools: string[];
  /** 正在进行的工具活动 */
  activity: string | null;
  /** 后端已 done/error,吐完缓冲即收尾 */
  ended: boolean;
  done: ChatDoneEvent | null;
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

const CHANNEL_NAMES: Record<ChannelId, string> = {
  opencode: "opencode 本地",
  deepseek: "DeepSeek 官方",
  zen: "Zen 直连",
  ollama: "Ollama 本地",
};

/** 打字机:60 字/秒 ≈ 每 33ms 吐 2 字(与答疑抽屉一致) */
const TICK_MS = 33;
const CHARS_PER_TICK = 2;

const FIX_TOGGLE_KEY = "sf-chat-fix";

/** 流式展示时藏起纠错标记之后的内容(完整正文由 done 事件给出) */
const stripFix = (t: string) => {
  const i = t.indexOf("⟦");
  return i === -1 ? t : t.slice(0, i);
};

const shortModel = (id: string) => id.split("/").pop() ?? id;

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
  const [fixEnabled, setFixEnabled] = useState(
    () => localStorage.getItem(FIX_TOGGLE_KEY) !== "0",
  );
  /** 正在生成的会话 id(侧栏小圆点 + 当前会话的发送/停止切换) */
  const [liveIds, setLiveIds] = useState<number[]>([]);
  /** 当前会话的实时流视图(打字机吐出的字 + 工具活动) */
  const [view, setView] = useState<{ text: string; tools: string[]; activity: string | null }>({
    text: "",
    tools: [],
    activity: null,
  });
  /** 按会话保留的错误提示(切走再回来仍看得到) */
  const [errors, setErrors] = useState<Record<number, string>>({});
  /** roleplay 的角色选择视图 / agent 的目录选择视图 */
  const [pickingRole, setPickingRole] = useState(false);
  const [customRole, setCustomRole] = useState("");
  const [agentSetup, setAgentSetup] = useState(false);
  const [agentDir, setAgentDir] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<ChatThreadInfo | null>(null);
  /** 通道探测缓存(模型选择器共用,秒开) */
  const [statuses, setStatuses] = useState<Partial<Record<ChannelId, ChannelStatus>>>({});
  /** 还没建会话时先选好的模型,建完立刻应用 */
  const [pending, setPending] = useState<{
    channel: ChannelId;
    model: string;
    label: string;
  } | null>(null);

  const streamsRef = useRef(new Map<number, ThreadStream>());
  const timerRef = useRef<number | null>(null);
  const activeIdRef = useRef<number | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const consumedPrefillRef = useRef(false);

  activeIdRef.current = active?.id ?? null;

  const scrollToBottom = useCallback(() => {
    const el = bodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  const syncLive = useCallback(() => {
    setLiveIds([...streamsRef.current.keys()]);
  }, []);

  /** 把当前会话的流状态推给渲染层(没有流则清空) */
  const syncView = useCallback(() => {
    const s = activeIdRef.current !== null ? streamsRef.current.get(activeIdRef.current) : undefined;
    setView(
      s
        ? { text: stripFix(s.shown), tools: s.tools, activity: s.activity }
        : { text: "", tools: [], activity: null },
    );
  }, []);

  const refreshThreads = useCallback(async () => {
    try {
      const list = await ipc.chatThreads();
      setThreads(list);
      // 当前会话的元数据(模型/标题)可能被后端改过,同步一份
      const id = activeIdRef.current;
      if (id !== null) {
        const fresh = list.find((t) => t.id === id);
        if (fresh) setActive(fresh);
      }
    } catch {
      /* 列表失败不阻塞页面 */
    }
  }, []);

  useEffect(() => {
    void refreshThreads();
  }, [refreshThreads]);

  // 进页面就把当前通道探一遍(模型选择器打开时已有缓存,不转圈)
  useEffect(() => {
    if (!channel || statuses[channel]) return;
    void ipc
      .probeChannel(channel)
      .then((s) => setStatuses((prev) => ({ ...prev, [channel]: s })))
      .catch(() => {});
  }, [channel, statuses]);

  /** 一轮流收尾:落地成消息气泡(仅当前会话)并撤掉流状态 */
  const finalizeStream = useCallback(
    (id: number, s: ThreadStream) => {
      streamsRef.current.delete(id);
      const text = s.done ? s.done.text : stripFix(s.shown).trim();
      if (id === activeIdRef.current && (text || s.tools.length > 0)) {
        setMessages((m) => [
          ...m,
          {
            role: "ai",
            text,
            fix: s.done?.fix ?? null,
            tools: s.tools,
            partial: s.done?.partial,
          },
        ]);
      }
      syncLive();
      syncView();
      void refreshThreads();
    },
    [syncLive, syncView, refreshThreads],
  );

  /** 打字机:一个定时器喂所有会话的流;没有流时自动停表 */
  const ensureTimer = useCallback(() => {
    if (timerRef.current !== null) return;
    timerRef.current = window.setInterval(() => {
      const streams = streamsRef.current;
      if (streams.size === 0) {
        window.clearInterval(timerRef.current!);
        timerRef.current = null;
        return;
      }
      for (const [id, s] of [...streams.entries()]) {
        if (s.buffer.length > 0) {
          s.shown += s.buffer.slice(0, CHARS_PER_TICK);
          s.buffer = s.buffer.slice(CHARS_PER_TICK);
        } else if (s.ended) {
          finalizeStream(id, s); // 收尾自带一次 syncView
        }
      }
      // 只有当前会话在流式时才推视图,后台会话不引起重渲染
      if (activeIdRef.current !== null && streamsRef.current.has(activeIdRef.current)) {
        syncView();
        scrollToBottom();
      }
    }, TICK_MS);
  }, [finalizeStream, syncView, scrollToBottom]);

  /** 取(或建)某会话的流桶 */
  const streamFor = useCallback(
    (id: number): ThreadStream => {
      let s = streamsRef.current.get(id);
      if (!s) {
        s = { buffer: "", shown: "", tools: [], activity: null, ended: false, done: null };
        streamsRef.current.set(id, s);
        syncLive();
        ensureTimer();
      }
      return s;
    },
    [syncLive, ensureTimer],
  );

  // 流事件订阅(常驻;按 thread_id 分桶,不再受"当前会话"影响)
  useEffect(() => {
    let cancelled = false;
    const unlistens: UnlistenFn[] = [];
    void (async () => {
      unlistens.push(
        await events.onChatChunk((p) => {
          if (cancelled) return;
          streamFor(p.thread_id).buffer += p.text;
        }),
        await events.onChatTool((p) => {
          if (cancelled) return;
          const s = streamFor(p.thread_id);
          if (p.status === "completed") {
            s.tools = [...s.tools, p.label];
            s.activity = null;
          } else if (p.status === "error") {
            s.activity = null;
          } else {
            s.activity = p.label;
          }
          syncView();
        }),
        await events.onChatDone((p) => {
          if (cancelled) return;
          const s = streamFor(p.thread_id);
          s.done = p;
          s.ended = true;
        }),
        await events.onChatError((p) => {
          if (cancelled) return;
          const s = streamsRef.current.get(p.thread_id);
          if (s) s.ended = true;
          setErrors((e) => ({
            ...e,
            [p.thread_id]:
              p.retry_after_secs !== null
                ? `触发限速,等 ${p.retry_after_secs} 秒再发一次`
                : p.message,
          }));
        }),
      );
      // 订阅之后再问「谁还在生成」:离开本页又回来时恢复生成中指示。
      // 顺序不能反 —— 先订阅才不会漏掉查询与订阅之间结束的那一条。
      try {
        for (const id of await ipc.chatActiveThreads()) {
          if (!cancelled) streamFor(id);
        }
      } catch {
        /* 拿不到就算了,回复仍会落库 */
      }
    })();
    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [streamFor, syncView]);

  /** 打开一个会话(读历史;进行中的流原样接着显示) */
  const openThread = useCallback(
    async (t: ChatThreadInfo) => {
      setActive(t);
      activeIdRef.current = t.id;
      setPickingRole(false);
      setAgentSetup(false);
      syncView();
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
    [scrollToBottom, toast, syncView],
  );

  // 首次载入:自动打开当前模式最近的会话(prefill 流程自己接管)
  const initializedRef = useRef(false);
  useEffect(() => {
    if (initializedRef.current || prefill) return;
    if (active || pickingRole || agentSetup) {
      initializedRef.current = true;
      return;
    }
    if (threads.length === 0) return;
    initializedRef.current = true;
    const latest = threads.find((t) => t.mode === mode);
    if (latest) void openThread(latest);
  }, [threads, mode, prefill, active, pickingRole, agentSetup, openThread]);

  /** 模式切换:自动打开该模式最近的会话(不打断别的会话的流) */
  const switchMode = useCallback(
    (next: ChatMode) => {
      setMode(next);
      setPickingRole(false);
      setAgentSetup(false);
      const latest = threads.find((t) => t.mode === next);
      if (latest) {
        void openThread(latest);
      } else {
        setActive(null);
        activeIdRef.current = null;
        setMessages([]);
        syncView();
        if (next === "roleplay") setPickingRole(true);
        if (next === "agent") setAgentSetup(true);
      }
    },
    [threads, openThread, syncView],
  );

  /** 新对话入口(按模式进入对应的创建流) */
  const newThread = useCallback(() => {
    setActive(null);
    activeIdRef.current = null;
    setMessages([]);
    syncView();
    setPickingRole(mode === "roleplay");
    setAgentSetup(mode === "agent");
    setAgentDir(null);
  }, [mode, syncView]);

  /** 建会话后把"还没建会话时先选好的模型"补上 */
  const applyPending = useCallback(
    async (t: ChatThreadInfo): Promise<ChatThreadInfo> => {
      if (!pending) return t;
      try {
        const updated = await ipc.chatThreadSetModel(
          t.id,
          pending.channel,
          pending.model,
          pending.label,
        );
        setPending(null);
        return updated;
      } catch {
        return t;
      }
    },
    [pending],
  );

  const createRoleplayThread = useCallback(
    async (card: Pick<RoleCard, "id" | "name" | "system" | "opener">) => {
      try {
        const created = await ipc.chatThreadCreate({
          mode: "roleplay",
          title: card.name,
          roleId: card.id,
          roleSystem: card.system,
          opener: card.opener,
        });
        const t = await applyPending(created);
        await refreshThreads();
        await openThread(t);
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [refreshThreads, openThread, toast, applyPending],
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

  const send = useCallback(
    async (raw?: string) => {
      const text = (raw ?? input).trim();
      const busyHere = active !== null && liveIds.includes(active.id);
      if (!text || busyHere) return;
      let thread = active;
      try {
        if (!thread) {
          if (mode !== "free") return; // roleplay/agent 必先建会话
          const created = await ipc.chatThreadCreate({
            mode: "free",
            title: text.slice(0, 20),
          });
          thread = await applyPending(created);
          setActive(thread);
          activeIdRef.current = thread.id;
          void refreshThreads();
        }
        const id = thread.id;
        setErrors((e) => {
          const next = { ...e };
          delete next[id];
          return next;
        });
        setMessages((m) => [...m, { role: "user", text, fix: null, tools: [] }]);
        setInput("");
        streamFor(id); // 先亮出"正在思考",再发请求
        syncView();
        window.setTimeout(scrollToBottom, 0);
        if (mode === "agent") {
          await ipc.agentSend(id, text);
        } else {
          await ipc.chatSend(id, text, fixEnabled);
        }
      } catch (e) {
        const id = thread?.id;
        if (id !== undefined) {
          streamsRef.current.delete(id);
          syncLive();
          syncView();
        }
        const err = e as CmdError;
        const message =
          err.code === "no_channel"
            ? "尚未配置 AI 通道 — 在「设置 · AI 接入」里选择一个"
            : err.code === "not_installed"
              ? "未找到 opencode — 到「设置 · AI 接入」一键安装后再来"
              : String(err.message ?? e);
        if (id !== undefined) setErrors((prev) => ({ ...prev, [id]: message }));
        else toast.show(message);
      }
    },
    [
      input,
      active,
      liveIds,
      mode,
      fixEnabled,
      applyPending,
      refreshThreads,
      streamFor,
      syncLive,
      syncView,
      scrollToBottom,
      toast,
    ],
  );

  const stop = useCallback(() => {
    if (active) void ipc.chatStop(active.id);
  }, [active]);

  const confirmDelete = useCallback(
    async (t: ChatThreadInfo, alsoFolder: boolean) => {
      setDeleting(null);
      try {
        const outcome = await ipc.chatThreadDelete(t.id, alsoFolder);
        streamsRef.current.delete(t.id);
        syncLive();
        setErrors((e) => {
          const next = { ...e };
          delete next[t.id];
          return next;
        });
        if (active?.id === t.id) {
          setActive(null);
          activeIdRef.current = null;
          setMessages([]);
          syncView();
        }
        void refreshThreads();
        toast.show(outcome.note || "已删除这个对话");
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [active, refreshThreads, toast, syncLive, syncView],
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
      const created = await ipc.chatThreadCreate({
        mode: "agent",
        title: `📁 ${base}`,
        workdir: agentDir,
      });
      const t = await applyPending(created);
      await refreshThreads();
      await openThread(t);
    } catch (e) {
      toast.show(String((e as CmdError).message ?? e));
    }
  }, [agentDir, refreshThreads, openThread, toast, applyPending]);

  const toggleFix = useCallback(() => {
    setFixEnabled((v) => {
      localStorage.setItem(FIX_TOGGLE_KEY, v ? "0" : "1");
      return !v;
    });
  }, []);

  /** 模型选择:落到会话上;还没建会话就先记着,建完再补 */
  const chooseModel = useCallback(
    async (pick: { channel: ChannelId; model: string; label: string } | null) => {
      if (!active) {
        setPending(pick);
        toast.show(pick ? `新对话将使用 ${pick.label}` : "新对话跟随设置里的模型");
        return;
      }
      try {
        const updated = await ipc.chatThreadSetModel(
          active.id,
          pick?.channel ?? null,
          pick?.model ?? "",
          pick?.label ?? "",
        );
        setActive(updated);
        void refreshThreads();
        toast.show(pick ? `这个对话改用 ${pick.label}(下一条起)` : "已恢复跟随设置");
      } catch (e) {
        toast.show(String((e as CmdError).message ?? e));
      }
    },
    [active, refreshThreads, toast],
  );

  const probeFor = useCallback(
    (id: ChannelId) => {
      if (statuses[id]) return;
      void ipc
        .probeChannel(id)
        .then((s) => setStatuses((prev) => ({ ...prev, [id]: s })))
        .catch((e) =>
          setStatuses((prev) => ({
            ...prev,
            [id]: { state: "error", message: String((e as CmdError).message ?? e) },
          })),
        );
    },
    [statuses],
  );

  const modeThreads = useMemo(() => threads.filter((t) => t.mode === mode), [threads, mode]);
  const needChannel = mode !== "agent" && !channel;
  const busy = active !== null && liveIds.includes(active.id);
  const activeError = active ? errors[active.id] : undefined;
  const globalLabel =
    settings.ai.model_label ?? (settings.ai.model ? shortModel(settings.ai.model) : "未选择");
  const currentModelLabel = (() => {
    if (active?.model) return active.model_label || shortModel(active.model);
    if (pending && !active) return pending.label;
    if (mode === "agent") {
      return settings.ai.model?.startsWith("opencode/") ? globalLabel : "opencode 默认";
    }
    return globalLabel;
  })();

  return (
    <div className="page page--aichat">
      <header className="page__header">
        <h1>AI 聊天</h1>
        {channel && <span className="workshop-channel">默认 AI:{globalLabel}</span>}
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
                  <span className="aichat-thread__meta">
                    {liveIds.includes(t.id) ? (
                      <span className="aichat-thread__live" title="正在生成回复">
                        ● 生成中
                      </span>
                    ) : (
                      threadDate(t.updated_at)
                    )}
                  </span>
                  <button
                    type="button"
                    className="aichat-thread__del"
                    aria-label="删除对话"
                    title="删除对话"
                    onClick={(e) => {
                      e.stopPropagation();
                      setDeleting(t);
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
                <div className="aichat-bar">
                  <ModelPicker
                    label={currentModelLabel}
                    pinned={Boolean(active?.model) || Boolean(pending && !active)}
                    agentOnly={mode === "agent"}
                    globalLabel={globalLabel}
                    statuses={statuses}
                    onProbe={probeFor}
                    onChoose={(pick) => void chooseModel(pick)}
                  />
                  {mode === "agent" && active && (
                    <span className="aichat-workdir" title={active.workdir}>
                      📁 {active.workdir}
                      <span className="aichat-workdir__warn">
                        AI 可在该文件夹读写文件、执行命令
                      </span>
                    </span>
                  )}
                </div>
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
                      {view.tools.map((t, i) => (
                        <div key={i} className="aichat-tool aichat-tool--done">
                          ⚙ {t}
                        </div>
                      ))}
                      {view.activity && <div className="aichat-tool">⚙ {view.activity}…</div>}
                      {view.text ? (
                        <Markdown text={view.text} />
                      ) : (
                        !view.activity && <span className="aichat-thinking">正在思考…</span>
                      )}
                      <span className="ask-caret" aria-hidden />
                    </div>
                  )}
                  {activeError && <div className="aichat-error">{activeError}</div>}
                </div>

                <div className="aichat-composer">
                  {mode !== "agent" && (
                    <label
                      className="aichat-fixtoggle"
                      title="AI 在每条回复下方指出你这句英文可以怎么说更好"
                    >
                      <input type="checkbox" checked={fixEnabled} onChange={toggleFix} />
                      帮我纠错
                    </label>
                  )}
                  <textarea
                    className="aichat-input"
                    rows={2}
                    value={input}
                    placeholder={
                      mode === "agent"
                        ? "让它帮你做点什么…(Enter 发送)"
                        : "用英文说点什么…(Enter 发送,Shift+Enter 换行)"
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

      <DeleteThreadModal
        thread={deleting}
        onCancel={() => setDeleting(null)}
        onConfirm={(alsoFolder) => deleting && void confirmDelete(deleting, alsoFolder)}
      />
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

/**
 * 每会话模型切换 —— opencode 的 `/model` 命令的可视化版本。
 * 通道按需探测(结果由页面缓存),选中后下一条消息生效。
 */
function ModelPicker({
  label,
  pinned,
  agentOnly,
  globalLabel,
  statuses,
  onProbe,
  onChoose,
}: {
  label: string;
  /** 已为本会话固定模型(非跟随设置) */
  pinned: boolean;
  /** 智能体模式:只能用 opencode 的模型 */
  agentOnly: boolean;
  globalLabel: string;
  statuses: Partial<Record<ChannelId, ChannelStatus>>;
  onProbe: (id: ChannelId) => void;
  onChoose: (pick: { channel: ChannelId; model: string; label: string } | null) => void;
}) {
  const { settings } = useApp();
  const channels: ChannelId[] = agentOnly
    ? ["opencode"]
    : ["opencode", "deepseek", "zen", "ollama"];
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<ChannelId>(
    agentOnly ? "opencode" : (settings.ai.channel ?? "opencode"),
  );
  const popRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    onProbe(tab);
    const onDown = (e: MouseEvent) => {
      if (popRef.current && !popRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, tab, onProbe]);

  const status = statuses[tab];
  const models: ModelInfo[] = status?.state === "ready" ? status.models : [];

  return (
    <div className="aichat-model" ref={popRef}>
      <button
        type="button"
        className={`aichat-model__chip${pinned ? " aichat-model__chip--pinned" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="这个对话使用的模型 —— 点击切换(只影响当前对话)"
      >
        ⚡ {label}
        {pinned && <span className="aichat-model__pin">本对话</span>}
        <span aria-hidden>▾</span>
      </button>
      {open && (
        <div className="aichat-model__pop" role="dialog" aria-label="选择模型">
          <div className="aichat-model__head">为这个对话选模型</div>
          {channels.length > 1 && (
            <div className="aichat-model__tabs">
              {channels.map((c) => (
                <button
                  key={c}
                  type="button"
                  className={`aichat-model__tab${tab === c ? " aichat-model__tab--on" : ""}`}
                  onClick={() => {
                    setTab(c);
                    onProbe(c);
                  }}
                >
                  {CHANNEL_NAMES[c]}
                </button>
              ))}
            </div>
          )}
          <div className="aichat-model__list">
            {pinned && (
              <button
                type="button"
                className="aichat-model__item"
                onClick={() => {
                  onChoose(null);
                  setOpen(false);
                }}
              >
                <span className="aichat-model__name">跟随设置({globalLabel})</span>
              </button>
            )}
            {!status && <div className="aichat-model__hint">检测中…</div>}
            {status?.state === "not_installed" && (
              <div className="aichat-model__hint">这个通道还没准备好 — 去「设置 · AI 接入」</div>
            )}
            {status?.state === "not_authed" && (
              <div className="aichat-model__hint">这个通道还没配置好 — 去「设置 · AI 接入」</div>
            )}
            {status?.state === "error" && (
              <div className="aichat-model__hint">{status.message}</div>
            )}
            {models.map((m) => (
              <button
                key={m.id}
                type="button"
                className="aichat-model__item"
                onClick={() => {
                  onChoose({ channel: tab, model: m.id, label: m.display_name });
                  setOpen(false);
                }}
                title={m.terms_note || undefined}
              >
                <span className="aichat-model__name">{m.display_name}</span>
                <span className="aichat-model__tag">
                  {m.needs_proxy ? "🔒 需代理" : "直连可用"}
                </span>
              </button>
            ))}
          </div>
          <div className="aichat-model__foot">换模型只影响这个对话,下一条消息起生效。</div>
        </div>
      )}
    </div>
  );
}

/** 删除对话确认;智能体会话可勾选连带清理工作文件夹(移入回收站) */
function DeleteThreadModal({
  thread,
  onCancel,
  onConfirm,
}: {
  thread: ChatThreadInfo | null;
  onCancel: () => void;
  onConfirm: (alsoFolder: boolean) => void;
}) {
  const [alsoFolder, setAlsoFolder] = useState(false);

  useEffect(() => {
    setAlsoFolder(false); // 每次打开都从"不删文件夹"开始
  }, [thread]);

  if (!thread) return null;
  const canFolder = thread.mode === "agent" && Boolean(thread.workdir);
  return (
    <Modal open title="删除这个对话?" onClose={onCancel}>
      <p className="aichat-del__text">
        「{thread.title}」的聊天记录会被清除,无法恢复。
      </p>
      {canFolder && (
        <label className="aichat-del__opt">
          <input
            type="checkbox"
            checked={alsoFolder}
            onChange={(e) => setAlsoFolder(e.target.checked)}
          />
          <span>
            <b>同时清理工作文件夹</b>
            <code className="aichat-del__path">{thread.workdir}</code>
            <em className="aichat-del__warn">
              ⚠ 文件夹连同里面的所有文件会被移入系统回收站(还能从回收站找回)。
              请先确认里面没有你还需要的资料。
            </em>
          </span>
        </label>
      )}
      <div className="aichat-del__actions">
        <Button variant="ghost" onClick={onCancel}>
          取消
        </Button>
        <Button className="aichat-del__danger" onClick={() => onConfirm(alsoFolder)}>
          {alsoFolder ? "删除对话并清理文件夹" : "删除对话"}
        </Button>
      </div>
    </Modal>
  );
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
        ⚠ AI 将可以在该文件夹内<b>读写文件、执行命令</b>。
        请为它单独准备一个文件夹,不要选桌面、系统盘根目录或含重要资料的目录。
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
