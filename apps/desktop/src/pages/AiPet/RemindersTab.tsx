/**
 * 提醒 · 番茄钟。
 *
 * 到点时走「系统通知 + 宠物 remind 动画 + 气泡」三通道 —— 宠物不止可爱,还有用。
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Switch, useToast } from "@sentenceflow/ui";
import { petEvents, petIpc } from "../../pet/ipc";
import type { PomodoroStatus, Reminder } from "../../pet/types";
import { Orb, SectionHead } from "./common";
import { errText } from "./usePetSettings";

const RING_R = 82;
const CIRC = 2 * Math.PI * RING_R;

export function RemindersTab() {
  const [reminders, setReminders] = useState<Reminder[]>([]);
  const [pomo, setPomo] = useState<PomodoroStatus | null>(null);
  const { show: toast } = useToast();

  const reload = useCallback(() => {
    petIpc.remindersList().then(setReminders).catch(() => setReminders([]));
  }, []);

  useEffect(() => {
    reload();
    petIpc.pomodoroStatus().then(setPomo).catch(() => undefined);
    const un = petEvents.onPomodoro(setPomo);
    return () => void un.then((f) => f());
  }, [reload]);

  // 提醒到点后一次性提醒会被自动停用,列表要跟着刷新
  useEffect(() => {
    const un = petEvents.onReminder(() => reload());
    return () => void un.then((f) => f());
  }, [reload]);

  return (
    <div className="aipet-panels">
      <PomodoroPanel status={pomo} onStatus={setPomo} />
      <CreatePanel onCreated={reload} />

      <section className="aipet-panel">
        <SectionHead icon="📋" title={`全部提醒(${reminders.length})`} />
        {reminders.length === 0 ? (
          <p className="aipet-empty">还没有提醒。宠物正闲着呢。</p>
        ) : (
          <div className="aipet-reminders">
            {reminders.map((r) => (
              <ReminderCard
                key={r.id}
                reminder={r}
                onToggle={async (enabled) => {
                  try {
                    await petIpc.reminderToggle(r.id, enabled);
                    reload();
                  } catch (e) {
                    toast(errText(e), "error");
                  }
                }}
                onDelete={async () => {
                  try {
                    await petIpc.reminderDelete(r.id);
                    reload();
                    toast("已删除", "success");
                  } catch (e) {
                    toast(errText(e), "error");
                  }
                }}
              />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

/* ---------------------------------------------------------------- 番茄钟 */

function PomodoroPanel({
  status,
  onStatus,
}: {
  status: PomodoroStatus | null;
  onStatus: (s: PomodoroStatus) => void;
}) {
  const [work, setWork] = useState(25);
  const [brk, setBrk] = useState(5);
  const [rounds, setRounds] = useState(4);
  const { show: toast } = useToast();

  const running = status?.running ?? false;
  const remain = status?.remainingSecs ?? 0;
  const total = status?.totalSecs ?? 0;
  const offset = running && total > 0 ? CIRC * (1 - remain / total) : CIRC;

  const toggle = useCallback(async () => {
    try {
      const cur = await petIpc.pomodoroStatus();
      onStatus(
        cur.running
          ? await petIpc.pomodoroStop()
          : await petIpc.pomodoroStart({ workMin: work, breakMin: brk, rounds }),
      );
    } catch (e) {
      toast(errText(e), "error");
    }
  }, [work, brk, rounds, onStatus, toast]);

  return (
    <section className="aipet-panel">
      <SectionHead icon="🍅" title="番茄钟" desc="专注结束时宠物会蹦过来叫你起来动一动。" />
      <div
        className={`aipet-pomo${running ? " aipet-pomo--on" : ""}${
          running && status?.phase !== "work" ? " aipet-pomo--break" : ""
        }`}
      >
        {/* 夜空盘:表盘底是一片夜空,月亮咬边、六颗星错峰闪烁(原版 .pomo-sky) */}
        <div className="aipet-pomo__sky" aria-hidden>
          <span className="aipet-pomo__moon" />
          {[0, 1, 2, 3, 4, 5].map((i) => (
            <span key={i} className="aipet-pomo__star" />
          ))}
        </div>
        <svg viewBox="0 0 184 184" className="aipet-pomo__ring" aria-hidden>
          <circle className="aipet-pomo__track" cx="92" cy="92" r={RING_R} />
          <circle
            className="aipet-pomo__prog"
            cx="92"
            cy="92"
            r={RING_R}
            style={{ strokeDasharray: CIRC, strokeDashoffset: offset }}
          />
        </svg>
        <div className="aipet-pomo__center">
          <div className="aipet-pomo__time">{running ? fmtSecs(remain) : "--:--"}</div>
          <div className="aipet-pomo__phase">
            {running ? (status?.phase === "work" ? "🍅 专注中" : "☕ 休息中") : "准备好了吗"}
          </div>
          {running && (
            <div className="aipet-pomo__round">
              第 {status?.round}/{status?.rounds} 轮
            </div>
          )}
        </div>
      </div>
      <div className="aipet-pomo__cfg">
        <label>
          专注
          <input
            className="settings-num"
            type="number"
            min={1}
            max={120}
            value={work}
            disabled={running}
            onChange={(e) => setWork(Number(e.target.value) || 25)}
          />
          分
        </label>
        <label>
          休息
          <input
            className="settings-num"
            type="number"
            min={1}
            max={60}
            value={brk}
            disabled={running}
            onChange={(e) => setBrk(Number(e.target.value) || 5)}
          />
          分
        </label>
        <label>
          轮数
          <input
            className="settings-num"
            type="number"
            min={1}
            max={12}
            value={rounds}
            disabled={running}
            onChange={(e) => setRounds(Number(e.target.value) || 4)}
          />
        </label>
      </div>
      <div className="aipet-actions aipet-actions--center">
        <Button onClick={() => void toggle()}>{running ? "停止" : "开始专注"}</Button>
      </div>
    </section>
  );
}

