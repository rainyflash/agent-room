import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { useEffect, useMemo } from 'react';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { router } from '@/app/router';
import { ControlPlaneAutomationGrantClient } from '@/features/automation/adapters/control-plane-automation-grant-client';
import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { BrowserDirectBlockRegistry } from '@/features/direct-sessions/adapters/browser-direct-block-registry';
import { ControlPlaneDirectSessionClient } from '@/features/direct-sessions/adapters/control-plane-direct-session-client';
import { MatrixSdkDirectSessionGateway } from '@/features/direct-sessions/adapters/matrix-direct-session-gateway';
import { DirectSessionCoordinator } from '@/features/direct-sessions/application/direct-session-coordinator';
import { ControlPlaneHandoffGateway } from '@/features/handoffs/adapters/control-plane-handoff-gateway';
import { ControlPlanePublicLobbyEntryClient } from '@/features/lobby-entry/adapters/control-plane-public-lobby-entry-client';
import { MatrixSdkPublicLobbyEntryGateway } from '@/features/lobby-entry/adapters/matrix-public-lobby-entry-gateway';
import { PublicLobbyEntryCoordinator } from '@/features/lobby-entry/application/public-lobby-entry-coordinator';
import { MatrixLobbyGateway } from '@/features/lobby/adapters/matrix-lobby-gateway';
import { ControlPlaneNetworkAgentLookup } from '@/features/lobby/adapters/control-plane-network-agent-lookup';
import { NetworkAgentLabelStore } from '@/features/lobby/application/network-agent-label-store';
import { NetworkAgentLabelsProvider } from '@/features/lobby/ui/network-agent-labels';
import { ControlPlaneAgentRosterPolicy } from '@/features/lobby/adapters/control-plane-agent-roster-policy';
import { MatrixSdkLobbySource } from '@/features/lobby/adapters/matrix-lobby-source';
import { BrowserContentVerifier } from '@/features/messages/adapters/browser-content-verifier';
import { BrowserMachineTranslationGateway } from '@/features/messages/adapters/browser-machine-translation-gateway';
import { BrowserMessageBodyPreparer } from '@/features/messages/adapters/browser-message-body-preparer';
import { BrowserMessageSubmissionJournal } from '@/features/messages/adapters/browser-message-submission-journal';
import { ControlPlaneContentClient } from '@/features/messages/adapters/control-plane-content-client';
import { ControlPlaneMessagePublicationContentGateway } from '@/features/messages/adapters/control-plane-message-publication-content-gateway';
import { MatrixSdkHumanMessageGateway } from '@/features/messages/adapters/matrix-human-message-gateway';
import { MatrixMessageGateway } from '@/features/messages/adapters/matrix-message-gateway';
import { MatrixSdkMessageSource } from '@/features/messages/adapters/matrix-message-source';
import { HumanMessagePublisher } from '@/features/messages/application/human-message-publisher';
import { ControlPlaneModerationClient } from '@/features/moderation/adapters/control-plane-moderation-client';
import { ControlPlaneOnboardingClient } from '@/features/onboarding/adapters/control-plane-onboarding-client';
import { OnboardingCoordinator } from '@/features/onboarding/application/onboarding-coordinator';
import { MatrixAccountPreferencesGateway } from '@/features/preferences/adapters/matrix-account-preferences-gateway';
import { AccountPreferencesStore } from '@/features/preferences/application/account-preferences-store';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import { ControlPlanePrivateRoomClient } from '@/features/private-rooms/adapters/control-plane-private-room-client';
import { MatrixSdkPrivateRoomGateway } from '@/features/private-rooms/adapters/matrix-private-room-gateway';
import { ControlPlanePublicRoomDirectoryClient } from '@/features/room-directory/adapters/control-plane-public-room-directory-client';
import { ControlPlaneAccessManagementClient } from '@/features/security/adapters/control-plane-access-management-client';
import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { DesktopControlPlaneClient } from '@/features/session/adapters/desktop-control-plane-client';
import { DesktopMatrixGateway } from '@/features/session/adapters/desktop-matrix-gateway';
import { GuardedMatrixGateway } from '@/features/session/adapters/guarded-matrix-gateway';
import { MatrixWebGateway } from '@/features/session/adapters/matrix-web-gateway';
import { TauriMatrixSessionVault } from '@/features/session/adapters/tauri-matrix-session-vault';
import { ControlPlaneFrontendTelemetryClient } from '@/features/telemetry/adapters/control-plane-frontend-telemetry-client';
import { WindowBrowserGateway } from '@/shared/browser/window-browser-gateway';
import type { RuntimeConfig } from '@/shared/config/runtime-config';
import { readLanguagePreference } from '@/shared/i18n/i18n';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';
import { ControlPlaneAgentDirectoryClient } from '@/features/workspace/adapters/control-plane-agent-directory-client';
import { SessionRequestScope } from '@/shared/http/session-request-scope';
import { MatrixWorkspaceGateway } from '@/features/personal-workspace/adapters/matrix-workspace-gateway';
import { BrowserWorkspaceCache } from '@/features/personal-workspace/adapters/browser-workspace-cache';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { ControlPlaneInboxGateway } from '@/features/inbox/adapters/control-plane-inbox-gateway';
import { MatrixInboxSource } from '@/features/inbox/adapters/matrix-inbox-source';
import { InboxStore } from '@/features/inbox/application/inbox-store';
import { InboxProvider } from '@/features/inbox/ui/inbox-provider';
import { ReceptionOwnershipClient } from '@/features/desktop/adapters/reception-ownership-client';

