import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { useEffect, useMemo } from 'react';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { router } from '@/app/router';
import { ControlPlaneAutomationGrantClient } from '@/features/automation/adapters/control-plane-automation-grant-client';
import { ControlPlaneDirectSessionClient } from '@/features/direct-sessions/adapters/control-plane-direct-session-client';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import { BrowserDirectBlockRegistry } from '@/features/direct-sessions/adapters/browser-direct-block-registry';
import { MatrixSdkDirectSessionGateway } from '@/features/direct-sessions/adapters/matrix-direct-session-gateway';
import { DirectSessionCoordinator } from '@/features/direct-sessions/application/direct-session-coordinator';
import { WebObserverHandoffGateway } from '@/features/handoffs/adapters/web-observer-handoff-gateway';
import { MatrixLobbyGateway } from '@/features/lobby/adapters/matrix-lobby-gateway';
import { MatrixSdkLobbySource } from '@/features/lobby/adapters/matrix-lobby-source';
import { ControlPlanePublicLobbyEntryClient } from '@/features/lobby-entry/adapters/control-plane-public-lobby-entry-client';
import { MatrixSdkPublicLobbyEntryGateway } from '@/features/lobby-entry/adapters/matrix-public-lobby-entry-gateway';
import { PublicLobbyEntryCoordinator } from '@/features/lobby-entry/application/public-lobby-entry-coordinator';
import { BrowserContentVerifier } from '@/features/messages/adapters/browser-content-verifier';
import { BrowserMachineTranslationGateway } from '@/features/messages/adapters/browser-machine-translation-gateway';
import { ControlPlaneContentClient } from '@/features/messages/adapters/control-plane-content-client';
import { MatrixMessageGateway } from '@/features/messages/adapters/matrix-message-gateway';
import { MatrixSdkMessageSource } from '@/features/messages/adapters/matrix-message-source';
import { WebObserverMessagePublisher } from '@/features/messages/adapters/web-observer-message-publisher';
import { ControlPlaneModerationClient } from '@/features/moderation/adapters/control-plane-moderation-client';
import { ControlPlaneOnboardingClient } from '@/features/onboarding/adapters/control-plane-onboarding-client';
import { OnboardingCoordinator } from '@/features/onboarding/application/onboarding-coordinator';
import { MatrixAccountPreferencesGateway } from '@/features/preferences/adapters/matrix-account-preferences-gateway';
import { AccountPreferencesStore } from '@/features/preferences/application/account-preferences-store';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import { ControlPlanePrivateRoomClient } from '@/features/private-rooms/adapters/control-plane-private-room-client';
import { MatrixSdkPrivateRoomGateway } from '@/features/private-rooms/adapters/matrix-private-room-gateway';
import { ControlPlaneAccessManagementClient } from '@/features/security/adapters/control-plane-access-management-client';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { MatrixWebGateway } from '@/features/session/adapters/matrix-web-gateway';
import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import { ControlPlaneFrontendTelemetryClient } from '@/features/telemetry/adapters/control-plane-frontend-telemetry-client';
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
          <RouterProvider router={router} />
        </AccountPreferencesProvider>
      </AppServicesProvider>
    </QueryClientProvider>
  );
}

function createRuntime(config: RuntimeConfig) {
  const controlPlane = new ControlPlaneClient({ baseUrl: config.controlPlaneUrl });
  const desktop = new TauriDesktopRuntimeGateway();
  const onboarding = new OnboardingCoordinator(
    new ControlPlaneOnboardingClient({ baseUrl: config.controlPlaneUrl }),
  );
  const frontendTelemetry = new ControlPlaneFrontendTelemetryClient({
    baseUrl: config.controlPlaneUrl,
  });
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
  const lobbyEntry = new PublicLobbyEntryCoordinator(
    new ControlPlanePublicLobbyEntryClient({ baseUrl: config.controlPlaneUrl }),
    new MatrixSdkPublicLobbyEntryGateway(matrixClients),
  );
  const messages = new MatrixMessageGateway(new MatrixSdkMessageSource(matrixClients));
  const content = new ControlPlaneContentClient({ baseUrl: config.controlPlaneUrl });
  const contentVerifier = new BrowserContentVerifier();
  const moderation = new ControlPlaneModerationClient({ baseUrl: config.controlPlaneUrl });
  const privateRooms = new ControlPlanePrivateRoomClient({ baseUrl: config.controlPlaneUrl });
  const privateRoomMatrix = new MatrixSdkPrivateRoomGateway(matrixClients);
  const directSessions = new ControlPlaneDirectSessionClient({ baseUrl: config.controlPlaneUrl });
  const directSessionMatrix = new MatrixSdkDirectSessionGateway(matrixClients);
  const directBlocks = new BrowserDirectBlockRegistry(window.localStorage);
  const directSessionCoordinator = new DirectSessionCoordinator(
    directSessions,
    directSessionMatrix,
    directBlocks,
  );
  const security = new MatrixSdkSecurityGateway(matrixClients, secretStorageKeys);
  const accessManagement = new ControlPlaneAccessManagementClient({
    baseUrl: config.controlPlaneUrl,
  });
  const automation = new ControlPlaneAutomationGrantClient({ baseUrl: config.controlPlaneUrl });
  const handoffs = new WebObserverHandoffGateway();
  const messagePublisher = new WebObserverMessagePublisher();
  const messageTranslation = new BrowserMachineTranslationGateway();
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
    accessManagement,
    automation,
    config,
    content,
    contentVerifier,
    controlPlane,
    directSessionCoordinator,
    directSessions,
    desktop,
    handoffs,
    lobby,
    lobbyEntry,
    messagePublisher,
    messages,
    messageTranslation,
    moderation,
    onboarding,
    privateRoomMatrix,
    privateRooms,
    security,
    session: { browser, controlPlane, matrix },
    telemetry: frontendTelemetry,
  };

  return {
    accountPreferences,
    frontendTelemetry,
    matrixClients,
    queryClient,
    services,
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
