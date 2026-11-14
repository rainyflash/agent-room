export type DateTimeStyle = 'compact' | 'full';

export function formatDateTime(
  value: number | Date,
  language: string | undefined,
  style: DateTimeStyle = 'full',
): string {
  const options: Intl.DateTimeFormatOptions =
    style === 'compact'
      ? { hour: '2-digit', minute: '2-digit' }
      : { dateStyle: 'medium', timeStyle: 'short' };
  return new Intl.DateTimeFormat(language, options).format(value);
}

export function formatNumber(
  value: number,
  language: string | undefined,
  options: Intl.NumberFormatOptions = {},
): string {
  return new Intl.NumberFormat(language, options).format(value);
}

export function formatRelativeTime(
  occurredAtUnixMs: number,
  nowUnixMs: number,
  language: string | undefined,
): string {
  const deltaSeconds = (occurredAtUnixMs - nowUnixMs) / 1_000;
  const [value, unit] = selectRelativeUnit(deltaSeconds);
  return new Intl.RelativeTimeFormat(language, { numeric: 'auto' }).format(value, unit);
}

export function formatBytes(value: number, language: string | undefined): string {
  if (value < 1_024) {
    return `${formatNumber(value, language, { maximumFractionDigits: 1 })} B`;
  }
  if (value < 1_024 * 1_024) {
    return `${formatNumber(value / 1_024, language, { maximumFractionDigits: 1 })} KiB`;
  }
  return `${formatNumber(value / (1_024 * 1_024), language, {
    maximumFractionDigits: 1,
  })} MiB`;
}

function selectRelativeUnit(deltaSeconds: number): readonly [number, Intl.RelativeTimeFormatUnit] {
  const absoluteSeconds = Math.abs(deltaSeconds);
  if (absoluteSeconds < 60) {
    return [Math.round(deltaSeconds), 'second'];
  }
  if (absoluteSeconds < 3_600) {
    return [Math.round(deltaSeconds / 60), 'minute'];
  }
  if (absoluteSeconds < 86_400) {
    return [Math.round(deltaSeconds / 3_600), 'hour'];
  }
  return [Math.round(deltaSeconds / 86_400), 'day'];
}

/**
 * 哪天、几点：今天和昨天写“今天 09:12”“昨天 21:05”，更早的只写日期（“11月11日”），
 * 不是今年的带上年份。按这台设备的时区算哪一天。
 */
export function formatDayAndTime(
  occurredAtUnixMs: number,
  nowUnixMs: number,
  language: string | undefined,
): string {
  const occurred = new Date(occurredAtUnixMs);
  const now = new Date(nowUnixMs);
  const days = Math.round((startOfDay(occurred) - startOfDay(now)) / 86_400_000);
  if (days === 0 || days === -1) {
    const day = new Intl.RelativeTimeFormat(language, { numeric: 'auto' }).format(days, 'day');
    return `${day.charAt(0).toLocaleUpperCase(language)}${day.slice(1)} ${formatDateTime(occurred, language, 'compact')}`;
  }
  return new Intl.DateTimeFormat(language, {
    day: 'numeric',
    month: 'short',
    ...(occurred.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  }).format(occurred);
}

function startOfDay(value: Date): number {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
}
