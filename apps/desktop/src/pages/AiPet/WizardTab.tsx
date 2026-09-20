/**
 * 孵化向导:先孵化,后进化。
 *
 * * 还没有宠物 → 「拖一张图」一步到位(L0 单帧,约 60 秒上桌面);
 * * 已有宠物 → 进化面板:核心进化(1 张网格图学会待机+走路)/ 视频进化
 *   (一次生成、多状态点亮)/ 完全体(追加网格)/ 专家模式(逐状态工作台)。
 *
 * 等待素材期间可开下载目录监听(默认关,见「宠物设置」),生成图落地即自动导入。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, useToast } from "@sentenceflow/ui";
import { petEvents, petIpc } from "../../pet/ipc";
import type {
  ActivePetAssets,
  GridSpell,
  PlatformSpells,
  Report,
  SessionView,
  SpellbookBundle,
  StateKey,
  StripView,
} from "../../pet/types";
import { ALL_STATES, STATE_NAMES } from "../../pet/types";
import { HatchStage, HatchTheater } from "./HatchTheater";
import {
  DropZone,
  IMG_EXTS,
  Progress,
  ReportView,
  SectionHead,
  VIDEO_EXTS,
  Warnings,
  bgName,
  copyText,
} from "./common";
import type { PetTab } from "./index";
import { errText, usePetSettings } from "./usePetSettings";

export function WizardTab({ onGoto }: { onGoto: (t: PetTab) => void }) {
  const [assets, setAssets] = useState<ActivePetAssets | null | undefined>(undefined);
  const [book, setBook] = useState<SpellbookBundle | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setAssets(await petIpc.activeAssets());
    } catch (e) {
      setError(errText(e));
      setAssets(null);
    }
  }, []);

  useEffect(() => {
    void reload();
    petIpc.spellbookGet().then(setBook).catch((e) => setError(errText(e)));
    const un = petEvents.onChanged(() => void reload());
    return () => void un.then((f) => f());
  }, [reload]);

  if (error) return <div className="aipet-note aipet-note--err">{error}</div>;
  if (assets === undefined || !book) return <p className="aipet-loading">载入中…</p>;

  return assets ? (
    <EvolvePanel assets={assets} book={book} onReload={reload} onGoto={onGoto} />
  ) : (
    <FirstHatch onHatched={reload} onGoto={onGoto} />
  );
}

/* ================================================================ 首次孵化 */

