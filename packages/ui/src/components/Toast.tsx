import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";

export interface ToastItem {
  id: number;
  message: string;
  kind: "info" | "success" | "error";
}

interface ToastContextValue {
  show: (message: string, kind?: ToastItem["kind"]) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const TOAST_DURATION_MS = 2400; // §5.5

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const show = useCallback((message: string, kind: ToastItem["kind"] = "info") => {
    const id = nextId.current++;
    setItems((prev) => [...prev, { id, message, kind }]);
    window.setTimeout(() => {
      setItems((prev) => prev.filter((t) => t.id !== id));
    }, TOAST_DURATION_MS);
  }, []);

  const value = useMemo(() => ({ show }), [show]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="sf-toast-region" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`sf-toast${t.kind !== "info" ? ` sf-toast--${t.kind}` : ""}`}>
            {t.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}
