import { receptionFixture } from './reception-fixture';
import { SessionProvider, useSession } from '@/features/session/ui/session-provider';
import type { SessionDependencies, WebSession } from '@/features/session/domain/session';
import { QueryClientProvider } from '@tanstack/react-query';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';
import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';
import { AppServicesProvider } from '@/app/app-services';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { createCloudRuntime } from '@/app/web-app-providers';
import type {
  BridgeRuntime,
  DesktopAgentTarget,
  DesktopRuntimeEventHandlers,
  DesktopRuntimeGateway,
  HostSessionDiagnostics,
  InvitationOffer,
  ReleaseUpdateCheck,
  ReleaseUpdateStatus,
} from '@/features/desktop/domain/desktop-runtime';
import { bridgePhaseSchema } from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import type { ReceptionRecord } from '@/features/desktop/domain/reception-ownership';
import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import { AccountWorkspacePage } from '@/features/workspace/ui/account-workspace-page';
import { SettingsLayout, SettingsSectionContent } from '@/features/settings/ui/settings-page';
import { isSettingsSection } from '@/features/settings/ui/settings-sections';
import { DesktopUpdateToast } from '@/features/updates/ui/desktop-update-toast';
import { MatrixConnectionToast } from '@/features/session/ui/matrix-connection-toast';
import { ToastStack } from '@agent-room/ui-system';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { RouterTestProvider } from '@/test/router-test-provider';
import type { MyAgentsFixtureControls } from './my-agents-fixture-controls';

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
const phase = requestedPhase.success ? requestedPhase.data : 'ready';
let bridge: BridgeRuntime = {
  // ?bridge=authorization_required 时带上设备码，像刚装好还没授权的电脑。
  authorization:
    phase === 'authorization_required'
      ? {
          expiresAtUnixMs: Date.now() + 600_000,
          promptId: 'fixture-authorization',
          userCode: 'ABCD-EFGH',
          verificationHost: 'identity.fixture.invalid',
        }
      : null,
  deviceReauthorizationAvailable: false,
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
    phase,
  },
};
const unavailable = () =>
  Promise.resolve(err({ code: 'fixture.external_action_unavailable', retryable: false }));
function ready<T>(value: T) {
  return Promise.resolve(ok(value));
}
const desktop = !new URLSearchParams(location.search).has('browser');
// ?matrix=signin 停在“等浏览器里登录完”，?matrix=failed 停在“浏览器登录页等太久没回来”，
// 看消息没连上时提示栈里的那一条。
const matrixState = new URLSearchParams(location.search).get('matrix');
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
// the way leftover or remote receptions appear on My agents.
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
// 我的 Agent 列表里的一份连接：Studio companion 在 Studio desktop 上在线。
const fixtureInstances: readonly AgentInstance[] = [
  {
    adapterType: 'agent-room-mcp',
    agentAvatarContentId: null,
    agentDisplayName: agent.displayName,
    agentId: agent.agentId,
    agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
    capabilityVersion: '1.0',
    createdAtUnixMs: 1,
    device: {
      deviceId: '0198b601-77a5-74f1-b4f4-940f291951d1',
      label: 'Studio desktop',
      platform: 'windows',
      trustState: 'verified',
    },
    lastSeenAtUnixMs: Date.now() - 5_000,
    matrixDeviceId: 'AR-STUDIO',
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status: 'online',
  },
];
// ?agents=none 模拟还没有 Agent 的新账户。
const fixtureAgents = new URLSearchParams(location.search).get('agents') === 'none' ? [] : [agent];
// ?update=current|failed|expired|translocated|none 看更新的几种情况：已是最新、没查成、
// 通道清单过期、Mac 上应用在只读位置运行、还没查过。不带时上次检查查到了 0.1.0-alpha.24。
const updateState = new URLSearchParams(location.search).get('update');
const fixtureUpdate: ReleaseUpdateCheck = {
  available: updateState !== 'current',
  channel: 'testing',
  currentVersion: '0.1.0-alpha.23',
  rollback: false,
  sequence: updateState === 'current' ? 23 : 24,
  targetVersion: updateState === 'current' ? '0.1.0-alpha.23' : '0.1.0-alpha.24',
};
const updateFailureCode =
  updateState === 'failed'
    ? 'desktop.update.manifest_network'
    : updateState === 'expired'
      ? 'desktop.update.manifest_expired'
      : null;
