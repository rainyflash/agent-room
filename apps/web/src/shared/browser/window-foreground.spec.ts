// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from 'vitest';

import { onWindowForegroundChange, windowInForeground } from './window-foreground';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('窗口在不在前台', () => {
  it('看得见又拿着焦点才算', () => {
    const focus = vi.spyOn(document, 'hasFocus');
    const visibility = vi.spyOn(document, 'visibilityState', 'get');
    focus.mockReturnValue(true);
    visibility.mockReturnValue('visible');
    expect(windowInForeground()).toBe(true);
    // 别的程序在前面：看得见，但没拿着焦点。
    focus.mockReturnValue(false);
    expect(windowInForeground()).toBe(false);
    // 开在别的标签页后面。
    focus.mockReturnValue(true);
    visibility.mockReturnValue('hidden');
    expect(windowInForeground()).toBe(false);
  });

  it('到前台、离开前台、标签页藏起来都叫一声，取消以后不再叫', () => {
    const listener = vi.fn();
    const stop = onWindowForegroundChange(listener);
    window.dispatchEvent(new Event('blur'));
    window.dispatchEvent(new Event('focus'));
    document.dispatchEvent(new Event('visibilitychange'));
    expect(listener).toHaveBeenCalledTimes(3);
    stop();
    window.dispatchEvent(new Event('blur'));
    document.dispatchEvent(new Event('visibilitychange'));
    expect(listener).toHaveBeenCalledTimes(3);
  });
});
