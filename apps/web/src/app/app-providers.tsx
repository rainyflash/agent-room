import { RouterProvider } from '@tanstack/react-router';
import { useMemo } from 'react';

import { AppServicesProvider, type DesktopAppServices, type RuntimeMode } from '@/app/app-services';
import { desktopRouter } from '@/app/desktop-router';
import { WebAppProviders } from '@/app/web-app-providers';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export type AppProvidersProps = {
  readonly config: RuntimeConfig;
  readonly runtimeMode?: RuntimeMode;
};

export function AppProviders({ config, runtimeMode }: AppProvidersProps) {
  const desktop = useMemo(() => new TauriDesktopRuntimeGateway(), []);
  const mode = runtimeMode ?? resolveRuntimeMode(import.meta.env.MODE, desktop.isAvailable());
  if (mode === 'desktop') {
    const services: DesktopAppServices = { config, desktop, runtimeMode: 'desktop' };
    return (
      <AppServicesProvider services={services}>
        <RouterProvider router={desktopRouter} />
      </AppServicesProvider>
    );
  }
  return <WebAppProviders config={config} desktop={desktop} />;
}

export function resolveRuntimeMode(buildMode: string, desktopAvailable: boolean): RuntimeMode {
  return buildMode === 'desktop' || desktopAvailable ? 'desktop' : 'web';
}
