import { createContext, useContext, type PropsWithChildren } from 'react';

import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import {
  useDesktopRuntime,
  type DesktopRuntimeController,
} from '@/features/desktop/ui/use-desktop-runtime';

const DesktopRuntimeContext = createContext<DesktopRuntimeController | null>(null);

export type DesktopRuntimeProviderProps = PropsWithChildren<{
  readonly gateway: DesktopRuntimeGateway;
}>;

export function DesktopRuntimeProvider({ children, gateway }: DesktopRuntimeProviderProps) {
  const controller = useDesktopRuntime(gateway);
  return (
    <DesktopRuntimeContext.Provider value={controller}>{children}</DesktopRuntimeContext.Provider>
  );
}

export function useDesktopRuntimeController(): DesktopRuntimeController {
  const controller = useContext(DesktopRuntimeContext);
  if (controller === null) {
    throw new Error('DesktopRuntimeProvider is missing.');
  }
  return controller;
}
