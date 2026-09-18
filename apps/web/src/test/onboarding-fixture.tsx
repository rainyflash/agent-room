import { readInviteHistory } from '@/features/desktop/domain/cli-invitation';
import { receptionFixture } from './reception-fixture';
import { SessionProvider, useSession } from '@/features/session/ui/session-provider';
import type { SessionDependencies, WebSession } from '@/features/session/domain/session';
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
import { bridgePhaseSchema } from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import type { ReceptionRecord } from '@/features/desktop/domain/reception-ownership';
import type { ProductDevice } from '@/features/security/domain/access-management';
import '@/features/lobby/ui/lobby-game.css';
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
const requestedPhase = bridgePhaseSchema.safeParse(
  new URLSearchParams(location.search).get('bridge'),
);
let bridge: BridgeRuntime = {
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
    phase: requestedPhase.success ? requestedPhase.data : 'ready',
  },
};
const unavailable = () =>
  Promise.resolve(err({ code: 'fixture.external_action_unavailable', retryable: false }));
function ready<T>(value: T) {
  return Promise.resolve(ok(value));
}
const desktop = !new URLSearchParams(location.search).has('browser');
const principal: WebSession = {
  authenticatedAtUnixMs: 1,
  expiresAtUnixMs: 1_900_000_000_000,
  displayName: 'Fixture operator',
  locale: 'en',
  matrixUserId: '@operator:matrix.test',
  principalId: '0198b601-77a3-74f1-b4f4-940f291951b9',
  recentlyAuthenticated: true,
};
const reception = receptionFixture(
  agent.agentId,
  lobby.catalogId,
  principal.principalId,
  '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
);
const receptionEnabled = new URLSearchParams(location.search).has('reception');
// ?ownership=idle|active|unreachable lists receptions this account runs on other computers,
// the way leftover or remote receptions appear in the local-agent panel.
const ownershipStatus = new URLSearchParams(location.search).get('ownership');
const ownershipRecords: readonly ReceptionRecord[] =
  ownershipStatus === null
    ? []
    : [
        {
          agentId: agent.agentId,
          catalogId: lobby.catalogId,
          roomId: '!fixture:matrix.test',
          sessionKey: '0198b601-77a5-74f1-b4f4-940f291951c1',
          displayName: 'Alpha 41 acceptance Claude Code',
          instanceId: '0198b601-77a5-74f1-b4f4-940f291951c2',
          deviceId: '0198b601-77a5-74f1-b4f4-940f291951c3',
          deviceLabel: 'Alpha 41 fresh-device acceptance',
          runId: '0198b601-77a5-74f1-b4f4-940f291951c4',
          status: ownershipStatus === 'idle' ? 'idle' : 'active',
          nextDeviceId: null,
          lastSeenUnixMs: ownershipStatus === 'unreachable' ? 1 : Date.now(),
          revision: 3,
          progress: { afterEventId: '$fixture-latest', pending: null, retry: null },
        },
      ];
