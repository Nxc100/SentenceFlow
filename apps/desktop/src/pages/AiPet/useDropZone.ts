/**
 * 文件拖放落区(React 版)。
 *
 * Tauri 的拖放是**窗口级**事件,只能拿到物理坐标 —— 所以需要一个进程内注册表
 * 把坐标映射到具体 DOM 落区。原 vanilla 实现只注册不注销,页面反复渲染后
 * 旧落区会一直留在表里(拖到已卸载的节点上仍会触发)。
 * 这里改为「注册即返还注销闭包」,由 `useEffect` 的清理函数收走。
 */

import { useEffect, type RefObject } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";

type DropHandler = (paths: string[]) => void;

const zones = new Map<HTMLElement, DropHandler>();
let unlistenAll: Promise<UnlistenFn> | null = null;

function zoneAt(xPhysical: number, yPhysical: number): HTMLElement | null {
  const dpr = window.devicePixelRatio || 1;
  const target = document.elementFromPoint(xPhysical / dpr, yPhysical / dpr);
  if (!target) return null;
  for (const zone of zones.keys()) {
    if (zone.contains(target)) return zone;
  }
  return null;
}

function clearHover(): void {
  for (const zone of zones.keys()) zone.classList.remove("aipet-drop--over");
}

/** 窗口级监听只挂一次:所有落区共用同一条事件流。 */
function ensureListener(): void {
  if (unlistenAll) return;
  unlistenAll = getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === "over") {
      const hit = zoneAt(payload.position.x, payload.position.y);
      for (const zone of zones.keys()) {
        zone.classList.toggle("aipet-drop--over", zone === hit);
      }
    } else if (payload.type === "drop") {
      clearHover();
      const hit = zoneAt(payload.position.x, payload.position.y);
      const first = payload.paths[0];
      if (hit && first !== undefined) zones.get(hit)?.(payload.paths);
    } else {
      clearHover();
    }
  });
}

/**
 * 把 `ref` 指向的元素登记为拖放落区。`enabled=false` 时不登记
 * (例如导入进行中,不该再接新文件)。
 */
export function useDropZone(
  ref: RefObject<HTMLElement | null>,
  onDrop: DropHandler,
  enabled = true,
): void {
  useEffect(() => {
    const node = ref.current;
    if (!node || !enabled) return;
    zones.set(node, onDrop);
    ensureListener();
    return () => {
      node.classList.remove("aipet-drop--over");
      zones.delete(node);
    };
  }, [ref, onDrop, enabled]);
}