/* ---------------------------------------------------------------- 新建 */

function CreatePanel({ onCreated }: { onCreated: () => void }) {
  const [title, setTitle] = useState("");
  const [kind, setKind] = useState<"once" | "every">("once");
  const [at, setAt] = useState(() => defaultDateTimeLocal());
  const [minutes, setMinutes] = useState(45);
  const { show: toast } = useToast();

  const submit = useCallback(async () => {
    const t = title.trim();
    if (!t) {
      toast("先写点提醒内容", "error");
      return;
    }
    try {
      if (kind === "once") await petIpc.reminderCreate(t, "once", at);
      else await petIpc.reminderCreate(t, "every", undefined, minutes);
      setTitle("");
      onCreated();
      toast("提醒已添加", "success");
    } catch (e) {
      toast(errText(e), "error");
    }
  }, [title, kind, at, minutes, onCreated, toast]);

  return (
    <section className="aipet-panel">
      <SectionHead
        icon="⏰"
        title="新建提醒"
        desc="常用循环:喝水 45 分钟 / 久坐提醒 60 分钟 / 滴眼药水 120 分钟。"
      />
      <div className="aipet-form">
        <input
          className="aipet-input"
          placeholder="提醒内容,如:喝水 / 起来活动 / 该练句了"
          maxLength={40}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit();
          }}
        />
        <div className="aipet-form__row">
          <select value={kind} onChange={(e) => setKind(e.target.value as "once" | "every")}>
            <option value="once">一次性(指定时间)</option>
            <option value="every">循环(每 N 分钟)</option>
          </select>
          {kind === "once" ? (
            <input
              className="aipet-input"
              type="datetime-local"
              value={at}
              onChange={(e) => setAt(e.target.value)}
            />
          ) : (
            <span className="aipet-inline">
              <input
                className="settings-num"
                type="number"
                min={1}
                max={1440}
                value={minutes}
                onChange={(e) => setMinutes(Number(e.target.value) || 45)}
              />
              分钟一次
            </span>
          )}
          <Button onClick={() => void submit()}>添加提醒</Button>
        </div>
      </div>
    </section>
  );
}

function ReminderCard({
  reminder: r,
  onToggle,
  onDelete,
}: {
  reminder: Reminder;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
}) {
  const desc = useMemo(
    () =>
      r.kind === "once"
        ? `${r.at ? fmtDateTime(r.at) : "-"}${r.lastFired ? " · 已触发" : ""}`
        : `每 ${r.minutes} 分钟`,
    [r],
  );
  return (
    <div className={`aipet-reminder${r.enabled ? "" : " aipet-reminder--off"}`}>
      <span className="aipet-reminder__icon" aria-hidden>
        {r.kind === "once" ? "⏰" : "🔁"}
      </span>
      <div className="aipet-reminder__body">
        <div className="aipet-reminder__title">{r.title}</div>
        <div className="aipet-reminder__desc">{desc}</div>
      </div>
      <Switch checked={r.enabled} onChange={onToggle} aria-label={`启用 ${r.title}`} />
      <Orb icon="🗑" label="删除" kind="danger" onClick={onDelete} />
    </div>
  );
}

/* ---------------------------------------------------------------- 工具 */

const pad = (n: number) => String(n).padStart(2, "0");

/** 默认时间 = 10 分钟后(`datetime-local` 要的是本地时间字面量,不带时区)。 */
function defaultDateTimeLocal(): string {
  const t = new Date(Date.now() + 10 * 60_000);
  return `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}T${pad(t.getHours())}:${pad(t.getMinutes())}`;
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fmtSecs(total: number): string {
  return `${pad(Math.floor(total / 60))}:${pad(Math.floor(total % 60))}`;
}