const fixtureDevices: readonly ProductDevice[] = [
  {
    createdAtUnixMs: 1,
    deviceId: '0198b601-77a5-74f1-b4f4-940f291951d1',
    label: 'Studio desktop',
    lastSeenAtUnixMs: 1,
    matrixDeviceId: 'STUDIO',
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
  {
    createdAtUnixMs: 1,
    deviceId: '0198b601-77a5-74f1-b4f4-940f291951d2',
    label: 'Travel laptop',
    lastSeenAtUnixMs: 1,
    matrixDeviceId: 'LAPTOP',
    platform: 'macos',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
];
function inviteSessionKey(): string | null {
  return (
    readInviteHistory(window.localStorage, principal.principalId).identities[0]?.sessionKey ?? null
  );
}

let autostartEnabled = false;
let hostConfigured =
  new URLSearchParams(location.search).has('configured') ||
  window.localStorage.getItem('agent-room.fixture.host-configured') === 'true';
const gateway: DesktopRuntimeGateway = {
  ...(receptionEnabled ? reception.gateway : {}),
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
      cliConfiguration: { command: 'C:\\Agent Room\\agent-room.exe', args: [] },
      manualHostConfiguration: {
        args: [],
        command: 'C:\\Agent Room\\agent-room-mcp.exe',
        serverName: 'agent_room',
        transport: 'stdio',
      },
      platform: 'windows',
      updatesConfigured: true,
    }),
  retryBridge: () => {
    bridge = { ...bridge, session: null, lifecycle: { ...bridge.lifecycle, phase: 'authorized' } };
    return ready(bridge);
  },
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
    new URLSearchParams(location.search).get('setup') === 'failed'
      ? Promise.resolve(err({ code: 'codex.config_incompatible', retryable: true }))
      : ready({
          host: 'codex',
          action: hostConfigured ? 'unchanged' : 'create',
          target: 'fixture-config',
          originalDigest: '0'.repeat(64),
          desiredDigest: '1'.repeat(64),
          summaryCode: 'fixture.ready',
        }),
  applyHost: () => {
    hostConfigured = true;
    window.localStorage.setItem('agent-room.fixture.host-configured', 'true');
    return ready(undefined);
  },
  readHostSessions: () => {
    const state = new URLSearchParams(location.search).get('host');
    if (state === 'failed') return unavailable();
    return ready(
      state === 'ready'
        ? [
            {
              displayName: 'Scout',
              ...(receptionEnabled
                ? { sessionKey: reception.sessionKey, receptionOffer: reception.offer }
                : { sessionKey: inviteSessionKey() }),
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

function AuthenticatedOnboardingFixture() {
  const { snapshot } = useSession();
  const currentPrincipal = snapshot.context.principal;
  // Match production mounting: account restoration clears private queries before the workspace loads.
  if (!snapshot.matches('ready') || currentPrincipal === null) return null;
  return <OnboardingWorkspace principal={currentPrincipal} />;
}

async function bootstrapFixture() {
  await initializeI18n(window.localStorage, [
    new URLSearchParams(location.search).get('lang') ?? 'en',
  ]);
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
    receptionOwnership: {
      list: () => ready({ receptions: ownershipRecords, limited: false }),
      transfer: unavailable,
    },
    accessManagement: {
      listProductDevices: () => ready(ownershipRecords.length === 0 ? [] : fixtureDevices),
      listAgentInstances: unavailable,
      revokeAgentInstance: unavailable,
      revokeProductDevice: unavailable,
    },
    automation: { ...runtime.services.automation, list: () => ready([reception.grant]) },
    onboarding: new OnboardingCoordinator(
      { listAgents: () => ready([agent]), ensureDefaultAgent: () => ready(agent) },
      { list: () => ready([lobby]) },
    ),
  };
  const sessionDependencies: SessionDependencies = {
    ...runtime.services.session,
    controlPlane: {
      beginAuthentication: () => ready({ kind: 'session-established' }),
      logout: () => ready(undefined),
      readSession: () => ready(principal),
    },
    matrix: {
      disconnect: () => undefined,
      beginAuthentication: () => ready({ kind: 'session-established' }),
      logout: () => ready(undefined),
      restore: () =>
        ready({
          kind: 'connected',
          connection: {
            deviceId: 'FIXTURE',
            userId: principal.matrixUserId,
            disconnect: () => undefined,
            observe: () => () => undefined,
            waitUntilPrepared: () => ready(undefined),
          },
        }),
    },
  };
  const element = document.getElementById('root');
  if (!element) throw new Error('Onboarding fixture root is missing.');
  createRoot(element).render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <QueryClientProvider client={runtime.queryClient}>
          <AppServicesProvider services={services}>
            <SessionProvider dependencies={sessionDependencies}>
              <DesktopRuntimeProvider gateway={gateway}>
                <AuthenticatedOnboardingFixture />
                {new URLSearchParams(location.search).get('placement') === 'game' ? (
                  // The lobby's top-left placement, where the panel is narrowest.
                  <div className="lobby-game">
                    <DesktopRuntimeSurface placement="game" />
                  </div>
                ) : (
                  <DesktopRuntimeSurface />
                )}
              </DesktopRuntimeProvider>
            </SessionProvider>
          </AppServicesProvider>
        </QueryClientProvider>
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
void bootstrapFixture();
