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
