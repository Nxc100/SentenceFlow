/**
 * 生成工坊(§4.4/§6.3):无通道 → 引导卡;就绪 → 工作台
 * (场景 + 等级 + 句数 → CostBar → 流式句卡 → 摘要)。
 * 队列进度点、[停止]即停表保留已产出、[续跑]。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Markdown, ProgressBar, levelOptionLabel, useToast } from "@sentenceflow/ui";
import type { LevelId, Sentence } from "@sentenceflow/ui";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useApp } from "../appState";
import type { BatchState, GenJob, WorkshopCard } from "../ipc";
import { events, ipc } from "../ipc";

type Card =
  | { key: number; kind: "accepted"; sentence: Sentence }
  | { key: number; kind: "discarded"; en: string; reason: string; recoverable: boolean; sentence?: Sentence };

export function WorkshopPage({
  prefillScene,
  onConsumedPrefill,
}: {
  prefillScene: string | null;
  onConsumedPrefill: () => void;
}) {
  const { level, specs, settings } = useApp();
  const toast = useToast();
  const channel = settings.ai.channel;
  const [scene, setScene] = useState("");
  const [genLevel, setGenLevel] = useState<LevelId>(level);
  const [count, setCount] = useState<10 | 20 | 30>(10);
  const [running, setRunning] = useState(false);
  const [cards, setCards] = useState<Card[]>([]);
  const [batches, setBatches] = useState<BatchState[]>([]);
  const [produced, setProduced] = useState(0);
  const [summary, setSummary] = useState<string | null>(null);
  const [backoff, setBackoff] = useState<number | null>(null);
  const [suggestSwitch, setSuggestSwitch] = useState(false);
  const [cost, setCost] = useState<number | null>(null);
  const [costWarn, setCostWarn] = useState(false);
  const [inlineError, setInlineError] = useState<string | null>(null);
  const [pausedJobs, setPausedJobs] = useState<GenJob[]>([]);
  const [spendToday, setSpendToday] = useState(0);
  const keyRef = useRef(0);

  useEffect(() => {
    if (prefillScene) {
      setScene(prefillScene);
      onConsumedPrefill();
    }
  }, [prefillScene, onConsumedPrefill]);

  const refreshJobs = useCallback(async () => {
    const jobs = await ipc.workshopJobs();
    setPausedJobs(jobs.filter((j) => j.state === "paused"));
    const spend = await ipc.spendSummary();
    setSpendToday(spend.today_requests);
  }, []);

  useEffect(() => {
    void refreshJobs();
  }, [refreshJobs]);

  // 事件订阅
  useEffect(() => {
    let cancelled = false;
    const unlistens: UnlistenFn[] = [];
    void (async () => {
      unlistens.push(
        await events.onWorkshopCard((card: WorkshopCard) => {
          if (cancelled) return;
          keyRef.current += 1;
          if (card.kind === "accepted") {
            setCards((c) => [...c, { key: keyRef.current, kind: "accepted", sentence: card.sentence }]);
          } else if (card.kind === "discarded") {
            setCards((c) => [
              ...c,
              {
                key: keyRef.current,
                kind: "discarded",
                en: card.en,
                reason: card.reason,
                recoverable: card.recoverable,
              },
            ]);
          }
        }),
        await events.onWorkshopProgress((p) => {
          if (cancelled) return;
          setBatches(p.batches);
          setProduced(p.produced);
        }),
        await events.onWorkshopBackoff((p) => {
          if (cancelled) return;
          setBackoff(p.wait_secs);
          setSuggestSwitch(p.suggest_switch);
          window.setTimeout(() => setBackoff(null), p.wait_secs * 1000);
        }),
        await events.onWorkshopMeter((p) => {
          if (cancelled) return;
          setCost(p.cost_cny);
          setCostWarn(p.warning);
        }),
        await events.onWorkshopDone((p) => {
          if (cancelled) return;
          setRunning(false);
          setSummary(p.summary);
          void refreshJobs();
        }),
        await events.onWorkshopError((p) => {
          if (cancelled) return;
          setInlineError(p.message);
        }),
      );
    })();
    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, [refreshJobs]);

  if (!channel) {
    return <GuideCard />;
  }

  const start = async () => {
    if (!scene.trim()) {
      toast.show("先描述一个场景,比如「下周出差要用的机场句子」");
      return;
    }
    setCards([]);
    setSummary(null);
    setInlineError(null);
    setProduced(0);
    setBatches([]);
    setRunning(true);
    try {
      await ipc.workshopStart({
        scene: scene.trim(),
        level: genLevel,
        total_sentences: count,
        microbatch: Math.min(count, 10),
        channel,
        model: settings.ai.model ?? "",
      });
    } catch (e) {
      setRunning(false);
      setInlineError(String((e as { message?: string }).message ?? e));
    }
  };

  const isFree = channel === "opencode" || channel === "zen" || channel === "ollama";

  return (
    <div className="page page--workshop">
      <header className="page__header">
        <h1>生成工坊</h1>
        <span className="workshop-channel">
          AI:{settings.ai.model_label ?? settings.ai.model?.split("/").pop() ?? channel}
        </span>
      </header>

      <div className="workshop-form">
        <input
          className="workshop-scene"
          placeholder="场景,如:机场值机和安检"
          value={scene}
          onChange={(e) => setScene(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !running) void start();
          }}
        />
        <select value={genLevel} onChange={(e) => setGenLevel(e.target.value as LevelId)}>
          {specs.map((s) => (
            <option key={s.id} value={s.id}>
              {levelOptionLabel(s.id, s)}
            </option>
          ))}
        </select>
        <div className="workshop-count">
          {([10, 20, 30] as const).map((n) => (
            <button
              key={n}
              type="button"
              className={`sf-chip${count === n ? " sf-chip--on" : ""}`}
              onClick={() => setCount(n)}
            >
              {n}
            </button>
          ))}
        </div>
        {running ? (
          <Button variant="secondary" onClick={() => void ipc.workshopStop()}>
            停止
          </Button>
        ) : (
          <Button onClick={() => void start()}>生成 →</Button>
        )}
      </div>

      {/* CostBar(§5.5 双形态) */}
      <div className={`costbar${costWarn || backoff !== null ? " costbar--warn" : ""}`}>
        {isFree ? (
          backoff !== null ? (
            <span>免费额度限速,{backoff}s 后自动继续{suggestSwitch && " · 建议切换通道"}</span>
          ) : (
            <>
              <span>免费额度</span>
              <span className="costbar__right">今日 {spendToday} 次请求</span>
            </>
          )
        ) : (
          <>
            <span>🪙 预算上限 ¥{settings.ai.per_run_budget_cny.toFixed(2)}</span>
            <span className="costbar__right">
              {cost !== null ? `已用 ≈ ¥${cost.toFixed(3)}` : "以官方账单为准"}
            </span>
          </>
        )}
      </div>

      {batches.length > 0 && (
        <div className="workshop-queue">
          任务:
          {batches.map((b, i) => (
            <span key={i} className={`queue-dot queue-dot--${b}`} />
          ))}
          <span className="workshop-produced">已入库 {produced}</span>
        </div>
      )}
      {running && <ProgressBar value={batches.length ? batches.filter((b) => b === "done").length / batches.length : 0} />}

      {inlineError && <div className="workshop-error">{inlineError}</div>}

      <div className="workshop-cards">
        {cards.map((card) =>
          card.kind === "accepted" ? (
            <div key={card.key} className="gen-card gen-card--ok">
              <span className="gen-card__badge">✓</span>
              <span className="gen-card__en">{card.sentence.en}</span>
              <span className="gen-card__zh">{card.sentence.zh}</span>
            </div>
          ) : (
            <details key={card.key} className="gen-card gen-card--discard">
              <summary>
                <span className="gen-card__badge">✕</span>
                <span className="gen-card__en">{card.en || "(解析失败)"}</span>
              </summary>
              <div className="gen-card__reason">{card.reason}</div>
            </details>
          ),
        )}
      </div>

      {summary && <div className="workshop-summary">{summary} · 已通过句子已自动入库,可去内容库开练</div>}

      {pausedJobs.length > 0 && (
        <div className="workshop-paused">
          {pausedJobs.map((j) => (
            <div key={j.job_id} className="workshop-paused__row">
              <span>
                中断任务:{j.params.scene}({j.produced}/{j.params.total_sentences})
              </span>
              <Button
                variant="ghost"
                onClick={async () => {
                  setRunning(true);
                  setSummary(null);
                  await ipc.workshopResume(j.job_id);
                }}
              >
                续跑
              </Button>
            </div>
          ))}
        </div>
      )}

      {/* 周点评入口(§4.4) */}
      <WeeklyReview />
      <p className="workshop-note">
        每次生成前先在本地校验(词表 / 成分 / 音标 / 查重),不合格的句子不会进入你的句库。
      </p>
    </div>
  );
}

