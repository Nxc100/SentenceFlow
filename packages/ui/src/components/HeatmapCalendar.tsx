/**
 * 练习热力日历(§5.5)。
 *
 * 日索引口径与后端 `sf_core::stats::day_index` 一致:**本地日**索引
 * (`(ts + tz_offset) / 86400` 向下取整)。因为偏移已经算进索引里,
 * 反解日期时必须读 UTC 字段 —— 再按本地时区解释一次就会偏一天。
 */

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

const WEEKDAY_ZH = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/** 日索引 → 当天零点的 Date(取值一律用 getUTC*,见文件头注释) */
function dateOf(day: number): Date {
  return new Date(day * 86_400_000);
}

/** 周一 = 0 的星期序号。1970-01-01(第 0 天)是周四 → (0+3)%7 = 3 ✓ */
function mondayIndex(day: number): number {
  return ((day + 3) % 7 + 7) % 7;
}

function dateLabel(day: number): string {
  const d = dateOf(day);
  const mm = String(d.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(d.getUTCDate()).padStart(2, "0");
  return `${d.getUTCFullYear()}-${mm}-${dd} ${WEEKDAY_ZH[d.getUTCDay()]}`;
}

/** 今天(本地)的日索引 */
function todayIndex(): number {
  const now = new Date();
  return Math.floor((now.getTime() - now.getTimezoneOffset() * 60_000) / 86_400_000);
}

/**
 * 列 = 自然周(周一起始),行 = 固定星期 —— 否则"每列 7 天"只是连续切片,
 * 同一行的格子不是同一个星期几,读起来不成日历。
 * 本周之后的格子留空占位,保持网格是整齐的矩形。
 */
export function HeatmapCalendar({ counts, endDay, weeks = 16 }: HeatmapCalendarProps) {
  const end = endDay ?? todayIndex();
  const lastMonday = end - mondayIndex(end);
  const firstMonday = lastMonday - (weeks - 1) * 7;

  const cols: number[][] = [];
  for (let w = 0; w < weeks; w++) {
    const col: number[] = [];
    for (let d = 0; d < 7; d++) col.push(firstMonday + w * 7 + d);
    cols.push(col);
  }

  // 月份标签落在"月份发生变化"的那一列;挨得太近就跳过,免得两个月号叠在一起。
  // 首列不标:它多半是某个月的中间,标上去只会把紧随其后的月号挤掉。
  const monthLabels: Array<string | null> = [];
  let lastLabeledCol = -99;
  let prevMonth = cols.length > 0 ? dateOf(cols[0]![0]!).getUTCMonth() : -1;
  cols.forEach((col, i) => {
    const month = dateOf(col[0]!).getUTCMonth();
    const changed = month !== prevMonth;
    prevMonth = month;
    if (changed && i - lastLabeledCol >= 3) {
      lastLabeledCol = i;
      monthLabels.push(`${month + 1}月`);
    } else {
      monthLabels.push(null);
    }
  });

  const activeDays = cols
    .flat()
    .filter((day) => day <= end && (counts[String(day)] ?? 0) > 0).length;

  return (
    <div
      className="sf-heatmap-cal"
      role="img"
      aria-label={`练习热力日历:${dateLabel(firstMonday)} 至 ${dateLabel(end)},其中 ${activeDays} 天有练习`}
    >
      <div className="sf-heatmap__months" aria-hidden>
        {monthLabels.map((label, i) => (
          <span key={i} className="sf-heatmap__month">
            {label}
          </span>
        ))}
      </div>
      <div className="sf-heatmap">
        {cols.map((col, i) => (
          <div key={i} className="sf-heatmap__col">
            {col.map((day) => {
              if (day > end) {
                // 未来的日子:占位不上色,也不给 tooltip
                return <div key={day} className="sf-heatmap__cell sf-heatmap__cell--future" />;
              }
              const count = counts[String(day)] ?? 0;
              return (
                <div
                  key={day}
                  className="sf-heatmap__cell"
                  data-level={level(count)}
                  title={`${dateLabel(day)} · ${count > 0 ? `${count} 句` : "未练习"}`}
                />
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
