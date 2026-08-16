import type { ReactNode } from "react";
import { ROLE_ZH, roleVars } from "../grammar";
import type { RoleTag } from "../types";

export interface RoleCardProps {
  role: RoleTag;
  children?: ReactNode;
  className?: string;
}

/** 成分卡片:圆角 16、1.5px 描边 (§5.2/§5.5) */
export function RoleCard({ role, children, className }: RoleCardProps) {
  return (
    <div className={["sf-role-card", className].filter(Boolean).join(" ")} style={roleVars(role)}>
      {children}
      <span className="sf-role-card__name">{ROLE_ZH[role]}</span>
    </div>
  );
}
