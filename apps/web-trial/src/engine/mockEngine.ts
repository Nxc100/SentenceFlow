/**
 * DEV ONLY — sf-wasm 缺席时的极简回退,让 UI 可以开发预览。
 * 生产构建必须先 `npm run build:wasm`;此实现不承诺与 sf-core 行为一致。
 */

import type {
  CoreEngine,
  Session,
  SrsState,
  Verdict,
  WordVerdict,
} from "@sentenceflow/ui";

function freshState(now: number): SrsState {
  return {
    box_idx: 1,
    progress: 0,
    due_at: now,
    err: 0,
    last_mode: null,
    last_at: now,
    seen_answer: false,
    marked_unfamiliar: false,
  };
}

const norm = (w: string) =>
  w
    .split("")
    .filter((c) => /[a-zA-Z']/.test(c))
    .join("")
    .toLowerCase();

export const mockEngine: CoreEngine = {
  async parseLevelSpec() {
    throw new Error("mock engine cannot parse YAML — build sf-wasm");
  },
  async buildSession(due, newPool, _spec, now, _seed, dailyNewOverride): Promise<Session> {
    const items = [
      ...due
        .filter(([, s]) => s.due_at <= now)
        .map(([id]) => ({
          sentence_id: id,
          mode: "typing" as const,
          is_review: true,
          reorder_first: false,
        })),
      ...newPool.slice(0, dailyNewOverride ?? 10).map((id) => ({
        sentence_id: id,
        mode: "typing" as const,
        is_review: false,
        reorder_first: true,
      })),
    ];
    return { items, overflow_reviews: 0 };
  },
  async applyOutcome(state, outcome, mode, _spec, now): Promise<SrsState> {
    const s = { ...state, last_mode: mode, last_at: now };
    if (outcome.kind === "correct") {
      s.box_idx = Math.min(5, s.box_idx + 1);
      s.due_at = now + s.box_idx * 86_400;
    } else {
      s.box_idx = 1;
      s.err += 1;
      s.due_at = now;
    }
    return s;
  },
  async newSrsState(now) {
    return freshState(now);
  },
  async judge(input, targets): Promise<Verdict> {
    const got = input.split(/\s+/).filter(Boolean).map(norm);
    const words: WordVerdict[] = targets.map((t, i) => {
      const g = got[i];
      if (g === undefined) return { kind: "missing", expected: t };
      if (g === norm(t)) return { kind: "correct", word: t };
      return { kind: "wrong", expected: t, got: g };
    });
    for (const g of got.slice(targets.length)) words.push({ kind: "extra", got: g });
    const errors = words.filter((w) => w.kind !== "correct").length;
    return { words, correct: errors === 0, errors };
  },
  async foldStats() {
    return {
      days: {},
      streak_days: 0,
      best_correct_streak: 0,
      weak_pos: [],
      weak_roles: [],
      total_attempts: 0,
      total_correct: 0,
    };
  },
};