export type CloudAppProvidersProps = {
  readonly config: RuntimeConfig;
  readonly localRuntime: DesktopRuntimeGateway;
};

export function CloudAppProviders({ config, localRuntime }: CloudAppProvidersProps) {
  const runtime = useMemo(() => createCloudRuntime(config, localRuntime), [config, localRuntime]);
  useVerticalSecurityDriver(runtime.matrixClients);
  return (
    <QueryClientProvider client={runtime.queryClient}>
      <AppServicesProvider services={runtime.services}>
        <AccountPreferencesProvider store={runtime.accountPreferences}>
          <PersonalWorkspaceProvider store={runtime.personalWorkspace}>
            <InboxProvider store={runtime.inbox}>
              <NetworkAgentLabelsProvider store={runtime.networkAgentLabels}>
                <RouterProvider router={router} />
              </NetworkAgentLabelsProvider>
            </InboxProvider>
          </PersonalWorkspaceProvider>
        </AccountPreferencesProvider>
      </AppServicesProvider>
    </QueryClientProvider>
  );
}

export function createCloudRuntime(
  config: RuntimeConfig,
  localRuntime: DesktopRuntimeGateway,
): CloudRuntimeComposition {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { refetchOnWindowFocus: true, retry: false } },
  });
  const privateRequests = new SessionRequestScope();
  const businessApi = { baseUrl: config.controlPlaneUrl, fetch: privateRequests.fetch };
  // Authentication must remain usable while private requests are being cancelled.
  const browserControlPlane = new ControlPlaneClient({ baseUrl: config.controlPlaneUrl });
  const controlPlane = localRuntime.isAvailable()
    ? new DesktopControlPlaneClient({ controlPlane: browserControlPlane, runtime: localRuntime })
    : browserControlPlane;
  const roomDirectory = new ControlPlanePublicRoomDirectoryClient(businessApi);
  const onboarding = new OnboardingCoordinator(
    new ControlPlaneOnboardingClient(businessApi),
    roomDirectory,
  );
  const telemetry = new ControlPlaneFrontendTelemetryClient({ baseUrl: config.controlPlaneUrl });
  const matrixClients = new MatrixClientRegistry();
  const secretStorageKeys = new MatrixSecretStorageKeyCache();
  const matrixCore = new MatrixWebGateway({
    baseUrl: config.matrixHomeserverUrl,
    deviceDisplayName: localRuntime.isAvailable() ? 'Agent Room Desktop' : 'Agent Room Web',
    onClientActivity: (client) => {
      matrixClients.refresh(client);
    },
    onClientChange: (client) => {
      matrixClients.replace(client);
    },
    secretStorageKeys,
    ...(localRuntime.isAvailable() ? { sessionVault: new TauriMatrixSessionVault() } : {}),
  });
  const matrix = new GuardedMatrixGateway(
    localRuntime.isAvailable()
      ? new DesktopMatrixGateway({ matrix: matrixCore, runtime: localRuntime })
      : matrixCore,
    window.sessionStorage,
    config.matrixHomeserverUrl,
  );
  const accountPreferences = new AccountPreferencesStore(
    new MatrixAccountPreferencesGateway(matrixClients),
    { language: readLanguagePreference(window.localStorage), lobbyView: 'scene' },
  );
  const personalWorkspace = new PersonalWorkspaceStore(
    new MatrixWorkspaceGateway(matrixClients),
    new BrowserWorkspaceCache(window.localStorage),
  );
  const lobby = new MatrixLobbyGateway(new MatrixSdkLobbySource(matrixClients));
  const networkAgentLabels = new NetworkAgentLabelStore(
    new ControlPlaneNetworkAgentLookup(businessApi),
  );
  const lobbyEntry = new PublicLobbyEntryCoordinator(
    new ControlPlanePublicLobbyEntryClient(businessApi),
    new MatrixSdkPublicLobbyEntryGateway(matrixClients),
  );
  const messages = new MatrixMessageGateway(new MatrixSdkMessageSource(matrixClients));
  const inbox = new InboxStore(
    new ControlPlaneInboxGateway(businessApi),
    new MatrixInboxSource(matrixClients),
    messages,
    personalWorkspace,
  );
  const messagePublisher = new HumanMessagePublisher({
    bodyPreparer: new BrowserMessageBodyPreparer(),
    content: new ControlPlaneMessagePublicationContentGateway(businessApi),
    journal: new BrowserMessageSubmissionJournal(window.localStorage, window.sessionStorage),
    matrix: new MatrixSdkHumanMessageGateway(matrixClients),
    session: controlPlane,
  });
  const directSessions = new ControlPlaneDirectSessionClient(businessApi);
  const directSessionCoordinator = new DirectSessionCoordinator(
    directSessions,
    new MatrixSdkDirectSessionGateway(matrixClients),
    new BrowserDirectBlockRegistry(window.localStorage),
  );
  const services: AppServices = {
    receptionOwnership: new ReceptionOwnershipClient(businessApi),
    accessManagement: new ControlPlaneAccessManagementClient(businessApi),
    agentDirectory: new ControlPlaneAgentDirectoryClient(businessApi),
    agentRosterPolicy: new ControlPlaneAgentRosterPolicy(businessApi.baseUrl, businessApi.fetch),
    automation: new ControlPlaneAutomationGrantClient(businessApi),
    config,
    content: new ControlPlaneContentClient(businessApi),
    contentVerifier: new BrowserContentVerifier(),
    controlPlane,
    directSessionCoordinator,
    directSessions,
    handoffs: new ControlPlaneHandoffGateway(businessApi),
    lobby,
    lobbyEntry,
    localRuntime,
    messagePublisher,
    messages,
    messageTranslation: new BrowserMachineTranslationGateway(),
    moderation: new ControlPlaneModerationClient(businessApi),
    onboarding,
    privateRoomMatrix: new MatrixSdkPrivateRoomGateway(matrixClients),
    privateRooms: new ControlPlanePrivateRoomClient(businessApi),
    roomDirectory,
    security: new MatrixSdkSecurityGateway(matrixClients, secretStorageKeys),
    session: {
      browser: new WindowBrowserGateway(),
      controlPlane,
      matrix,
      privateState: {
        clear: () => {
          privateRequests.clear();
          queryClient.clear();
        },
      },
    },
    telemetry,
  };
  return {
    accountPreferences,
    personalWorkspace,
    inbox,
    matrixClients,
    networkAgentLabels,
    queryClient,
    services,
  };
}

export type CloudRuntimeComposition = {
  readonly accountPreferences: AccountPreferencesStore;
  readonly personalWorkspace: PersonalWorkspaceStore;
  readonly inbox: InboxStore;
  readonly matrixClients: MatrixClientRegistry;
  readonly networkAgentLabels: NetworkAgentLabelStore;
  readonly queryClient: QueryClient;
  readonly services: AppServices;
};

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
