import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { useMemo } from 'react';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { router } from '@/app/router';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { MatrixWebGateway } from '@/features/session/adapters/matrix-web-gateway';
import { SessionProvider } from '@/features/session/ui/session-provider';
import type { RuntimeConfig } from '@/shared/config/runtime-config';
import { WindowBrowserGateway } from '@/shared/browser/window-browser-gateway';

export type AppProvidersProps = {
  readonly config: RuntimeConfig;
};

export function AppProviders({ config }: AppProvidersProps) {
  const runtime = useMemo(() => createRuntime(config), [config]);

  return (
    <QueryClientProvider client={runtime.queryClient}>
      <AppServicesProvider services={runtime.services}>
        <SessionProvider dependencies={runtime.sessionDependencies}>
          <RouterProvider router={router} />
        </SessionProvider>
      </AppServicesProvider>
    </QueryClientProvider>
  );
}

function createRuntime(config: RuntimeConfig) {
  const controlPlane = new ControlPlaneClient({ baseUrl: config.controlPlaneUrl });
  const matrix = new MatrixWebGateway({ baseUrl: config.matrixHomeserverUrl });
  const browser = new WindowBrowserGateway();
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: true,
        retry: false,
      },
    },
  });
  const services: AppServices = { config, controlPlane };

  return {
    queryClient,
    services,
    sessionDependencies: { browser, controlPlane, matrix },
  };
}
