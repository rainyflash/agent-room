import { expect, it, vi } from 'vitest';

import { reportApiRoundTrip } from './api-round-trip';
import type { ApiRoundTripProbe, FrontendTelemetryGateway } from '../domain/frontend-metric';

function setup(measured: number | null) {
  let resolve: (value: number | null) => void = () => undefined;
  const probe: ApiRoundTripProbe = {
    measure: () =>
      new Promise((done) => {
        resolve = done;
      }),
  };
  const record = vi.fn<FrontendTelemetryGateway['record']>(() => Promise.resolve());
  return {
    finish: () => {
      resolve(measured);
    },
    probe,
    record,
  };
}

it('量完报一次 api_round_trip，只带指标、表面和毫秒数', async () => {
  const { finish, probe, record } = setup(142);
  reportApiRoundTrip(probe, { record }, 'desktop');
  finish();
  await vi.waitFor(() => {
    expect(record).toHaveBeenCalledTimes(1);
  });
  expect(record).toHaveBeenCalledWith({ metric: 'api_round_trip', surface: 'desktop', value: 142 });
});

it('退出登录以后量完也不报，没量成或超出范围的不报', async () => {
  const stopped = setup(90);
  const stop = reportApiRoundTrip(stopped.probe, { record: stopped.record }, 'web');
  stop();
  stopped.finish();

  for (const measured of [null, -1, 60_001, Number.NaN]) {
    const instance = setup(measured);
    reportApiRoundTrip(instance.probe, { record: instance.record }, 'web');
    instance.finish();
    await Promise.resolve();
    expect(instance.record).not.toHaveBeenCalled();
  }
  await Promise.resolve();
  expect(stopped.record).not.toHaveBeenCalled();
});
