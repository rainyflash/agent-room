// @vitest-environment jsdom

import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BRIDGE_UPTIME_WINDOW_MS } from '@/features/desktop/domain/bridge-uptime';
import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import { useDesktopRuntimeTelemetry } from './use-desktop-runtime-telemetry';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('桌面运行时观测', () => {
  it('普通浏览器不产生桌面连接指标', () => {
    vi.useFakeTimers();
    const telemetry = { record: vi.fn().mockResolvedValue(undefined) };
    renderHook(() => {
      useDesktopRuntimeTelemetry(false, 'discovering', telemetry);
    });
    vi.advanceTimersByTime(BRIDGE_UPTIME_WINDOW_MS);
    expect(telemetry.record).not.toHaveBeenCalled();
  });

  it('界面重复渲染不重复记录，重连完成只记录一次持续时间', () => {
    const telemetry = { record: vi.fn().mockResolvedValue(undefined) };
    const clock = vi.spyOn(performance, 'now').mockReturnValue(100);
    const { rerender } = renderHook(
      ({ phase }: { readonly phase: BridgePhase }) => {
        useDesktopRuntimeTelemetry(true, phase, telemetry);
      },
      { initialProps: { phase: 'starting' } },
    );
    rerender({ phase: 'starting' });
    expect(telemetry.record).not.toHaveBeenCalled();
    clock.mockReturnValue(350);
    rerender({ phase: 'ready' });
    rerender({ phase: 'ready' });
    expect(telemetry.record).toHaveBeenCalledTimes(1);
    expect(telemetry.record).toHaveBeenCalledWith({
      metric: 'bridge_reconnect',
      surface: 'desktop',
      value: 250,
    });
  });

  it('重连完成到已授权状态也记录重连耗时', () => {
    const telemetry = { record: vi.fn().mockResolvedValue(undefined) };
    const clock = vi.spyOn(performance, 'now').mockReturnValue(100);
    const { rerender } = renderHook(
      ({ phase }: { readonly phase: BridgePhase }) => {
        useDesktopRuntimeTelemetry(true, phase, telemetry);
      },
      { initialProps: { phase: 'reconnecting' } },
    );
    clock.mockReturnValue(500);
    rerender({ phase: 'authorized' });
    expect(telemetry.record).toHaveBeenCalledWith({
      metric: 'bridge_reconnect',
      surface: 'desktop',
      value: 400,
    });
  });

  it('开着时每 15 分钟报一次这段时间连着的比例，启动那几秒不拉低它', () => {
    vi.useFakeTimers();
    const telemetry = { record: vi.fn().mockResolvedValue(undefined) };
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0);
    const { rerender, unmount } = renderHook(
      ({ phase }: { readonly phase: BridgePhase }) => {
        useDesktopRuntimeTelemetry(true, phase, telemetry);
      },
      { initialProps: { phase: 'starting' } },
    );
    clock.mockReturnValue(9_000);
    rerender({ phase: 'ready' });
    telemetry.record.mockClear();
    clock.mockReturnValue(BRIDGE_UPTIME_WINDOW_MS);
    act(() => {
      vi.advanceTimersByTime(BRIDGE_UPTIME_WINDOW_MS);
    });
    expect(telemetry.record).toHaveBeenCalledExactlyOnceWith({
      metric: 'bridge_uptime',
      surface: 'desktop',
      value: (BRIDGE_UPTIME_WINDOW_MS - 9_000) / BRIDGE_UPTIME_WINDOW_MS,
    });
    unmount();
    act(() => {
      vi.advanceTimersByTime(BRIDGE_UPTIME_WINDOW_MS);
    });
    expect(telemetry.record).toHaveBeenCalledTimes(1);
  });
});