function FirstHatch({
  onHatched,
  onGoto,
}: {
  onHatched: () => void;
  onGoto: (t: PetTab) => void;
}) {
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [name, setName] = useState("");
  const [autoTol, setAutoTol] = useState<number | null>(null);
  const [tol, setTol] = useState<number | null>(null); // null = 跟随自动
  const [busy, setBusy] = useState(false);
  /** 非空时正在演登场动画;演完才 onHatched() 跳走 */
  const [debut, setDebut] = useState<{ name: string; thumb: string | null } | null>(null);
  const { show: toast } = useToast();

  const refresh = useCallback(
    async (p: string, manualTol: number | null) => {
      setBusy(true);
      try {
        const r = await petIpc.l0Preview(p, manualTol ?? undefined);
        setPreview(r.preview);
        setWarnings(r.warnings);
        setAutoTol(r.tolerance);
      } catch (e) {
        toast(errText(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [toast],
  );

  const pick = useCallback(
    (p: string) => {
      setPath(p);
      setReport(null);
      void refresh(p, tol);
    },
    [refresh, tol],
  );

  const hatch = useCallback(async () => {
    if (!path) {
      toast("先选一张图", "error");
      return;
    }
    if (!name.trim()) {
      toast("先给宠物起个名字", "error");
      return;
    }
    setBusy(true);
    try {
      const result = await petIpc.l0Hatch(path, tol ?? autoTol, name.trim());
      if (result.ok) {
        // 签名时刻:先把登场动画演完,再跳去下一步 —— 这是用户等了几十秒的回报
        let thumb: string | null = null;
        try {
          thumb = result.petId ? await petIpc.thumb(result.petId) : null;
        } catch {
          /* 缩略图拿不到就用 🐣 兜底,不能让动画因此不演 */
        }
        setDebut({ name: name.trim(), thumb });
      } else {
        setReport(result.report);
        toast("素材未通过校验,看看下方提示", "error");
      }
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      setBusy(false);
    }
  }, [path, name, tol, autoTol, toast]);

  return (
    <div className="aipet-panels">
      {debut && (
        <HatchTheater
          name={debut.name}
          thumb={debut.thumb}
          onDone={() => {
            setDebut(null);
            onHatched();
          }}
        />
      )}
      <HatchStage />
      <section className="aipet-panel aipet-hero">
        <div className="aipet-hero__egg" aria-hidden>
          🥚
        </div>
        <div>
          <h2>拖图进来,孵一只桌宠</h2>
          <p>
            自家猫猫狗狗的照片、随手画的角色都行;复杂背景会自动用本地智能抠图分离。
            第 1 分钟它就活在桌面上,之后每次进化只要几分钟。
          </p>
        </div>
      </section>

      <section className="aipet-panel">
        <div className="aipet-l0">
          <div className="aipet-l0__left">
            <DropZone
              icon="🖼️"
              title="拖一张图进来"
              hint="或点这里选择文件"
              busy={busy}
              onFiles={(paths) => {
                const first = paths[0];
                if (first) pick(first);
              }}
              onPick={async () => {
                const picked = await petIpc.pickFile("选择一张图", "图片", IMG_EXTS);
                if (picked) pick(picked);
              }}
            />
            <details className="aipet-details">
              <summary>高级:抠图容差</summary>
              <div className="aipet-form__row">
                <input
                  type="range"
                  min={0}
                  max={100}
                  value={tol ?? Math.round(autoTol ?? 40)}
                  onChange={(e) => setTol(Number(e.target.value))}
                  onMouseUp={() => path && void refresh(path, tol)}
                  onTouchEnd={() => path && void refresh(path, tol)}
                />
                <span className="aipet-unit">
                  {tol === null
                    ? `容差(自动 ${Math.round(autoTol ?? 0)})`
                    : `容差 ${tol}`}
                </span>
                {tol !== null && (
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setTol(null);
                      if (path) void refresh(path, null);
                    }}
                  >
                    恢复自动
                  </Button>
                )}
              </div>
              <p className="aipet-desc">默认自动估计;背景抠不净调高,主体被误删调低。</p>
            </details>
            <Warnings items={warnings} />
            <ReportView report={report} />
          </div>

          <div className="aipet-l0__right">
            <div className="aipet-l0__preview">
              {preview ? (
                <img className="aipet-checker" src={preview} alt="抠图预览" />
              ) : (
                <span className="aipet-empty">预览区</span>
              )}
            </div>
            <input
              className="aipet-input"
              placeholder="给它起个名字(必填)"
              maxLength={12}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <Button disabled={busy || !path} onClick={() => void hatch()}>
              🥚 孵化!
            </Button>
          </div>
        </div>
      </section>

      <p className="aipet-note">
        💡 已经有 .petkit 宠物包?去
        <button type="button" className="aipet-link" onClick={() => onGoto("pets")}>
          我的宠物
        </button>
        直接导入。
      </p>
    </div>
  );
}

/* ================================================================ 进化面板 */

function EvolvePanel({
  assets,
  book,
  onReload,
  onGoto,
}: {
  assets: ActivePetAssets;
  book: SpellbookBundle;
  onReload: () => void;
  onGoto: (t: PetTab) => void;
}) {
  const spec = assets.spec;
  const [platformId, setPlatformId] = useState(book.default_platform);
  const platform = useMemo(
    () => book.platforms.find((p) => p.id === platformId) ?? book.platforms[0]!,
    [book, platformId],
  );
  const primaryPair = useMemo(
    () => platform.grid.find((g) => g.primary) ?? platform.grid[0],
    [platform],
  );
  const hasRealIdle = (spec.states["idle"]?.frames ?? 0) >= 2;
  const hasWalk = !!spec.states["walk-right"];

  return (
    <div className="aipet-panels">
      <section className="aipet-panel">
        <SectionHead
          icon="🐣"
          title={`${spec.name} 的进化面板`}
          desc="每次进化只要 1 次 AI 生成。生成交给你熟悉的平台,孵化在本机完成。"
          right={<span className="aipet-badge">{spec.tier ?? "L0"}</span>}
        />
        <div className="aipet-chips">
          {ALL_STATES.map((s) => {
            const clip = spec.states[s];
            const lit = !!clip && (s !== "idle" || (clip.frames ?? 0) >= 2);
            const mirror = !!clip?.mirrorOf;
            return (
              <span
                key={s}
                className={`aipet-chip${lit ? " aipet-chip--lit" : ""}`}
                title={mirror ? "由右走镜像派生" : undefined}
              >
                {STATE_NAMES[s]}
                {mirror ? "⇄" : ""}
              </span>
            );
          })}
        </div>
        <PlatformPills book={book} current={platform.id} onPick={setPlatformId} />
      </section>

      {primaryPair && (
        <GridEvolveCard
          platform={platform}
          pair={primaryPair}
          done={hasRealIdle && hasWalk}
          onEvolved={onReload}
        />
      )}

      <VideoEvolveCard book={book} platform={platform} note={book.video_note} onEvolved={onReload} />

      <FullBodyCard platform={platform} onEvolved={onReload} />

      <ExpertBench onEvolved={onReload} onGoto={onGoto} platform={platform} />
    </div>
  );
}

function PlatformPills({
  book,
  current,
  onPick,
}: {
  book: SpellbookBundle;
  current: string;
  onPick: (id: string) => void;
}) {
  return (
    <div className="aipet-pills">
      {book.platforms.map((p) => (
        <button
          key={p.id}
          type="button"
          className={`aipet-pill${p.id === current ? " aipet-pill--on" : ""}`}
          onClick={() => onPick(p.id)}
        >
          {p.role === "default" ? `${p.name}(推荐)` : p.name}
        </button>
      ))}
    </div>
  );
}

/* ---------------------------------------------------------------- 核心进化 */

function GridEvolveCard({
  platform,
  pair,
  done,
  onEvolved,
}: {
  platform: PlatformSpells;
  pair: GridSpell;
  done: boolean;
  onEvolved: () => void;
}) {
  const [step, setStep] = useState<1 | 2>(1);
  const [report, setReport] = useState<Report | null>(null);
  const [busy, setBusy] = useState(false);
  const { show: toast } = useToast();

  const doImport = useCallback(
    async (path: string) => {
      setBusy(true);
      setReport(null);
      try {
        await petIpc.wizardAddSheet(path, pair.states, pair.cols);
        const result = await petIpc.wizardEvolve();
        if (result.ok) {
          void petIpc.watcherStop().catch(() => undefined);
          toast(`✨ ${result.unlocked.join("、")}已点亮!去桌面看看它`, "success");
          onEvolved();
        } else {
          setReport(result.report);
        }
      } catch (e) {
        toast(errText(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [pair, onEvolved, toast],
  );

  const watch = useWatcher("image", doImport);

  // 进到第 2 步 = 用户已经去平台生成了 → 开始盯下载目录(设置里关着则自动空转)。
  // 后端监听器是单例,网格卡与视频卡按 kind 各取所需;离开这一步即停,
  // 不让它在用户已经走开之后还盯着下载文件夹。
  useEffect(() => {
    if (step !== 2) return;
    void watch.start();
    return () => watch.stop();
  }, [step, watch.start, watch.stop]);

  const copyAndOpen = useCallback(async () => {
    if (!(await copyText(pair.text))) {
      toast("复制失败:请到「咒语包」页手动复制", "error");
      return;
    }
    let note = "记得把宠物形象图作为参考图一并上传";
    try {
      const refPath = await petIpc.referenceExport();
      note = `参考图已存到:${refPath}(上传时一并附上)`;
    } catch {
      /* 用户取消另存为或还没有 idle 素材:用兜底文案 */
    }
    try {
      await petIpc.openUrl(platform.url);
    } catch {
      /* 打不开浏览器不阻塞流程 */
    }
    toast(`咒语已复制。${note}`, "success");
    setStep(2);
  }, [pair, platform, toast]);

  return (
    <section className={`aipet-panel${done ? " aipet-panel--done" : ""}`}>
      <SectionHead
        icon={done ? "✅" : "🧩"}
        title={done ? "待机 + 走路 · 已解锁" : "核心进化 · 学会走路"}
        desc={
          done
            ? "想更新素材?重新生成一张网格图拖进来即可覆盖。"
            : "只需 1 次 AI 生成:一张 2 行网格图(第 1 行待机、第 2 行走路),拖回来当场学会。"
        }
      />
      {step === 1 ? (
        <div className="aipet-step">
          {!done && <span className="aipet-step__no">第 1 / 2 步</span>}
          <Button onClick={() => void copyAndOpen()}>📋 复制咒语并打开{platform.name}</Button>
          <Button
            variant="ghost"
            onClick={async () => {
              await copyText(pair.purified);
              toast("净化版已复制(平台乱加特效时用)", "success");
            }}
          >
            净化版
          </Button>
          <Button variant="ghost" onClick={() => setStep(2)}>
            已经生成好了,跳过 →
          </Button>
        </div>
      ) : (
        <>
          <div className="aipet-step">
            {!done && <span className="aipet-step__no">第 2 / 2 步</span>}
            <Button variant="ghost" onClick={() => setStep(1)}>
              ← 上一步
            </Button>
            {watch.hint}
          </div>
          <DropZone
            icon="🧩"
            title="把生成的网格图拖回来"
            hint="或点这里选择文件 · 一次出 4 张就挑最好的那张"
            busy={busy}
            onFiles={(paths) => {
              const first = paths[0];
              if (first) void doImport(first);
            }}
            onPick={async () => {
              const picked = await petIpc.pickFile("选择网格图", "图片", IMG_EXTS);
              if (picked) void doImport(picked);
            }}
          />
          <p className="aipet-desc">
            导入后若行序颠倒(第 1 行成了走路):在「专家模式」里一键调换,不用重新生成。
          </p>
          <ReportView report={report} />
        </>
      )}
    </section>
  );
}

/* ---------------------------------------------------------------- 视频进化 */

function VideoEvolveCard({
  book,
  platform,
  note,
  onEvolved,
}: {
  book: SpellbookBundle;
  platform: PlatformSpells;
  note: string;
  onEvolved: () => void;
}) {
  const videoPlatforms = useMemo(() => book.platforms.filter((p) => p.video), [book]);
  const vp = useMemo(
    () => videoPlatforms.find((p) => p.id === platform.id) ?? videoPlatforms[0],
    [videoPlatforms, platform],
  );
  const plans = vp?.video?.plans ?? [];
  const [planId, setPlanId] = useState<string | null>(null);
  const plan = plans.find((p) => p.id === planId) ?? plans[0];

  const [session, setSession] = useState<SessionView | null>(null);
  const [progress, setProgress] = useState<{ pct: number; label: string } | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const { show: toast } = useToast();

  useEffect(() => {
    const un = petEvents.onVideoProgress((p) =>
      setProgress({
        pct: p.total > 0 ? (p.done / p.total) * 100 : 0,
        label: p.total > 1 ? `${p.stage} ${p.done}/${p.total}` : p.stage,
      }),
    );
    return () => void un.then((f) => f());
  }, []);

  const doImport = useCallback(
    async (path: string) => {
      if (!plan) return;
      setProgress({ pct: 0, label: "准备中…" });
      setReport(null);
      try {
        const view = await petIpc.wizardAddVideoMulti(
          path,
          plan.segments.map((s) => ({
            stateKey: s.state,
            start: s.start,
            end: s.end,
            frames: s.frames,
          })),
        );
        setSession(view);
      } catch (e) {
        toast(errText(e), "error");
      } finally {
        setProgress(null);
      }
    },
    [plan, toast],
  );

  const watch = useWatcher("video", doImport);

  const apply = useCallback(async () => {
    try {
      const result = await petIpc.wizardEvolve();
      if (result.ok) {
        void petIpc.watcherStop().catch(() => undefined);
        setSession(null);
        toast(`✨ ${result.unlocked.join("、")}已升级为视频动画!`, "success");
        onEvolved();
      } else {
        setReport(result.report);
      }
    } catch (e) {
      toast(errText(e), "error");
    }
  }, [onEvolved, toast]);

  if (!vp || !plan) return null;

  return (
    <section className="aipet-panel aipet-panel--hero">
      <SectionHead
        icon="🎬"
        title="视频进化 · 一次生成,全部学会"
        desc="让参考图按脚本连着演完待机、行走等动作,拖回 mp4,软件按时间轴自动切段,每段各成一条动画。"
      />
      <p className="aipet-desc">{vp.video?.fullSetup}</p>

      <div className="aipet-pills">
        {plans.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`aipet-pill${p.id === plan.id ? " aipet-pill--on" : ""}`}
            onClick={() => setPlanId(p.id)}
          >
            {p.title}
          </button>
        ))}
      </div>

      <p className="aipet-desc">
        脚本共 {plan.seconds} 秒 · {plan.segments.length} 个动作 · {plan.desc}
      </p>
      <div className="aipet-timeline">
        {plan.segments.map((seg, i) => {
          // 原版标的是 `from–to` 区间而不是单段时长 —— 用户照着脚本演时
          // 需要知道"第几秒该换动作",单看时长还得自己做加法。
          const from = plan.segments.slice(0, i).reduce((s, x) => s + x.seconds, 0);
          return (
            <div
              key={i}
              className="aipet-timeline__seg"
              style={{ flex: seg.seconds }}
              title={seg.desc}
            >
              <span className="aipet-timeline__state">{seg.stateName}</span>
              <span className="aipet-timeline__time">
                {from}–{from + seg.seconds}s
              </span>
            </div>
          );
        })}
      </div>

      {session ? (
        <>
          <div className="aipet-note aipet-note--ok">
            ✅ 已切出:{session.strips.map((s) => s.stateName).join("、")}
          </div>
          {session.strips.map((strip) => (
            <StripPreview key={strip.state} strip={strip} />
          ))}
          <ReportView report={report} />
          <div className="aipet-actions">
            <Button onClick={() => void apply()}>⚡ 应用到宠物</Button>
            {session.strips.some((s) => s.state === "idle") &&
              session.strips.some((s) => s.state === "walk-right") && (
                <Button
                  variant="ghost"
                  onClick={async () => {
                    setSession(await petIpc.wizardSwapStates("idle", "walk-right"));
                    toast("已调换 待机 ⇄ 右走", "success");
                  }}
                >
                  ⇄ 调换 待机/右走
                </Button>
              )}
            <Button variant="ghost" onClick={() => setSession(null)}>
              重新导入
            </Button>
          </div>
        </>
      ) : (
        <>
          <div className="aipet-step">
            <span className="aipet-step__no">第 1 / 2 步</span>
            <Button
              onClick={async () => {
                if (!(await copyText(plan.text))) {
                  toast("复制失败:请到「咒语包」页手动复制", "error");
                  return;
                }
                let note2 = "记得把宠物形象图作为参考图一并上传";
                try {
                  note2 = `参考图已存到:${await petIpc.referenceExport()}`;
                } catch {
                  /* 取消另存为:用兜底文案 */
                }
                try {
                  await petIpc.openUrl(vp.url);
                } catch {
                  /* 忽略 */
                }
                void watch.start();
                toast(`脚本已复制。${note2}`, "success");
              }}
            >
              📋 复制脚本并打开{vp.name}
            </Button>
          </div>
          <div className="aipet-step">
            <span className="aipet-step__no">第 2 / 2 步</span>
            {watch.hint}
          </div>
          <DropZone
            icon="🎬"
            title="把生成的 mp4 拖回来"
            hint="按脚本时间轴自动切段 → 每段独立选帧 → 边界自动吸附到动作之间的停顿处"
            small
            busy={progress !== null}
            onFiles={(paths) => {
              const first = paths[0];
              if (first) void doImport(first);
            }}
            onPick={async () => {
              const picked = await petIpc.pickFile("选择视频", "视频", VIDEO_EXTS);
              if (picked) void doImport(picked);
            }}
          />
          {progress && <Progress pct={progress.pct} label={progress.label} />}
        </>
      )}
      <p className="aipet-note aipet-note--warn">⚠ {note}</p>
    </section>
  );
}

function StripPreview({ strip }: { strip: StripView }) {
  return (
    <div className="aipet-strip">
      <div className="aipet-strip__title">
        {strip.stateName} · {strip.frames} 帧 · 背景:{bgName(strip.background)}
      </div>
      <div className="aipet-thumbs">
        {strip.previews.map((src, i) => (
          <img key={i} className="aipet-thumb aipet-checker" src={src} alt={`第 ${i + 1} 帧`} />
        ))}
      </div>
      <Warnings items={strip.warnings} />
    </div>
  );
}

/* ---------------------------------------------------------------- 完全体 */

function FullBodyCard({
  platform,
  onEvolved,
}: {
  platform: PlatformSpells;
  onEvolved: () => void;
}) {
  const { show: toast } = useToast();
  const extras = platform.grid.filter((g) => !g.primary);
  if (extras.length === 0) return null;
  return (
    <section className="aipet-panel">
      <SectionHead
        icon="👑"
        title="完全体 · 全 8 状态"
        desc="开心/睡觉/吃东西/提醒默认已由道具动效补齐;追求完全体再生成 2 张网格图即可。"
      />
      {extras.map((pair) => (
        <div key={pair.id} className="aipet-step">
          <span className="aipet-step__label">{pair.title}</span>
          <Button
            variant="ghost"
            onClick={async () => {
              await copyText(pair.text);
              try {
                await petIpc.openUrl(platform.url);
              } catch {
                /* 忽略 */
              }
              toast("咒语已复制(记得附参考图)", "success");
            }}
          >
            复制咒语并打开平台
          </Button>
          <Button
            variant="ghost"
            onClick={async () => {
              try {
                const picked = await petIpc.pickFile("选择生成图", "图片", IMG_EXTS);
                if (!picked) return;
                await petIpc.wizardAddSheet(picked, pair.states, pair.cols);
                const result = await petIpc.wizardEvolve();
                if (result.ok) {
                  toast(`✨ ${result.unlocked.join("、")}已点亮!`, "success");
                  onEvolved();
                } else {
                  const first = result.report.findings.find((f) => f.level === "error");
                  toast(first ? `校验未通过:${first.message}` : "校验未通过", "error");
                }
              } catch (e) {
                toast(errText(e), "error");
              }
            }}
          >
            导入生成图
          </Button>
        </div>
      ))}
    </section>
  );
}

/* ---------------------------------------------------------------- 专家模式 */

function ExpertBench({
  platform,
  onEvolved,
  onGoto,
}: {
  platform: PlatformSpells;
  onEvolved: () => void;
  onGoto: (t: PetTab) => void;
}) {
  const [open, setOpen] = useState(false);
  const [session, setSession] = useState<SessionView | null>(null);
  const [compose, setCompose] = useState<{ sheet: string; report: Report } | null>(null);
  const [newName, setNewName] = useState("");
  const { show: toast } = useToast();

  const reload = useCallback(async () => {
    try {
      setSession(await petIpc.wizardSession());
    } catch (e) {
      toast(errText(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    if (open) void reload();
  }, [open, reload]);

  const stripOf = (state: StateKey) => session?.strips.find((s) => s.state === state);
  const hasIdle = !!stripOf("idle");
  const hasStrips = (session?.strips.length ?? 0) > 0;

  return (
    <details className="aipet-panel aipet-details" onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>🛠 专家模式:逐状态工作台(画师 / 发烧友)</summary>
      <p className="aipet-desc">
        逐状态导入单行动画条(颗粒度最细,支持救帧 / 单帧替换 / 指定帧数重切)。容差自动估计。
      </p>

      <div className="aipet-bench">
        <div className="aipet-bench__grid">
          {ALL_STATES.map((state) => (
            <StateCard
              key={state}
              state={state}
              strip={stripOf(state)}
              platform={platform}
              onChanged={reload}
            />
          ))}
        </div>

        <aside className="aipet-bench__side">
          <h4>合成</h4>
          <div className="aipet-l0__preview">
            {compose ? (
              <img className="aipet-checker" src={compose.sheet} alt="图集预览" />
            ) : (
              <span className="aipet-empty">{hasStrips ? "点「生成预览」看图集" : "先导入素材"}</span>
            )}
          </div>
          <Button
            variant="secondary"
            disabled={!hasIdle}
            onClick={async () => {
              try {
                const view = await petIpc.wizardPreview();
                setCompose({ sheet: view.sheet, report: view.report });
              } catch (e) {
                toast(errText(e), "error");
              }
            }}
          >
            生成预览
          </Button>
          <ReportView report={compose?.report ?? null} />
          <Button
            disabled={!hasStrips}
            onClick={async () => {
              try {
                const result = await petIpc.wizardEvolve();
                if (result.ok) {
                  toast(`✨ ${result.unlocked.join("、")}已更新!`, "success");
                  setCompose(null);
                  onEvolved();
                  void reload();
                } else {
                  setCompose({ sheet: compose?.sheet ?? "", report: result.report });
                }
              } catch (e) {
                toast(errText(e), "error");
              }
            }}
          >
            ⚡ 应用到当前宠物
          </Button>

          <input
            className="aipet-input"
            placeholder="孵化为新宠物:名字"
            maxLength={12}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
          <Button
            variant="secondary"
            disabled={!hasIdle || !newName.trim()}
            onClick={async () => {
              try {
                const result = await petIpc.wizardHatch(newName.trim());
                if (result.ok) {
                  toast(`${newName.trim()} 破壳而出!`, "success");
                  setNewName("");
                  onEvolved();
                  void reload();
                } else {
                  setCompose({ sheet: compose?.sheet ?? "", report: result.report });
                }
              } catch (e) {
                toast(errText(e), "error");
              }
            }}
          >
            🐣 孵化为新宠物
          </Button>

          {stripOf("idle") && stripOf("walk-right") && (
            <Button
              variant="ghost"
              onClick={async () => {
                setSession(await petIpc.wizardSwapStates("idle", "walk-right"));
                toast("已调换 待机 ⇄ 右走 的行序", "success");
              }}
            >
              ⇄ 行序调换(待机/右走)
            </Button>
          )}
          <Button variant="ghost" onClick={() => onGoto("spellbook")}>
            去咒语包拿单行咒语
          </Button>
        </aside>
      </div>
    </details>
  );
}

function StateCard({
  state,
  strip,
  platform,
  onChanged,
}: {
  state: StateKey;
  strip: StripView | undefined;
  platform: PlatformSpells;
  onChanged: () => void;
}) {
  const entry = platform.singles.find((s) => s.state === state);
  const [reslice, setReslice] = useState<number | null>(null);
  const { show: toast } = useToast();

  const guard = useCallback(
    async (run: () => Promise<void>) => {
      try {
        await run();
        onChanged();
      } catch (e) {
        toast(errText(e), "error");
      }
    },
    [onChanged, toast],
  );

  return (
    <div className={`aipet-state${strip ? " aipet-state--filled" : ""}`}>
      <div className="aipet-state__head">
        <span className="aipet-state__name">
          {STATE_NAMES[state]} <code>{state}</code>
        </span>
        <span className="aipet-badge">
          {state === "idle" ? "核心" : state === "walk-left" ? "可镜像" : "可选"}
        </span>
      </div>

      {strip ? (
        <>
          <div className="aipet-thumbs">
            {strip.previews.map((src, i) => (
              <img
                key={i}
                className="aipet-thumb aipet-checker"
                src={src}
                alt={`第 ${i + 1} 帧`}
                title={`第 ${i + 1} 帧 · 点击用新图替换(救帧)`}
                onClick={() =>
                  void guard(async () => {
                    const picked = await petIpc.pickFile("选择替换图", "图片", IMG_EXTS);
                    if (!picked) return;
                    await petIpc.wizardReplaceFrame(state, i, picked);
                    toast(`已替换第 ${i + 1} 帧`, "success");
                  })
                }
              />
            ))}
          </div>
          <p className="aipet-desc">
            {strip.frames} 帧(建议 {strip.recommended[0]}–{strip.recommended[1]})· 背景:
            {bgName(strip.background)}
          </p>
          <Warnings items={strip.warnings} />
          <div className="aipet-actions aipet-actions--wrap">
            <Button
              variant="ghost"
              onClick={() =>
                void guard(async () => {
                  const picked = await petIpc.pickFile("选择动画条", "图片", IMG_EXTS);
                  if (!picked) return;
                  await petIpc.wizardAddStrip(state, picked);
                })
              }
            >
              重新导入
            </Button>
            <span className="aipet-inline">
              <input
                className="settings-num"
                type="number"
                min={1}
                max={8}
                placeholder={String(strip.frames)}
                value={reslice ?? ""}
                onChange={(e) => setReslice(e.target.value === "" ? null : Number(e.target.value))}
              />
              <Button
                variant="ghost"
                disabled={!reslice || reslice < 1 || reslice > 8}
                onClick={() =>
                  void guard(async () => {
                    await petIpc.wizardReslice(state, reslice!);
                    toast(`已按 ${reslice} 帧等宽重切`, "success");
                    setReslice(null);
                  })
                }
              >
                指定帧数重切
              </Button>
            </span>
            <Button
              variant="ghost"
              onClick={() =>
                void guard(async () => {
                  const dest = await petIpc.savePath(
                    "导出首帧",
                    `${state}_首帧参考.png`,
                    "PNG",
                    ["png"],
                  );
                  if (!dest) return;
                  await petIpc.wizardExportFrame(state, 0, dest);
                  toast("首帧已导出", "success");
                })
              }
            >
              导出首帧
            </Button>
            {entry && (
              <Button
                variant="ghost"
                onClick={async () => {
                  await copyText(entry.rescue);
                  toast("救帧咒语已复制(附首帧参考图)", "success");
                }}
              >
                救帧咒语
              </Button>
            )}
            <Button
              variant="ghost"
              onClick={() => void guard(async () => void (await petIpc.wizardRemoveStrip(state)))}
            >
              移除
            </Button>
          </div>
        </>
      ) : (
        <DropZone
          icon="➕"
          title="拖入动画条"
          hint="或点击选择文件"
          small
          onFiles={(paths) => {
            const first = paths[0];
            if (first) void guard(() => petIpc.wizardAddStrip(state, first).then(() => undefined));
          }}
          onPick={() =>
            void guard(async () => {
              const picked = await petIpc.pickFile("选择动画条", "图片", IMG_EXTS);
              if (picked) await petIpc.wizardAddStrip(state, picked);
            })
          }
        />
      )}

      {entry && (
        <div className="aipet-actions aipet-actions--wrap">
          <Button
            variant="ghost"
            onClick={async () => {
              await copyText(entry.text);
              toast("单行咒语已复制(附上参考图)", "success");
            }}
          >
            复制咒语
          </Button>
          <Button
            variant="ghost"
            onClick={async () => {
              await copyText(entry.purified);
              toast("净化版已复制", "success");
            }}
          >
            净化版
          </Button>
        </div>
      )}
    </div>
  );
}

/* ---------------------------------------------------------------- 下载监听 */

/**
 * 「去生成期间盯着下载目录」的小状态机。
 *
 * 只在用户真的按了「复制咒语并打开平台」时才接管(避免网格卡与视频卡互相抢监听);
 * 组件卸载时一定停 —— 监听是隐私敏感面,不能因为切了页签就留在开着。
 *
 * `onFile` 存进 ref:导入回调每次渲染都是新函数,直接进依赖数组会让监听不停地
 * 重挂,而它的语义是「一直是最新那个」。
 */
function useWatcher(kind: "image" | "video", onFile: (path: string) => void) {
  const [active, setActive] = useState(false);
  const { settings } = usePetSettings();
  const handlerRef = useRef(onFile);

  useEffect(() => {
    handlerRef.current = onFile;
  }, [onFile]);

  const stop = useCallback(() => {
    setActive(false);
    void petIpc.watcherStop().catch(() => undefined);
  }, []);

  const start = useCallback(async () => {
    if (!settings?.watcher_enabled) return;
    try {
      await petIpc.watcherStart();
      setActive(true);
    } catch {
      /* 监听不可用不影响手动拖入 */
    }
  }, [settings]);

  // 卸载即停:切走页签后不该还在盯着用户的下载目录
  useEffect(() => () => void petIpc.watcherStop().catch(() => undefined), []);

  useEffect(() => {
    if (!active) return;
    const un = petEvents.onWatcherFile((f) => {
      if (f.kind === kind) handlerRef.current(f.path);
    });
    return () => void un.then((fn) => fn());
  }, [active, kind]);

  const hint = settings?.watcher_enabled ? (
    <span className="aipet-desc">
      {active ? "👀 正在盯着下载目录,生成好的文件落地即自动导入" : "开着这个页面去生成也行"}
    </span>
  ) : null;

  return { active, start, stop, hint };
}
