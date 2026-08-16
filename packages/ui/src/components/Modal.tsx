import { useEffect } from "react";
import type { ReactNode } from "react";

export interface ModalProps {
  open: boolean;
  title?: string;
  onClose: () => void;
  children: ReactNode;
}

/** 弹窗:≤560 宽,遮罩 rgba(16,20,30,.45)+4px 模糊 (§5.5) */
export function Modal({ open, title, onClose, children }: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="sf-modal__overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="sf-modal" role="dialog" aria-modal="true" aria-label={title}>
        {title ? <h2 className="sf-modal__title">{title}</h2> : null}
        {children}
      </div>
    </div>
  );
}
