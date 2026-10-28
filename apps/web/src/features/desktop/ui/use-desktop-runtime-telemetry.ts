import { useEffect, useRef, useState } from 'react';
import {
  BRIDGE_UPTIME_WINDOW_MS,
  BridgeUptimeWindow,
} from '@/features/desktop/domain/bridge-uptime';
import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import { localConnectionReady } from '@/features/desktop/domain/desktop-connection';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';

export function useDesktopRuntimeTelemetry(
  available: boolean,
  phase: BridgePhase,
  telemetry: FrontendTelemetryGateway | undefined,
): void {
  const previousPhase = useRef<BridgePhase | null>(null);
  const reconnectStartedAt = useRef<number | null>(null);
  const [uptime] = useState(() => new BridgeUptimeWindow());
  useEffect(() => {
    if (!available || telemetry === undefined || previousPhase.current === phase) return;
    const now = performance.now();
    uptime.observe(phase, now);
    if (phase === 'retry_scheduled' || phase === 'starting' || phase === 'reconnecting')
      reconnectStartedAt.current ??= now;
    if (localConnectionReady(phase) && reconnectStartedAt.current !== null) {
      void telemetry.record({
        metric: 'bridge_reconnect',
        surface: 'desktop',
        value: now - reconnectStartedAt.current,
      });
      reconnectStartedAt.current = null;
    }
    previousPhase.current = phase;
  }, [available, phase, telemetry, uptime]);
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
