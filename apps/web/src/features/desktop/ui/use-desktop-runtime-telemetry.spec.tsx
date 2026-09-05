// @vitest-environment jsdom

import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import { useDesktopRuntimeTelemetry } from './use-desktop-runtime-telemetry';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('桌面运行时观测', () => {
  it('普通浏览器不产生桌面连接指标', () => {
    const telemetry = { record: vi.fn().mockResolvedValue(undefined) };
    renderHook(() => {
      useDesktopRuntimeTelemetry(false, 'discovering', telemetry);
    });
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
    expect(telemetry.record).toHaveBeenCalledTimes(1);
    expect(telemetry.record).toHaveBeenCalledWith({
      metric: 'bridge_availability',
      surface: 'desktop',
      value: 0,
    });
    clock.mockReturnValue(350);
    rerender({ phase: 'ready' });
    rerender({ phase: 'ready' });
    expect(telemetry.record).toHaveBeenCalledTimes(3);
    expect(telemetry.record).toHaveBeenCalledWith({
      metric: 'bridge_reconnect',
      surface: 'desktop',
      value: 250,
    });
    expect(telemetry.record).toHaveBeenLastCalledWith({
      metric: 'bridge_availability',
      surface: 'desktop',
      value: 1,
    });
  });
});
