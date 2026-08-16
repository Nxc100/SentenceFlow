/**
 * 桌面端外壳:左侧窄栏六项导航(§2.2);练习与完成页全屏隐藏导航。
 * 首启(无 level 设置)进入等级选择流(§6.3,≤60 秒零配置)。
 */

import { useEffect, useState } from "react";
import type { Bootstrap } from "./ipc";
import { ipc } from "./ipc";
import { AppStateProvider, useApp } from "./appState";
import { Onboarding } from "./pages/Onboarding";
import { TodayPage } from "./pages/Today";
import { PracticeScreen } from "./pages/Practice";
import type { PracticeLaunch } from "./pages/Practice";
import { LibraryPage } from "./pages/Library";
import { WorkshopPage } from "./pages/Workshop";
import { ReportPage } from "./pages/Report";
import { SettingsPage } from "./pages/Settings";

export type NavKey = "today" | "library" | "workshop" | "report" | "settings";

const NAV: Array<{ key: NavKey; label: string; icon: string }> = [
  { key: "today", label: "今日", icon: "☀" },
  { key: "library", label: "内容库", icon: "▤" },
  { key: "workshop", label: "生成工坊", icon: "✦" },
  { key: "report", label: "报告", icon: "▦" },
  { key: "settings", label: "设置", icon: "⚙" },
];

export function App() {
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ipc
      .bootstrap()
      .then(setBootstrap)
      .catch((e) => setError(typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : String(e)));
  }, []);

  if (error) {
    return (
      <div className="app-boot-error">
        <h2>启动失败</h2>
        <p>{error}</p>
      </div>
    );
  }
  if (!bootstrap) {
    return <div className="app-boot" aria-busy="true" />;
  }
  return (
    <AppStateProvider bootstrap={bootstrap}>
      <Shell />
    </AppStateProvider>
  );
}

function Shell() {
  const { settings, license } = useApp();
  const [nav, setNav] = useState<NavKey>("today");
  const [practice, setPractice] = useState<PracticeLaunch | null>(null);
  const [workshopPrefill, setWorkshopPrefill] = useState<string | null>(null);

  // 首启:未定级 → 等级选择(通道配置绝不出现在首启路径,§6.3)
  if (settings.level === null) {
    return <Onboarding />;
  }

  // 练习全屏模态(§2.2)
  if (practice) {
    return <PracticeScreen launch={practice} onExit={() => setPractice(null)} />;
  }

  const trialCapsule = (() => {
    if (license.kind === "trial") {
      return (
        <span className={`trial-capsule${license.days_left <= 3 ? " trial-capsule--warn" : ""}`}>
          试用 · 剩 {license.days_left} 天
        </span>
      );
    }
    if (license.kind === "lapsed") {
      return (
        <button
          type="button"
          className="trial-capsule trial-capsule--lapsed"
          onClick={() => setNav("settings")}
        >
          体验模式 · 每日 {license.daily_limit} 句 · 购买完整版
        </button>
      );
    }
    return null;
  })();

  return (
    <div className="app-shell">
      <nav className="app-nav">
        <div className="app-nav__brand" title="句流 SentenceFlow">句</div>
        {NAV.map((item) => (
          <button
            key={item.key}
            type="button"
            className={`app-nav__item${nav === item.key ? " app-nav__item--on" : ""}`}
            onClick={() => setNav(item.key)}
          >
            <span className="app-nav__icon" aria-hidden>{item.icon}</span>
            <span className="app-nav__label">{item.label}</span>
          </button>
        ))}
        <div className="app-nav__spacer" />
        <div
          className="app-nav__shield"
          title="所有学习数据仅存于本机 · 密钥在系统钥匙串"
        >
          🛡
        </div>
      </nav>
      <div className="app-main">
        {trialCapsule && <div className="app-topbar">{trialCapsule}</div>}
        {nav === "today" && <TodayPage onStart={(launch) => setPractice(launch)} />}
        {nav === "library" && <LibraryPage onPractice={(launch) => setPractice(launch)} />}
        {nav === "workshop" && (
          <WorkshopPage prefillScene={workshopPrefill} onConsumedPrefill={() => setWorkshopPrefill(null)} />
        )}
        {nav === "report" && (
          <ReportPage
            onDrill={(scene) => {
              setWorkshopPrefill(scene);
              setNav("workshop");
            }}
          />
        )}
        {nav === "settings" && <SettingsPage />}
      </div>
    </div>
  );
}
