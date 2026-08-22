/** 主题(浅/深/马卡龙少女/跟随系统)与减少动效偏好,localStorage 持久化(§4.8 外观)。 */

/** 取值与 tokens.css 的 `[data-theme=…]` 同名;"system" 解析成 light/dark。 */
export type ThemePref = "light" | "dark" | "macaron" | "system";

/** 可直接落到 data-theme 上的具体主题(即除 system 外的全部)。 */
const CONCRETE: readonly ThemePref[] = ["light", "dark", "macaron"];

const THEME_KEY = "sf-theme";
const MOTION_KEY = "sf-motion";

export function applyTheme(pref: ThemePref) {
  const root = document.documentElement;
  if (pref === "system") {
    const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    root.dataset.theme = dark ? "dark" : "light";
  } else {
    root.dataset.theme = pref;
  }
  localStorage.setItem(THEME_KEY, pref);
}

export function getThemePref(): ThemePref {
  const raw = localStorage.getItem(THEME_KEY) as ThemePref | null;
  return raw && CONCRETE.includes(raw) ? raw : "system";
}

export function applyReducedMotion(on: boolean) {
  document.documentElement.dataset.motion = on ? "reduced" : "";
  localStorage.setItem(MOTION_KEY, on ? "1" : "0");
}

export function getReducedMotion(): boolean {
  return localStorage.getItem(MOTION_KEY) === "1";
}

export function initTheme() {
  applyTheme(getThemePref());
  applyReducedMotion(getReducedMotion());
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", () => {
      if (getThemePref() === "system") applyTheme("system");
    });
}
