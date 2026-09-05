import { createContext, useContext, type PropsWithChildren } from 'react';

import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';
import { useDesktopRuntimeTelemetry } from './use-desktop-runtime-telemetry';
import {
  useDesktopRuntime,
  type DesktopRuntimeController,
} from '@/features/desktop/ui/use-desktop-runtime';

const DesktopRuntimeContext = createContext<DesktopRuntimeController | null>(null);

export type DesktopRuntimeProviderProps = PropsWithChildren<{
  readonly gateway: DesktopRuntimeGateway;
  readonly telemetry?: FrontendTelemetryGateway;
}>;

export function DesktopRuntimeProvider({
  children,
  gateway,
  telemetry,
}: DesktopRuntimeProviderProps) {
  const controller = useDesktopRuntime(gateway);
  useDesktopRuntimeTelemetry(
    controller.available,
    controller.snapshot?.bridge.lifecycle.phase ?? 'discovering',
    telemetry,
  );
  return (
    <DesktopRuntimeContext.Provider value={controller}>{children}</DesktopRuntimeContext.Provider>
  );
}

export function useDesktopRuntimeController(): DesktopRuntimeController {
  const controller = useContext(DesktopRuntimeContext);
  if (controller === null) {
    throw new Error('桌面运行时上下文未提供。');
  }
  return controller;
}
