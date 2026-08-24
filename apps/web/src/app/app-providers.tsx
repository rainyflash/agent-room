import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { useMemo } from 'react';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { router } from '@/app/router';
import { WebObserverHandoffGateway } from '@/features/handoffs/adapters/web-observer-handoff-gateway';
import { MatrixLobbyGateway } from '@/features/lobby/adapters/matrix-lobby-gateway';
import { MatrixSdkLobbySource } from '@/features/lobby/adapters/matrix-lobby-source';
import { BrowserContentVerifier } from '@/features/messages/adapters/browser-content-verifier';
import { ControlPlaneContentClient } from '@/features/messages/adapters/control-plane-content-client';
import { MatrixMessageGateway } from '@/features/messages/adapters/matrix-message-gateway';
import { MatrixSdkMessageSource } from '@/features/messages/adapters/matrix-message-source';
import { WebObserverMessagePublisher } from '@/features/messages/adapters/web-observer-message-publisher';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { MatrixWebGateway } from '@/features/session/adapters/matrix-web-gateway';
import { SessionProvider } from '@/features/session/ui/session-provider';
import type { RuntimeConfig } from '@/shared/config/runtime-config';
import { WindowBrowserGateway } from '@/shared/browser/window-browser-gateway';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

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
  const matrixClients = new MatrixClientRegistry();
  const matrix = new MatrixWebGateway({
    baseUrl: config.matrixHomeserverUrl,
    onClientActivity: (client) => {
      matrixClients.refresh(client);
    },
    onClientChange: (client) => {
      matrixClients.replace(client);
    },
  });
  const lobby = new MatrixLobbyGateway(new MatrixSdkLobbySource(matrixClients));
  const messages = new MatrixMessageGateway(new MatrixSdkMessageSource(matrixClients));
  const content = new ControlPlaneContentClient({ baseUrl: config.controlPlaneUrl });
  const contentVerifier = new BrowserContentVerifier();
  const handoffs = new WebObserverHandoffGateway();
  const messagePublisher = new WebObserverMessagePublisher();
  const browser = new WindowBrowserGateway();
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: true,
        retry: false,
      },
    },
  });
  const services: AppServices = {
    config,
    content,
    contentVerifier,
    controlPlane,
    handoffs,
    lobby,
    messagePublisher,
    messages,
  };

  return {
    queryClient,
    services,
    sessionDependencies: { browser, controlPlane, matrix },
  };
}
