import { describe, expect, it } from 'vitest';

import { formatBytes, formatNumber, formatRelativeTime } from './formatters';

describe('本地化格式化器', () => {
  it('使用 Intl 处理数字、相对时间和字节单位', () => {
    expect(formatNumber(12_345.5, 'en')).toBe('12,345.5');
    expect(formatNumber(12_345.5, 'zh-CN')).toBe('12,345.5');
    expect(formatRelativeTime(0, 60_000, 'en')).toBe('1 minute ago');
    expect(formatRelativeTime(0, 60_000, 'zh-CN')).toBe('1分钟前');
    expect(formatBytes(1_536, 'en')).toBe('1.5 KiB');
  });
});
