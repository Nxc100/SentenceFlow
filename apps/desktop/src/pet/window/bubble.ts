/** 提醒气泡：宠物头顶的文字气泡 + 操作按钮（知道了 / 稍后）。 */

import { el, clear } from "./dom";

export interface BubbleAction {
  label: string;
  onClick: () => void;
}

let hideTimer: number | undefined;

export function showBubble(text: string, actions: BubbleAction[] = [], ttlMs = 30_000): void {
  const bubble = document.getElementById("bubble")!;
  const textEl = document.getElementById("bubble-text")!;
  const actionsEl = document.getElementById("bubble-actions")!;

  textEl.textContent = text;
  clear(actionsEl as HTMLElement);
  for (const action of actions) {
    actionsEl.append(
      el(
        "button",
        {
          class: "bubble-btn",
          onclick: () => {
            action.onClick();
            hideBubble();
          },
        },
        action.label,
      ),
    );
  }
  bubble.classList.remove("hidden");
  window.clearTimeout(hideTimer);
  if (ttlMs > 0) {
    hideTimer = window.setTimeout(hideBubble, ttlMs);
  }
}

export function hideBubble(): void {
  document.getElementById("bubble")?.classList.add("hidden");
  window.clearTimeout(hideTimer);
}

/** 气泡是否可见（可见期间气泡区域必须可点击）。 */
export function isBubbleVisible(): boolean {
  const bubble = document.getElementById("bubble");
  return !!bubble && !bubble.classList.contains("hidden");
}
