import { RouterProvider } from '@tanstack/react-router';
import { lazy, Suspense, useMemo } from 'react';

import { AppServicesProvider, type DesktopAppServices, type RuntimeMode } from '@/app/app-services';
import { desktopRouter } from '@/app/desktop-router';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

const WebAppProviders =
  import.meta.env.MODE === 'desktop'
    ? null
    : lazy(async () => {
        const module = await import('@/app/web-app-providers');
        return { default: module.WebAppProviders };
      });

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
  if (WebAppProviders === null) {
    throw new Error('Web providers are excluded from the packaged desktop build.');
  }
  return (
    <Suspense fallback={null}>
      <WebAppProviders config={config} desktop={desktop} />
    </Suspense>
  );
}

export function resolveRuntimeMode(buildMode: string, desktopAvailable: boolean): RuntimeMode {
  return buildMode === 'desktop' || desktopAvailable ? 'desktop' : 'web';
}
