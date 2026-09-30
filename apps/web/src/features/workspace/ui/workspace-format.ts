export function formatWorkspaceTime(timestamp: number, language: string | undefined): string {
  return new Intl.DateTimeFormat(language ?? 'en', {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(timestamp);
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
/** 超过一个月就直接写日期，“47 天前”不如日期好认。 */
const RELATIVE_LIMIT_MS = 30 * DAY_MS;

/** “3 小时前”“昨天”这样的说法；一分钟以内算“现在”。 */
export function formatRelativeTime(
  timestamp: number,
  now: number,
  language: string | undefined,
): string {
  const elapsed = timestamp - now;
  const distance = Math.abs(elapsed);
  if (distance >= RELATIVE_LIMIT_MS) {
    return new Intl.DateTimeFormat(language ?? 'en', { dateStyle: 'medium' }).format(timestamp);
  }
  const formatter = new Intl.RelativeTimeFormat(language ?? 'en', { numeric: 'auto' });
  if (distance >= DAY_MS) return formatter.format(Math.round(elapsed / DAY_MS), 'day');
  if (distance >= HOUR_MS) return formatter.format(Math.round(elapsed / HOUR_MS), 'hour');
  return formatter.format(Math.round(elapsed / MINUTE_MS), 'minute');
}
