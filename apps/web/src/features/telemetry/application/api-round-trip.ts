import type {
  ApiRoundTripProbe,
  FrontendSurface,
  FrontendTelemetryGateway,
} from '@/features/telemetry/domain/frontend-metric';

/** 服务器只收 0 到 60 秒的耗时。 */
const MAX_ROUND_TRIP_MS = 60_000;

/**
 * 登录以后量一次调接口来回要多久，报上去（服务器按粗地区汇总）。返回的函数停掉这一次：
 * 退出登录以后不再发需要登录的请求。
 */
export function reportApiRoundTrip(
  probe: ApiRoundTripProbe,
  gateway: FrontendTelemetryGateway,
  surface: FrontendSurface,
): () => void {
  let stopped = false;
  void probe.measure().then((value) => {
    if (stopped || value === null || !Number.isFinite(value)) return;
    if (value < 0 || value > MAX_ROUND_TRIP_MS) return;
    void gateway.record({ metric: 'api_round_trip', surface, value });
  });
  return () => {
    stopped = true;
  };
}
