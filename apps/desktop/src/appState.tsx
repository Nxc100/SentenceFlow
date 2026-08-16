/**
 * 应用级状态:bootstrap 数据(specs/license/settings)+ 设置写回 + 主题应用。
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import type { ReactNode } from "react";
import type { LevelId, LevelSpec } from "@sentenceflow/ui";
import type { Bootstrap, LicenseState, Settings } from "./ipc";
import { ipc } from "./ipc";

interface AppCtx {
  specs: LevelSpec[];
  specFor: (level: LevelId) => LevelSpec | undefined;
  license: LicenseState;
  settings: Settings;
  updateSettings: (patch: (s: Settings) => Settings) => Promise<void>;
  refreshLicense: () => Promise<void>;
  level: LevelId;
  setLevel: (l: LevelId) => Promise<void>;
  contentRev: string | null;
  sentenceCount: number;
}

const Ctx = createContext<AppCtx | null>(null);

export function useApp(): AppCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useApp outside provider");
  return ctx;
}

function applyAppearance(settings: Settings) {
  const root = document.documentElement;
  const { theme, paper } = settings.appearance;
  if (theme === "system") {
    root.dataset.theme = window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  } else {
    root.dataset.theme = theme;
  }
  if (paper && root.dataset.theme === "light") root.dataset.theme = "paper";
  const rm = settings.accessibility.reduce_motion;
  root.dataset.motion =
    rm === "on" || (rm === "system" && window.matchMedia("(prefers-reduced-motion: reduce)").matches)
      ? "reduced"
      : "";
  root.dataset.dyslexic = settings.accessibility.dyslexic_font ? "1" : "";
}

export function AppStateProvider({
  bootstrap,
  children,
}: {
  bootstrap: Bootstrap;
  children: ReactNode;
}) {
  const [settings, setSettings] = useState<Settings>(bootstrap.settings);
  const [license, setLicense] = useState<LicenseState>(bootstrap.license);

  useEffect(() => {
    applyAppearance(settings);
  }, [settings]);

  const updateSettings = useCallback(
    async (patch: (s: Settings) => Settings) => {
      const next = patch(structuredClone(settings));
      setSettings(next);
      await ipc.setSettings(next);
    },
    [settings],
  );

  const refreshLicense = useCallback(async () => {
    setLicense(await ipc.getLicenseState());
  }, []);

  const level: LevelId = settings.level ?? "L1";
  const setLevel = useCallback(
    async (l: LevelId) => {
      await updateSettings((s) => ({ ...s, level: l }));
    },
    [updateSettings],
  );

  const value: AppCtx = {
    specs: bootstrap.specs,
    specFor: (l) => bootstrap.specs.find((s) => s.id === l),
    license,
    settings,
    updateSettings,
    refreshLicense,
    level,
    setLevel,
    contentRev: bootstrap.content_rev,
    sentenceCount: bootstrap.sentence_count,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
