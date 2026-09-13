import { useEffect, useRef } from 'react';
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
  useEffect(() => {
    if (!available || telemetry === undefined || previousPhase.current === phase) return;
    const now = performance.now();
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
    void telemetry.record({
      metric: 'bridge_availability',
      surface: 'desktop',
      value: localConnectionReady(phase) ? 1 : 0,
    });
    previousPhase.current = phase;
  }, [available, phase, telemetry]);
}
