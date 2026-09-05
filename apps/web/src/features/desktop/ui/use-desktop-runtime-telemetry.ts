import { useEffect, useRef } from 'react';
import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
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
    if (phase === 'retry_scheduled' || phase === 'starting') reconnectStartedAt.current ??= now;
    if (phase === 'ready' && reconnectStartedAt.current !== null) {
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
      value: phase === 'ready' ? 1 : 0,
    });
    previousPhase.current = phase;
  }, [available, phase, telemetry]);
}
