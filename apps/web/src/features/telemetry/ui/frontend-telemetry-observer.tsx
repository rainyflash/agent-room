import { useEffect } from 'react';

import { reportApiRoundTrip } from '@/features/telemetry/application/api-round-trip';
import { BrowserPerformanceSampler } from '@/features/telemetry/application/browser-performance-sampler';
import { resolveFrontendSurface } from '@/features/telemetry/adapters/runtime-surface';
import type {
  ApiRoundTripProbe,
  FrontendTelemetryGateway,
} from '@/features/telemetry/domain/frontend-metric';
import { useSession } from '@/features/session/ui/session-provider';

export type FrontendTelemetryObserverProps = {
  readonly gateway: FrontendTelemetryGateway;
  /** 登录以后量一次调接口来回要多久；测试和夹具里可以没有。 */
  readonly roundTrip?: ApiRoundTripProbe | undefined;
};

export function FrontendTelemetryObserver({ gateway, roundTrip }: FrontendTelemetryObserverProps) {
  const { snapshot } = useSession();
  const authenticated = snapshot.context.principal !== null;

  useEffect(() => {
    if (!authenticated) {
      return undefined;
    }
    const surface = resolveFrontendSurface();
    const stopSampler = new BrowserPerformanceSampler(gateway, surface).start();
    const stopRoundTrip =
      roundTrip === undefined ? undefined : reportApiRoundTrip(roundTrip, gateway, surface);
    return () => {
      stopSampler();
      stopRoundTrip?.();
    };
  }, [authenticated, gateway, roundTrip]);

  return null;
}
