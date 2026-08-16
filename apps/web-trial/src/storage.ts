/**
 * 试用版进度存储:IndexedDB(SrsState by 句子 en 文本 + LogRow 列表)。
 * 键用 en 文本而非数字 id —— 桌面版导入按 en 精确匹配出厂库(§7.9)。
 */

import type { LogRow, SrsState } from "@sentenceflow/ui";

const DB_NAME = "sentenceflow-trial";
const DB_VERSION = 1;
const SRS_STORE = "srs";
const LOG_STORE = "log";

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(SRS_STORE)) db.createObjectStore(SRS_STORE);
      if (!db.objectStoreNames.contains(LOG_STORE))
        db.createObjectStore(LOG_STORE, { autoIncrement: true });
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function tx<T>(
  db: IDBDatabase,
  store: string,
  mode: IDBTransactionMode,
  run: (s: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const t = db.transaction(store, mode);
    const req = run(t.objectStore(store));
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function loadSrsMap(): Promise<Map<string, SrsState>> {
  const db = await openDb();
  const [keys, values] = await Promise.all([
    tx<IDBValidKey[]>(db, SRS_STORE, "readonly", (s) => s.getAllKeys()),
    tx<SrsState[]>(db, SRS_STORE, "readonly", (s) => s.getAll()),
  ]);
  const map = new Map<string, SrsState>();
  keys.forEach((k, i) => {
    const v = values[i];
    if (typeof k === "string" && v) map.set(k, v);
  });
  return map;
}

export async function saveSrs(en: string, state: SrsState): Promise<void> {
  const db = await openDb();
  await tx(db, SRS_STORE, "readwrite", (s) => s.put(state, en));
}

export async function appendLog(row: LogRow): Promise<void> {
  const db = await openDb();
  await tx(db, LOG_STORE, "readwrite", (s) => s.add(row));
}

export async function loadLogs(): Promise<LogRow[]> {
  const db = await openDb();
  return tx<LogRow[]>(db, LOG_STORE, "readonly", (s) => s.getAll());
}

/** 导出 JSON(桌面版 import_trial_progress 的入参形状)。 */
export async function exportProgress(): Promise<string> {
  const srs = await loadSrsMap();
  const items = [...srs.entries()].map(([en, state]) => ({ en, srs: state }));
  return JSON.stringify({ version: 1, items }, null, 2);
}

export function downloadProgress(json: string) {
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "sentenceflow-trial-progress.json";
  a.click();
  URL.revokeObjectURL(url);
}
