import { POS_ZH, posVars } from "../grammar";
import type { PosTag } from "../types";

export interface PosCapsuleProps {
  pos: PosTag;
  className?: string;
}

/** 词性胶囊,高 22 (§5.5) — 配色为教学语义,取自语法色板 */
export function PosCapsule({ pos, className }: PosCapsuleProps) {
  return (
    <span className={["sf-pos-capsule", className].filter(Boolean).join(" ")} style={posVars(pos)}>
      {POS_ZH[pos]}
    </span>
  );
}
