export interface ProgressBarProps {
  /** 0–1 */
  value: number;
  "aria-label"?: string;
}

/** 4px 进度条 (§5.5) */
export function ProgressBar({ value, ...aria }: ProgressBarProps) {
  const pct = Math.max(0, Math.min(1, value)) * 100;
  return (
    <div
      className="sf-progress"
      role="progressbar"
      aria-valuenow={Math.round(pct)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-label={aria["aria-label"]}
    >
      <div className="sf-progress__fill" style={{ width: `${pct}%` }} />
    </div>
  );
}
