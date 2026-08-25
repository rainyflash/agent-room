import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { useEffect, useMemo } from 'react';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { router } from '@/app/router';
import { ControlPlaneDirectSessionClient } from '@/features/direct-sessions/adapters/control-plane-direct-session-client';
import { MatrixSdkDirectSessionGateway } from '@/features/direct-sessions/adapters/matrix-direct-session-gateway';
import { DirectSessionCoordinator } from '@/features/direct-sessions/application/direct-session-coordinator';
import { WebObserverHandoffGateway } from '@/features/handoffs/adapters/web-observer-handoff-gateway';
import { MatrixLobbyGateway } from '@/features/lobby/adapters/matrix-lobby-gateway';
import { MatrixSdkLobbySource } from '@/features/lobby/adapters/matrix-lobby-source';
import { BrowserContentVerifier } from '@/features/messages/adapters/browser-content-verifier';
import { ControlPlaneContentClient } from '@/features/messages/adapters/control-plane-content-client';
import { MatrixMessageGateway } from '@/features/messages/adapters/matrix-message-gateway';
import { MatrixSdkMessageSource } from '@/features/messages/adapters/matrix-message-source';
import { WebObserverMessagePublisher } from '@/features/messages/adapters/web-observer-message-publisher';
import { MatrixAccountPreferencesGateway } from '@/features/preferences/adapters/matrix-account-preferences-gateway';
import { AccountPreferencesStore } from '@/features/preferences/application/account-preferences-store';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import { ControlPlanePrivateRoomClient } from '@/features/private-rooms/adapters/control-plane-private-room-client';
import { MatrixSdkPrivateRoomGateway } from '@/features/private-rooms/adapters/matrix-private-room-gateway';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { MatrixWebGateway } from '@/features/session/adapters/matrix-web-gateway';
import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import { SessionProvider } from '@/features/session/ui/session-provider';
import type { RuntimeConfig } from '@/shared/config/runtime-config';
import { readLanguagePreference } from '@/shared/i18n/i18n';
import { WindowBrowserGateway } from '@/shared/browser/window-browser-gateway';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';

export type AppProvidersProps = {
  readonly config: RuntimeConfig;
};

export function AppProviders({ config }: AppProvidersProps) {
  const runtime = useMemo(() => createRuntime(config), [config]);
  useVerticalSecurityDriver(runtime.matrixClients);

  return (
    <QueryClientProvider client={runtime.queryClient}>
      <AppServicesProvider services={runtime.services}>
        <AccountPreferencesProvider store={runtime.accountPreferences}>
          <SessionProvider dependencies={runtime.sessionDependencies}>
            <RouterProvider router={router} />
          </SessionProvider>
        </AccountPreferencesProvider>
      </AppServicesProvider>
    </QueryClientProvider>
  );
}

function createRuntime(config: RuntimeConfig) {
  const controlPlane = new ControlPlaneClient({ baseUrl: config.controlPlaneUrl });
  const matrixClients = new MatrixClientRegistry();
  const secretStorageKeys = new MatrixSecretStorageKeyCache();
  const matrix = new MatrixWebGateway({
    baseUrl: config.matrixHomeserverUrl,
    onClientActivity: (client) => {
      matrixClients.refresh(client);
    },
    onClientChange: (client) => {
      matrixClients.replace(client);
    },
    secretStorageKeys,
  });
  const accountPreferences = new AccountPreferencesStore(
    new MatrixAccountPreferencesGateway(matrixClients),
    {
      language: readLanguagePreference(window.localStorage),
      lobbyView: 'scene',
    },
  );
  const lobby = new MatrixLobbyGateway(new MatrixSdkLobbySource(matrixClients));
  const messages = new MatrixMessageGateway(new MatrixSdkMessageSource(matrixClients));
  const content = new ControlPlaneContentClient({ baseUrl: config.controlPlaneUrl });
  const contentVerifier = new BrowserContentVerifier();
  const privateRooms = new ControlPlanePrivateRoomClient({ baseUrl: config.controlPlaneUrl });
  const privateRoomMatrix = new MatrixSdkPrivateRoomGateway(matrixClients);
  const directSessions = new ControlPlaneDirectSessionClient({ baseUrl: config.controlPlaneUrl });
  const directSessionMatrix = new MatrixSdkDirectSessionGateway(matrixClients);
  const directSessionCoordinator = new DirectSessionCoordinator(
    directSessions,
    directSessionMatrix,
  );
  const security = new MatrixSdkSecurityGateway(matrixClients, secretStorageKeys);
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
    directSessionCoordinator,
    directSessions,
    handoffs,
    lobby,
    messagePublisher,
    messages,
    privateRoomMatrix,
    privateRooms,
    security,
  };

  return {
    accountPreferences,
    matrixClients,
    queryClient,
    services,
    sessionDependencies: { browser, controlPlane, matrix },
  };
}

function useVerticalSecurityDriver(matrixClients: MatrixClientRegistry): void {
  useEffect(() => {
    if (import.meta.env.VITE_AGENT_ROOM_VERTICAL_SECURITY_DRIVER !== 'enabled') {
      return;
    }
    let active = true;
    let uninstall: (() => void) | undefined;
    void import('@/test/vertical-security-driver').then((driver) => {
      if (active) {
        uninstall = driver.installVerticalSecurityDriver(matrixClients);
      }
    });
    return () => {
      active = false;
      uninstall?.();
    };
  }, [matrixClients]);
}
