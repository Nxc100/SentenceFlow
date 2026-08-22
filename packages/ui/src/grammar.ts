/** 词性/成分的中文名与配色变量映射(色值只存在于 tokens.css) */

import type * as React from "react";
import type { PosTag, RoleTag } from "./types";

export const POS_ZH: Record<PosTag, string> = {
  pron: "代词",
  n: "名词",
  v: "动词",
  aux: "助动词",
  modal: "情态动词",
  adj: "形容词",
  wh: "疑问词",
  adv: "副词",
  prep: "介词",
  art: "冠词",
  conj: "连词",
  num: "数词",
  propn: "专有名词",
  part: "引导词",
};

export const ROLE_ZH: Record<RoleTag, string> = {
  subj: "主语",
  pred: "谓语",
  link: "系动词",
  obj: "宾语",
  comp: "表语",
  advl: "状语",
  objc: "宾补",
  marker: "引导词",
};

export function posVars(pos: PosTag): React.CSSProperties {
  return {
    background: `var(--sf-pos-${pos}-bg)`,
    color: `var(--sf-pos-${pos}-text)`,
  };
}

export function roleVars(role: RoleTag): React.CSSProperties {
  return {
    background: `var(--sf-role-${role}-bg)`,
    color: `var(--sf-role-${role}-text)`,
    borderColor: `var(--sf-role-${role}-border)`,
    // 成分色的"可用作光晕的那一档";CSS 里拿不到"当前成分"是谁,只能这样传进去
    "--sf-role-tint": `var(--sf-role-${role}-border)`,
  } as React.CSSProperties;
}

/**
 * 撒花的**兜底**色表(§6.2 原文:取语法色板底色系)。
 * 正常路径不走这里 —— `confetti.ts` 会在运行时读当前主题的语法色板文字色,
 * 因为这组近白粉彩压在浅色底上只有 ~1.1:1 的对比,肉眼看不见(见 confetti.ts 头注)。
 * 保留它只为 SSR / 测试等拿不到 CSS 变量的场合有个确定值。
 */
export const CONFETTI_COLORS = [
  "#FFE3EE", "#FFE2DC", "#FCE0F3", "#EBE3FC", "#E6DEFA", "#F0E4FB",
  "#E1EBFF", "#DFF3E3", "#FFEDD5", "#D9F2EF", "#DBF0FA", "#FFF3CD",
  "#FBE3E9", "#E9E3FA", "#DEEBFB", "#FBF1D3",
];
