import { useEffect } from 'react';

import { BrowserPerformanceSampler } from '@/features/telemetry/application/browser-performance-sampler';
import { resolveFrontendSurface } from '@/features/telemetry/adapters/runtime-surface';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';
import { useSession } from '@/features/session/ui/session-provider';

export type FrontendTelemetryObserverProps = {
  readonly gateway: FrontendTelemetryGateway;
};

export function FrontendTelemetryObserver({ gateway }: FrontendTelemetryObserverProps) {
  const { snapshot } = useSession();
  const authenticated = snapshot.context.principal !== null;

  useEffect(() => {
    if (!authenticated) {
      return undefined;
    }
    return new BrowserPerformanceSampler(gateway, resolveFrontendSurface()).start();
  }, [authenticated, gateway]);

  return null;
}
