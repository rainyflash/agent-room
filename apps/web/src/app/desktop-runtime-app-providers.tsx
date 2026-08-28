import { RouterProvider } from '@tanstack/react-router';
import { useMemo } from 'react';

import { AppServicesProvider, type DesktopAppServices } from '@/app/app-services';
import { desktopRouter } from '@/app/desktop-router';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export function AppProviders({ config }: { readonly config: RuntimeConfig }) {
  const desktop = useMemo(() => new TauriDesktopRuntimeGateway(), []);
  const services: DesktopAppServices = { config, desktop, runtimeMode: 'desktop' };
  return (
    <AppServicesProvider services={services}>
      <RouterProvider router={desktopRouter} />
    </AppServicesProvider>
  );
}
