import { QueryClientProvider } from '@tanstack/react-query';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';
import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';
import { AppServicesProvider } from '@/app/app-services';
import { createCloudRuntime } from '@/app/web-app-providers';
import type {
  BridgeRuntime,
  DesktopAgentTarget,
  DesktopRuntimeGateway,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { OnboardingCoordinator } from '@/features/onboarding/application/onboarding-coordinator';
import { OnboardingWorkspace } from '@/features/onboarding/ui/onboarding-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { RouterTestProvider } from '@/test/router-test-provider';

const agent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  avatarContentId: null,
  description: 'Your companion in the shared room.',
  displayName: 'Studio companion',
  matrixUserId: '@companion:matrix.test',
  registeredAtUnixMs: 1,
  slug: 'studio-companion',
  visibility: 'private' as const,
};
const lobby = {
  activeInstanceCount: 1,
  catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
  description: 'A place to meet people and their agents.',
  language: 'en',
  name: 'Builders Exchange',
  onlineAgentCount: 8,
  slug: 'builders',
};
const target: DesktopAgentTarget = {
  agentId: agent.agentId,
  lobbyLanguage: 'en',
  publicLobbyCatalogId: lobby.catalogId,
};
const bridge: BridgeRuntime = {
  authorization: null,
  session: {
    agentId: agent.agentId,
    instanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
    matrixRoomId: '!fixture:matrix.test',
  },
  lifecycle: {
    automaticRestartCount: 0,
    changedAtUnixMs: 1,
    diagnosticCode: null,
    lastFailureCode: null,
    lastExitCode: null,
    nextRetryAtUnixMs: null,
    ownership: 'managed',
    phase: 'ready',
  },
};
const unavailable = () =>
  Promise.resolve(err({ code: 'fixture.external_action_unavailable', retryable: false }));
function ready<T>(value: T) {
  return Promise.resolve(ok(value));
}
const desktop = !new URLSearchParams(location.search).has('browser');
let autostartEnabled = false;
const gateway: DesktopRuntimeGateway = {
  beginHumanAuthentication: unavailable,
  beginMatrixAuthentication: unavailable,
  clearHumanSession: unavailable,
  restoreHumanSession: unavailable,
  bootstrapDefaultAgent: () => ready(target),
  configureAgentRuntime: (next) => ready(next),
  isAvailable: () => desktop,
  snapshot: () =>
    ready({
      agentTarget: target,
      autostartEnabled,
      bridge,
      deepLink: null,
      manualHostConfiguration: {
        args: [],
        command: 'C:\\Agent Room\\agent-room-mcp.exe',
        serverName: 'agent_room',
        transport: 'stdio',
      },
      platform: 'windows',
      updatesConfigured: true,
    }),
  retryBridge: () => ready(bridge),
  setAutostart: (enabled) => {
    autostartEnabled = enabled;
    return ready(enabled);
  },
  openAuthorization: unavailable,
  readLobby: unavailable,
  detectHosts: () =>
    ready([
      {
        host: 'codex',
        installed: true,
        configurable: true,
        mechanism: 'mcp-stdio',
        diagnosticCode: 'fixture.host.detected',
      },
    ]),
  planHost: () =>
    ready({
      host: 'codex',
      action: 'create',
      target: 'fixture-config',
      originalDigest: '0'.repeat(64),
      desiredDigest: '1'.repeat(64),
      summaryCode: 'fixture.ready',
    }),
  applyHost: () => ready(undefined),
  readHostSessions: () => {
    const state = new URLSearchParams(location.search).get('host');
    if (state === 'failed') return unavailable();
    return ready(
      state === 'ready'
        ? [
            {
              displayName: 'Scout',
              session: {
                sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
                state: 'ready',
                agentId: agent.agentId,
                errorCode: null,
              },
              lastInboxReadAgoMs: 1_000,
              lastMessageReceivedAgoMs: 2_000,
              lastMessageSentAgoMs: null,
            },
          ]
        : [],
    );
  },
  installUpdate: unavailable,
  checkUpdate: (channel) =>
    ready({
      available: true,
      channel,
      currentVersion: '0.1.0-alpha.23',
      targetVersion: '0.1.0-alpha.24',
      rollback: false,
      sequence: 24,
    }),
  subscribe: () => ready(() => undefined),
};

async function bootstrapFixture() {
  await initializeI18n(window.localStorage, ['en']);
  const runtime = createCloudRuntime(
    {
      controlPlaneUrl: 'https://api.fixture.invalid',
      matrixHomeserverUrl: 'https://matrix.fixture.invalid',
      registrationMode: 'open-email',
      windowsDownloadUrl: 'https://download.fixture.invalid/windows.exe',
    },
    gateway,
  );
  const services = {
    ...runtime.services,
    onboarding: new OnboardingCoordinator(
      { listAgents: () => ready([agent]), ensureDefaultAgent: () => ready(agent) },
      { list: () => ready([lobby]) },
    ),
  };
  const element = document.getElementById('root');
  if (!element) throw new Error('Onboarding fixture root is missing.');
  createRoot(element).render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <QueryClientProvider client={runtime.queryClient}>
          <AppServicesProvider services={services}>
            <DesktopRuntimeProvider gateway={gateway}>
              <OnboardingWorkspace
                principal={{
                  authenticatedAtUnixMs: 1,
                  expiresAtUnixMs: 1_900_000_000_000,
                  displayName: 'Fixture operator',
                  locale: 'en',
                  matrixUserId: '@operator:matrix.test',
                  principalId: '0198b601-77a3-74f1-b4f4-940f291951b9',
                  recentlyAuthenticated: true,
                }}
              />
              <DesktopRuntimeSurface />
            </DesktopRuntimeProvider>
          </AppServicesProvider>
        </QueryClientProvider>
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
void bootstrapFixture();
