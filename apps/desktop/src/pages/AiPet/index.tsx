/**
 * 「AI 萌宠」页 —— 原 HatchDesk 孵化器窗并入句流主窗后的五页签壳。
 *
 * 总开关(`settings.pet.enabled`)默认关:未开启时只显示一张引导卡,
 * 不建宠物窗、不起心跳线程,与整合前的句流行为完全一致。
 *
 * 宠物右键菜单的「打开孵化器 / 设置提醒 / 去孵化」经 `pet://nav` 落到这里切签。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, Switch, useToast } from "@sentenceflow/ui";
import { petEvents, petIpc } from "../../pet/ipc";
import { PetsTab } from "./PetsTab";
import { WizardTab } from "./WizardTab";
import { SpellbookTab } from "./SpellbookTab";
import { RemindersTab } from "./RemindersTab";
import { PetSettingsTab } from "./PetSettingsTab";
import { errText, usePetSettings } from "./usePetSettings";

export type PetTab = "wizard" | "pets" | "spellbook" | "reminders" | "settings";

const TABS: Array<{ key: PetTab; label: string }> = [
  { key: "wizard", label: "孵化向导" },
  { key: "pets", label: "我的宠物" },
  { key: "spellbook", label: "咒语包" },
  { key: "reminders", label: "提醒 · 番茄钟" },
  { key: "settings", label: "宠物设置" },
];

/**
 * 离开页面多久后丢掉向导会话。
 *
 * 向导的素材暂存区是**未压缩的 RGBA 帧**,一段视频进化下来可达数十 MB,
 * 一直挂在进程里没有意义。切走后 5 分钟内回来仍在原处;超时才清。
 */
const SESSION_TTL_MS = 5 * 60_000;

/**
 * 待执行的会话清理计时器。
 *
 * **必须放在模块级**:计时器是在组件卸载时排的,而取消它的是**下一次挂载** ——
 * 用 `useRef` 的话新实例拿到的是一只新 ref,取消不掉上一实例排下的清理,
 * 用户切回来接着编辑,五分钟后素材会凭空消失。
 */
let sessionResetTimer: number | undefined;

export function AiPetPage() {
  const { settings, patch, error } = usePetSettings();
  const [tab, setTab] = useState<PetTab>("wizard");
  const { show: toast } = useToast();

  // 宠物右键菜单 → pet://nav → 页内切签
  useEffect(() => {
    const un = petEvents.onNav((target) => {
      if (TABS.some((t) => t.key === target)) setTab(target as PetTab);
    });
    return () => void un.then((f) => f());
  }, []);

  // 向导的素材暂存区是未压缩 RGBA 帧,一段视频进化下来可达数十 MB。
  // 进页面先撤销待执行的清理;离开 5 分钟后再没回来才真的释放。
  useEffect(() => {
    window.clearTimeout(sessionResetTimer);
    return () => {
      sessionResetTimer = window.setTimeout(() => {
        void petIpc.wizardReset().catch(() => {
          /* 会话已被清或后端不在:无所谓 */
        });
      }, SESSION_TTL_MS);
    };
  }, []);

  const toggleEnabled = useCallback(
    async (on: boolean) => {
      const saved = await patch({ enabled: on });
      if (saved) toast(on ? "AI 萌宠已开启,宠物这就上桌面" : "已关闭,宠物窗已收起");
    },
    [patch, toast],
  );

  if (!settings) {
    return (
      <div className="page page--aipet">
        <header className="page__header">
          <h1>AI 萌宠</h1>
        </header>
        <p className="aipet-loading">{error ? `载入失败:${error}` : "载入中…"}</p>
      </div>
    );
  }

  return (
    <div className="page page--aipet">
      <header className="page__header">
        <h1>AI 萌宠</h1>
        <span className="aipet-master">
          <span className="aipet-master__label">桌面宠物</span>
          <Switch checked={settings.enabled} onChange={(v) => void toggleEnabled(v)} />
        </span>
      </header>

      {!settings.enabled && <IntroCard onEnable={() => void toggleEnabled(true)} />}

      <nav className="aipet-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            aria-selected={tab === t.key}
            className={`aipet-tab${tab === t.key ? " aipet-tab--on" : ""}`}
            onClick={() => setTab(t.key)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      <div className="aipet-body">
        {tab === "wizard" && <WizardTab onGoto={setTab} />}
        {tab === "pets" && <PetsTab onGoto={setTab} />}
        {tab === "spellbook" && <SpellbookTab onGoto={setTab} />}
        {tab === "reminders" && <RemindersTab />}
        {tab === "settings" && <PetSettingsTab />}
      </div>

      {error && <div className="aipet-note aipet-note--err">{errText(error)}</div>}
    </div>
  );
}

/** 总开关关闭时的引导卡:先说清「开了会发生什么」,再让用户决定。 */
function IntroCard({ onEnable }: { onEnable: () => void }) {
  return (
    <div className="aipet-intro">
      <div className="aipet-intro__egg" aria-hidden>
        🥚
      </div>
      <div className="aipet-intro__text">
        <h2>拖一张图,让它活在你的桌面上</h2>
        <p>
          自家猫猫狗狗的照片、随手画的角色都行 —— 本机抠图、本机做动画,
          全程不联网、不上传。开启后会多出一个透明的置顶宠物窗,
          只有宠物本体挡鼠标,其余地方点得穿。
        </p>
        <p className="aipet-intro__meta">
          它还能替你管提醒和番茄钟:到点抱着闹钟蹦过来。不想要了随时关掉,句流其余功能完全不受影响。
        </p>
        <Button onClick={onEnable}>开启 AI 萌宠</Button>
      </div>
    </div>
  );
}
