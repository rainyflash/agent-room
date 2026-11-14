import { describe, expect, it } from 'vitest';

import { formatBytes, formatDayAndTime, formatNumber, formatRelativeTime } from './formatters';

describe('本地化格式化器', () => {
  it('使用 Intl 处理数字、相对时间和字节单位', () => {
    expect(formatNumber(12_345.5, 'en')).toBe('12,345.5');
    expect(formatNumber(12_345.5, 'zh-CN')).toBe('12,345.5');
    expect(formatRelativeTime(0, 60_000, 'en')).toBe('1 minute ago');
    expect(formatRelativeTime(0, 60_000, 'zh-CN')).toBe('1分钟前');
    expect(formatBytes(1_536, 'en')).toBe('1.5 KiB');
  });
});

describe('formatDayAndTime', () => {
  // 按这台机器的时区算哪一天，所以时刻也用本地时间给。
  const now = new Date(2026, 10, 14, 15, 30).getTime();

  it('今天和昨天写“今天”“昨天”加时刻', () => {
    const today = new Date(2026, 10, 14, 9, 12).getTime();
    const yesterday = new Date(2026, 10, 13, 21, 5).getTime();
    expect(formatDayAndTime(today, now, 'zh-CN')).toBe('今天 09:12');
    expect(formatDayAndTime(yesterday, now, 'zh-CN')).toBe('昨天 21:05');
    expect(formatDayAndTime(today, now, 'en')).toMatch(/^Today 09:12/u);
    expect(formatDayAndTime(yesterday, now, 'en')).toMatch(/^Yesterday 09:05/u);
  });

  it('昨天半夜前一分钟也算昨天，不按隔了多少小时算', () => {
    const lateYesterday = new Date(2026, 10, 13, 23, 59).getTime();
    const earlyToday = new Date(2026, 10, 14, 0, 1).getTime();
    expect(formatDayAndTime(lateYesterday, earlyToday, 'zh-CN')).toBe('昨天 23:59');
  });

  it('再早只写月和日；不是今年的带上年', () => {
    expect(formatDayAndTime(new Date(2026, 10, 3, 8, 0).getTime(), now, 'zh-CN')).toBe('11月3日');
    expect(formatDayAndTime(new Date(2026, 10, 3, 8, 0).getTime(), now, 'en')).toBe('Nov 3');
    expect(formatDayAndTime(new Date(2025, 11, 30, 8, 0).getTime(), now, 'zh-CN')).toBe(
      '2025年12月30日',
    );
    expect(formatDayAndTime(new Date(2025, 11, 30, 8, 0).getTime(), now, 'en')).toBe(
      'Dec 30, 2025',
    );
  });
});
