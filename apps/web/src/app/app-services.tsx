import { createContext, useContext, type PropsWithChildren } from 'react';

import type { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export type AppServices = {
  readonly config: RuntimeConfig;
  readonly controlPlane: ControlPlaneClient;
};

const AppServicesContext = createContext<AppServices | null>(null);

export type AppServicesProviderProps = PropsWithChildren<{
  readonly services: AppServices;
}>;

export function AppServicesProvider({ children, services }: AppServicesProviderProps) {
  return <AppServicesContext.Provider value={services}>{children}</AppServicesContext.Provider>;
}

export function useAppServices(): AppServices {
  const services = useContext(AppServicesContext);
  if (services === null) {
    throw new Error('AppServicesProvider is missing.');
  }
  return services;
}