const updateStatus: ReleaseUpdateStatus | null =
  updateState === 'none'
    ? null
    : {
        channel: 'testing',
        checkedAtUnixMs: Date.now() - 5 * 60_000,
        check: updateFailureCode === null ? fixtureUpdate : null,
        failure:
          updateFailureCode === null
            ? null
            : { code: updateFailureCode, retryable: updateState === 'failed' },
      };
// 订阅时交来的处理函数：托盘菜单的请求和下载进度都经它送进页面。
let runtimeHandlers: DesktopRuntimeEventHandlers | null = null;
// 接入对话框挂在连接服务上的人物；arriveAgent() 让一个新会话接走它。
let parkedInvitation: InvitationOffer | null = null;
const arrivedSessions: HostSessionDiagnostics[] = [];
const fixtureControls: MyAgentsFixtureControls = {
  arriveAgent: () => {
    const sessionKey = parkedInvitation?.sessionKey ?? null;
    parkedInvitation = null;
    arrivedSessions.push({
      displayName: 'Scout',
      sessionKey,
      roomId: '!fixture:matrix.test',
      session: {
        sessionId: `0198b601-77a6-7bb8-83eb-a8fe68c97e${String(50 + arrivedSessions.length)}`,
        state: 'ready',
        agentId: '0198b601-77a6-7bb8-83eb-a8fe68c97e49',
        errorCode: null,
      },
      lastInboxReadAgoMs: 1_000,
      lastMessageReceivedAgoMs: null,
      lastMessageSentAgoMs: null,
    });
  },
  parkedInvitation: () => parkedInvitation?.sessionKey ?? null,
  requestUpdate: () => {
    runtimeHandlers?.onUpdateRequested?.();
  },
};
Object.defineProperty(window, '__agentRoomFixtureControls', {
  configurable: true,
  value: fixtureControls,
});

