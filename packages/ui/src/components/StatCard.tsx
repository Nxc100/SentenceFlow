export interface StatCardProps {
  value: string;
  label: string;
}

/** 完成页/报告统计卡 (§5.5) */
export function StatCard({ value, label }: StatCardProps) {
  return (
    <div className="sf-stat-card">
      <div className="sf-stat-card__value">{value}</div>
      <div className="sf-stat-card__label">{label}</div>
    </div>
  );
}
