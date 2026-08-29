/** 交互层（方案 §6 P0-1）：点击=摸、手动拖拽窗口、右键菜单。 */

import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

import { el, clear } from "./dom";

export interface MenuItem {
  label: string;
  icon?: string;
  onClick: () => void;
  separatorAfter?: boolean;
}

export interface InteractCallbacks {
  onTap: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  menuItems: () => MenuItem[];
}

const DRAG_THRESHOLD = 6;

export function setupInteractions(canvas: HTMLCanvasElement, cb: InteractCallbacks): { isDragging: () => boolean } {
  const win = getCurrentWindow();
  let pressed = false;
  let dragging = false;
  let grabX = 0;
  let grabY = 0;
  let moving = false; // setPosition 节流

  canvas.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    hideMenu();
    pressed = true;
    grabX = e.clientX;
    grabY = e.clientY;
    canvas.setPointerCapture(e.pointerId);
  });

  canvas.addEventListener("pointermove", async (e) => {
    if (!pressed) return;
    const dx = e.clientX - grabX;
    const dy = e.clientY - grabY;
    if (!dragging && Math.hypot(dx, dy) > DRAG_THRESHOLD) {
      dragging = true;
      cb.onDragStart();
    }
    if (dragging && !moving) {
      moving = true;
      try {
        const dpr = window.devicePixelRatio || 1;
        const pos = await win.outerPosition();
        await win.setPosition(
          new PhysicalPosition(
            Math.round(pos.x + dx * dpr),
            Math.round(pos.y + dy * dpr),
          ),
        );
      } finally {
        moving = false;
      }
    }
  });

  const release = (e: PointerEvent) => {
    if (!pressed) return;
    pressed = false;
    canvas.releasePointerCapture?.(e.pointerId);
    if (dragging) {
      dragging = false;
      cb.onDragEnd();
    } else if (e.button === 0) {
      cb.onTap();
    }
  };
  canvas.addEventListener("pointerup", release);
  canvas.addEventListener("pointercancel", release);

  // —— 右键菜单 ——
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showMenu(e.clientX, e.clientY, cb.menuItems());
  });
  window.addEventListener("blur", hideMenu);
  document.addEventListener("click", (e) => {
    if (!(e.target as HTMLElement).closest("#context-menu")) hideMenu();
  });

  return { isDragging: () => dragging };
}

function showMenu(x: number, y: number, items: MenuItem[]): void {
  const menu = document.getElementById("context-menu")!;
  clear(menu as HTMLElement);
  for (const item of items) {
    menu.append(
      el(
        "li",
        {
          class: "menu-item",
          onclick: () => {
            hideMenu();
            item.onClick();
          },
        },
        item.icon ? `${item.icon} ${item.label}` : item.label,
      ),
    );
    if (item.separatorAfter) {
      menu.append(el("li", { class: "menu-sep" }));
    }
  }
  menu.classList.remove("hidden");
  // 贴边收纳
  const rect = menu.getBoundingClientRect();
  const px = Math.min(x, window.innerWidth - rect.width - 4);
  const py = Math.min(y, window.innerHeight - rect.height - 4);
  menu.style.left = `${Math.max(2, px)}px`;
  menu.style.top = `${Math.max(2, py)}px`;
}

export function hideMenu(): void {
  document.getElementById("context-menu")?.classList.add("hidden");
}

/** 右键菜单是否展开（展开期间整窗保持可交互，否则点外部无法收起）。 */
export function isMenuOpen(): boolean {
  const menu = document.getElementById("context-menu");
  return !!menu && !menu.classList.contains("hidden");
}
