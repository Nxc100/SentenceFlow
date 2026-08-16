export interface HeatmapCalendarProps {
  /** 本地日索引(unix 天) → 当日练习句数 */
  counts: Record<string, number>;
  /** 展示的最后一天(默认今天的日索引) */
  endDay?: number;
  /** 展示周数(列数) */
  weeks?: number;
}

/** 计数 → 0–4 档(5 级绿,§5.5) */
function level(count: number): number {
  if (count <= 0) return 0;
  if (count < 5) return 1;
  if (count < 15) return 2;
  if (count < 30) return 3;
  return 4;
}

export function HeatmapCalendar({ counts, endDay, weeks = 16 }: HeatmapCalendarProps) {
  const end = endDay ?? Math.floor(Date.now() / 86_400_000);
  const totalDays = weeks * 7;
  const start = end - totalDays + 1;
  const cols: number[][] = [];
  for (let w = 0; w < weeks; w++) {
    const col: number[] = [];
    for (let d = 0; d < 7; d++) {
      col.push(start + w * 7 + d);
    }
    cols.push(col);
  }
  return (
    <div className="sf-heatmap" role="img" aria-label="练习热力日历">
      {cols.map((col, i) => (
        <div key={i} className="sf-heatmap__col">
          {col.map((day) => (
            <div
              key={day}
              className="sf-heatmap__cell"
              data-level={level(counts[String(day)] ?? 0)}
              title={`第 ${day} 天:${counts[String(day)] ?? 0} 句`}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
