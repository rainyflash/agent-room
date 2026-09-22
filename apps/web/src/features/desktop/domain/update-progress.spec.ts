import { describe, expect, it } from 'vitest';
import { updateProgressLabel } from './update-progress';

const t = ((key: string, options?: { percent?: number }) =>
  options?.percent === undefined ? key : `${key}:${String(options.percent)}`) as never;

describe('update progress label', () => {
  it('下载阶段显示整数百分比，没有进度时从 0 开始，安装阶段显示安装中', () => {
    expect(updateProgressLabel(t, null)).toBe('desktop.update.downloading:0');
    expect(
      updateProgressLabel(t, { phase: 'downloading', downloadedBytes: 1, totalBytes: 3 }),
    ).toBe('desktop.update.downloading:33');
    expect(
      updateProgressLabel(t, { phase: 'downloading', downloadedBytes: 9, totalBytes: 0 }),
    ).toBe('desktop.update.downloading:0');
    expect(
      updateProgressLabel(t, { phase: 'downloading', downloadedBytes: 7, totalBytes: 5 }),
    ).toBe('desktop.update.downloading:100');
    expect(updateProgressLabel(t, { phase: 'installing', downloadedBytes: 5, totalBytes: 5 })).toBe(
      'desktop.update.installing',
    );
  });
});
