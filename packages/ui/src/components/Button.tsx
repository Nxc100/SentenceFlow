import type { ButtonHTMLAttributes, ReactNode } from "react";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost";
  children: ReactNode;
}

export function Button({ variant = "primary", className, children, ...rest }: ButtonProps) {
  const cls = ["sf-btn", `sf-btn--${variant}`, className].filter(Boolean).join(" ");
  return (
    <button type="button" className={cls} {...rest}>
      {children}
    </button>
  );
}
