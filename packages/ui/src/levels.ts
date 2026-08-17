/**
 * 等级友好显示 — 面向小白用户,不露 L1/CEFR 术语(§4.9 的展示层)。
 *
 * L1–L6 与 CEFR 仅是内部/数据层标识;用户界面统一用「阶段名 + 能干什么」
 * 表达。能力描述优先取 LevelSpec.can_do(单一事实源),这里的兜底文案仅
 * 供拿不到 spec 的场合(如试用站静态数据缺字段时)。
 */

import type { LevelId, LevelSpec } from "./types";

/** 六级阶段名:单调递进、无需背景知识即可比较高低。 */
export const LEVEL_NAME: Record<LevelId, string> = {
  L1: "入门",
  L2: "初级",
  L3: "中级",
  L4: "中高级",
  L5: "高级",
  L6: "精通",
};

/** 兜底能力描述(§4.9 典型 can-do;正常路径用 spec.can_do)。 */
const FALLBACK_CAN_DO: Record<LevelId, string> = {
  L1: "打招呼、自我介绍",
  L2: "购物、问价、问路",
  L3: "点餐、约时间",
  L4: "电话沟通、讲经历",
  L5: "表达观点、处理投诉",
  L6: "工作闲聊、协商",
};

/** 阶段名;未知 id(防御)原样返回。 */
export function levelName(id: string): string {
  return LEVEL_NAME[id as LevelId] ?? id;
}

/** 能力描述:「打招呼、自我介绍」。 */
export function levelCanDo(id: string, spec?: Pick<LevelSpec, "can_do">): string {
  if (spec?.can_do?.length) return spec.can_do.join("、");
  return FALLBACK_CAN_DO[id as LevelId] ?? "";
}

/** 下拉/列表选项文案:「中级 · 点餐、约时间」。 */
export function levelOptionLabel(id: string, spec?: Pick<LevelSpec, "can_do">): string {
  const cando = levelCanDo(id, spec);
  return cando ? `${levelName(id)} · ${cando}` : levelName(id);
}