let autostartEnabled = false;
const gateway: DesktopRuntimeGateway = {
  ...(receptionEnabled ? reception.gateway : {}),
  beginHumanAuthentication: unavailable,
  beginMatrixAuthentication: unavailable,
  clearHumanSession: unavailable,
  restoreHumanSession: unavailable,
  sendControlPlaneRequest: unavailable,
  bootstrapDefaultAgent: () => ready(target),
  configureAgentRuntime: (next) => ready(next),
  isAvailable: () => desktop,
  snapshot: () =>
    ready({
      agentTarget: target,
      autostartEnabled,
      bridge,
      currentVersion: '0.1.0-alpha.23',
      updateStatus,
      appTranslocated: updateState === 'translocated',
      deepLink: null,
      cliConfiguration: { command: 'C:\\Agent Room\\agent-room.exe', args: [] },
      manualHostConfiguration: {
        args: [],
        command: 'C:\\Agent Room\\agent-room-mcp.exe',
        serverName: 'agent_room',
        transport: 'stdio',
      },
      platform: updateState === 'translocated' ? 'macos' : 'windows',
      updatesConfigured: true,
    }),
  retryBridge: () => {
    bridge = { ...bridge, session: null, lifecycle: { ...bridge.lifecycle, phase: 'authorized' } };
    return ready(bridge);
  },
  reauthorizeBridge: () => {
    bridge = {
      ...bridge,
      session: null,
      deviceReauthorizationAvailable: false,
      lifecycle: { ...bridge.lifecycle, phase: 'starting' },
    };
    return ready(bridge);
  },
  setAutostart: (enabled) => {
    autostartEnabled = enabled;
    return ready(enabled);
  },
  openAuthorization: () => ready(undefined),
  openLogs: () => ready(undefined),
  offerInvitation: (invitation) => {
    parkedInvitation = invitation;
    return ready({ invitation, expiresInMs: 600_000 });
  },
  withdrawInvitation: (sessionKey) => {
    if (parkedInvitation?.sessionKey === sessionKey) parkedInvitation = null;
    return ready(undefined);
  },
  // ?host=ready|failed：任务连接诊断，接入面板与后台回复的用例都靠它。
  readHostSessions: () => {
    const state = new URLSearchParams(location.search).get('host');
    if (state === 'failed') return unavailable();
    const present: HostSessionDiagnostics[] =
      state === 'ready'
        ? [
            {
              displayName: 'Scout',
              ...(receptionEnabled
                ? { sessionKey: reception.sessionKey, receptionOffer: reception.offer }
                : { sessionKey: null }),
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
        : [];
    return ready([...present, ...arrivedSessions]);
  },
  // 下载到一半停住：真的装好了桌面端会重启，页面看不到结果。
  installUpdate: () => {
    runtimeHandlers?.onUpdateProgress?.({
      phase: 'downloading',
      downloadedBytes: 21_000_000,
      totalBytes: 42_000_000,
    });
    return new Promise(() => undefined);
  },
  checkUpdate: (channel) =>
    updateFailureCode === null
      ? ready({ ...fixtureUpdate, channel })
      : Promise.resolve(err({ code: updateFailureCode, retryable: updateState === 'failed' })),
  subscribe: (handlers) => {
    runtimeHandlers = handlers;
    return ready(() => {
      if (runtimeHandlers === handlers) runtimeHandlers = null;
    });
  },
};

// ?settings=general|account|this-computer|about 显示设置页的这一节（安全一节有自己的测试页）。
const settingsSection = new URLSearchParams(location.search).get('settings');

function AuthenticatedMyAgentsFixture() {
  const { snapshot } = useSession();
  const currentPrincipal = snapshot.context.principal;
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  // Match production mounting: account restoration clears private queries before the workspace loads.
  // 停在消息没连上时（?matrix=），生产上页面只看控制面，照样显示。
  const mounted =
    matrixState === null ? snapshot.matches('ready') : snapshot.context.controlStatus === 'ready';
  if (!mounted || currentPrincipal === null) return null;
  if (
    settingsSection !== null &&
    isSettingsSection(settingsSection) &&
    settingsSection !== 'security'
  ) {
    return (
      <SettingsLayout section={settingsSection}>
        <SettingsSectionContent section={settingsSection} />
      </SettingsLayout>
    );
  }
  return (
    <AccountWorkspacePage
      onSelectAgent={setSelectedAgentId}
      principal={currentPrincipal}
      selectedAgentId={selectedAgentId}
    />
  );
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
      macosDownloadUrl: 'https://download.fixture.invalid/macos.dmg',
    },
    gateway,
  );
  const services = {
    ...runtime.services,
    account: {
      exportData: () => ready({ fileName: 'agent-room-account-fixture.json', json: '{}\n' }),
      requestDeletion: () => ready(undefined),
    },
    receptionOwnership: {
      list: () => ready({ receptions: ownershipRecords, limited: false }),
      transfer: unavailable,
    },
    agentDirectory: {
      listOwnedAgents: () => ready(fixtureAgents),
      deleteAgent: unavailable,
    },
    accessManagement: {
      listProductDevices: () => ready(fixtureDevices),
      listAgentInstances: () => ready(fixtureAgents.length === 0 ? [] : fixtureInstances),
      revokeAgentInstance: unavailable,
      revokeProductDevice: unavailable,
    },
    automation: { ...runtime.services.automation, list: () => ready([reception.grant]) },
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
      beginAuthentication: () =>
        matrixState === 'signin'
          ? new Promise(() => undefined)
          : matrixState === 'failed'
            ? Promise.resolve(
                err({
                  boundary: 'matrix' as const,
                  code: 'desktop.matrix_session.loopback_timeout',
                  offline: false,
                  retryable: true,
                }),
              )
            : ready({ kind: 'session-established' as const }),
      logout: () => ready(undefined),
      restore: () =>
        matrixState === 'signin' || matrixState === 'failed'
          ? ready({ kind: 'authentication-required' as const })
          : ready({
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
  if (!element) throw new Error('My agents fixture root is missing.');
  createRoot(element).render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <QueryClientProvider client={runtime.queryClient}>
          <AppServicesProvider services={services}>
            <AccountPreferencesProvider store={runtime.accountPreferences}>
              <PersonalWorkspaceProvider store={runtime.personalWorkspace}>
                <SessionProvider dependencies={sessionDependencies}>
                  <DesktopRuntimeProvider gateway={gateway}>
                    <AuthenticatedMyAgentsFixture />
                    <ToastStack label="Notifications">
                      <MatrixConnectionToast hidden={false} />
                      <DesktopUpdateToast />
                    </ToastStack>
                  </DesktopRuntimeProvider>
                </SessionProvider>
              </PersonalWorkspaceProvider>
            </AccountPreferencesProvider>
          </AppServicesProvider>
        </QueryClientProvider>
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
void bootstrapFixture();
