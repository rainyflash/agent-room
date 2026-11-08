import { useEffect, useRef, useState } from 'react';
import {
  BRIDGE_UPTIME_WINDOW_MS,
  BridgeUptimeWindow,
  bridgeUptimeReady,
  type BridgeUptimeState,
} from '@/features/desktop/domain/bridge-uptime';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';

export function useDesktopRuntimeTelemetry(
  available: boolean,
  state: BridgeUptimeState,
  telemetry: FrontendTelemetryGateway | undefined,
): void {
  const previousState = useRef<BridgeUptimeState | null>(null);
  const reconnectStartedAt = useRef<number | null>(null);
  const [uptime] = useState(() => new BridgeUptimeWindow());
  useEffect(() => {
    if (!available || telemetry === undefined || previousState.current === state) return;
    const now = performance.now();
    uptime.observe(state, now);
    // 等人登录的那段不算重连用的时间：登录好了才开始算。
    if (state === 'waiting_for_sign_in') reconnectStartedAt.current = null;
    if (state === 'retry_scheduled' || state === 'starting' || state === 'reconnecting')
      reconnectStartedAt.current ??= now;
    if (bridgeUptimeReady(state) && reconnectStartedAt.current !== null) {
      void telemetry.record({
        metric: 'bridge_reconnect',
        surface: 'desktop',
        value: now - reconnectStartedAt.current,
      });
      reconnectStartedAt.current = null;
    }
    previousState.current = state;
  }, [available, state, telemetry, uptime]);
  useEffect(() => {
    if (!available || telemetry === undefined) return;
    const timer = window.setInterval(() => {
      const share = uptime.take(performance.now());
      if (share !== null)
        void telemetry.record({ metric: 'bridge_uptime', surface: 'desktop', value: share });
    }, BRIDGE_UPTIME_WINDOW_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [available, telemetry, uptime]);
}
