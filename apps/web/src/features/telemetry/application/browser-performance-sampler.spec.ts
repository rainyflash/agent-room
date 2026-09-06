// @vitest-environment jsdom

import { afterEach, expect, it, vi } from 'vitest';

import { BrowserPerformanceSampler } from './browser-performance-sampler';
import type { FrontendTelemetryGateway } from '../domain/frontend-metric';

afterEach(() => vi.unstubAllGlobals());

function sampler() {
  const navigation = {
    domInteractive: 125,
    duration: 200,
    entryType: 'navigation',
    name: 'test-navigation',
    startTime: 0,
    toJSON: () => ({}),
  };
  vi.stubGlobal('performance', {
    getEntriesByType: () => [navigation],
    now: () => 200,
  });
  vi.stubGlobal('PerformanceObserver', undefined);
  const record = vi.fn<FrontendTelemetryGateway['record']>(() => Promise.resolve());
  return { record, sampler: new BrowserPerformanceSampler({ record }, 'web') };
}

it('退出登录时停止采样会丢弃待发指标，不再发送需要登录的请求', () => {
  const instance = sampler();
  const stop = instance.sampler.start();
  stop();
  window.dispatchEvent(new Event('pagehide'));
  expect(instance.record).not.toHaveBeenCalled();
});

it('登录期间关闭窗口仍会上报一次，停止后不会重复发送或遗留旧数据', () => {
  const instance = sampler();
  const stop = instance.sampler.start();
  window.dispatchEvent(new Event('pagehide'));
  window.dispatchEvent(new Event('pagehide'));
  expect(instance.record).toHaveBeenCalledExactlyOnceWith({
    metric: 'time_to_interactive',
    surface: 'web',
    value: 125,
  });
  stop();
  window.dispatchEvent(new Event('pagehide'));
  expect(instance.record).toHaveBeenCalledTimes(1);
});