function GuideCard() {
  return (
    <div className="page page--workshop">
      <h1>生成工坊</h1>
      <div className="workshop-guide">
        <h2>为任何场景生成专属练习</h2>
        <p>接入任一 AI 通道后,这里可以为「下周出差」「面试」等场景现场生成带全套解析的句集。</p>
        <table className="workshop-guide__table">
          <tbody>
            <tr><td>opencode 本地</td><td>一键安装即免费用(限速,无需登录)</td></tr>
            <tr><td>DeepSeek 官方</td><td>自己的 Key,质量稳定,按量计费</td></tr>
            <tr><td>Zen 直连</td><td>不装 CLI 也能用免费模型</td></tr>
            <tr><td>Ollama 本地</td><td>全离线,本机模型</td></tr>
          </tbody>
        </table>
        <p className="workshop-guide__hint">在「设置 · AI 接入」选择通道。<b>不接入也不影响任何学习功能。</b></p>
      </div>
    </div>
  );
}

function WeeklyReview() {
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  return (
    <div className="workshop-weekly">
      <Button
        variant="ghost"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          try {
            setText(await ipc.weeklyReview());
          } catch (e) {
            toast.show(String((e as { message?: string }).message ?? e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "生成中…" : "生成本周 AI 点评"}
      </Button>
      {text && (
        <div className="workshop-weekly__text">
          <Markdown text={text} />
        </div>
      )}
    </div>
  );
}
