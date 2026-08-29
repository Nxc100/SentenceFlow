/**
 * 「AI 萌宠」页的设置读写。
 *
 * 设置的权威副本在 Rust 侧(句流 `Settings.pet` 分节);这里只持有快照,
 * 写入一律经 `pet_settings_set`,并订阅 `pet://settings` 接住来自
 * 宠物窗右键菜单等其它入口的改动。
 */

import { useCallback, useEffect, useState } from "react";
import { petEvents, petIpc } from "../../pet/ipc";
import type { PetSettings } from "../../pet/types";

export interface PetSettingsHandle {
  settings: PetSettings | null;
  /** 局部更新(只写传入的字段)。返回后端确认后的新快照。 */
  patch: (changes: Partial<PetSettings>) => Promise<PetSettings | null>;
  error: string | null;
}

export function usePetSettings(): PetSettingsHandle {
  const [settings, setSettings] = useState<PetSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    petIpc
      .settingsGet()
      .then((s) => {
        if (alive) setSettings(s);
      })
      .catch((e) => alive && setError(errText(e)));

    const un = petEvents.onSettings((s) => setSettings(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);

  const patch = useCallback(
    async (changes: Partial<PetSettings>): Promise<PetSettings | null> => {
      if (!settings) return null;
      const next = { ...settings, ...changes };
      // 乐观更新:开关是高频操作,等一次 IPC 往返会有明显的迟滞感
      setSettings(next);
      try {
        const saved = await petIpc.settingsSet(next);
        setSettings(saved);
        setError(null);
        return saved;
      } catch (e) {
        setSettings(settings); // 回滚
        setError(errText(e));
        return null;
      }
    },
    [settings],
  );

  return { settings, patch, error };
}

/** 命令错误是 `{ code, message }`;其它情况兜底成字符串。 */
export function errText(e: unknown): string {
  if (typeof e === "object" && e !== null && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}
