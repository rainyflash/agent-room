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
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { AgentPresenceSource } from '@/features/lobby/adapters/matrix-presence-tracker';
import type { AgentRoom } from '@/features/workspace/domain/agent-directory';
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
// ?matrix=failed 停在“聊天登录没拿到”，看消息没连上时提示栈里的那一条。
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
// the way leftover or remote receptions appear on My agents. ?ownership=mixed lists one active
// reception and three stopped ones left behind by earlier acceptance runs.
const ownershipStatus = new URLSearchParams(location.search).get('ownership');
function ownershipRecord(
  index: number,
  status: ReceptionRecord['status'],
  lastSeenUnixMs: number,
): ReceptionRecord {
  const suffix = String(index).padStart(2, '0');
  return {
    agentId: agent.agentId,
    catalogId: index === 0 ? lobby.catalogId : `0198b601-77a5-74f1-b4f4-9402919519${suffix}`,
    roomId: '!fixture:matrix.test',
    sessionKey: `0198b601-77a5-74f1-b4f4-940f291951${suffix}`,
    displayName: `Alpha ${String(41 + index)} acceptance Claude Code`,
    instanceId: `0198b601-77a5-74f1-b4f4-940f291952${suffix}`,
    deviceId: `0198b601-77a5-74f1-b4f4-940f291953${suffix}`,
    deviceLabel: `Alpha ${String(41 + index)} fresh-device acceptance`,
    runId: `0198b601-77a5-74f1-b4f4-940f291954${suffix}`,
    status,
    nextDeviceId: null,
    lastSeenUnixMs,
    revision: 3,
    progress: { afterEventId: '$fixture-latest', pending: null, retry: null },
  };
}
const ownershipRecords: readonly ReceptionRecord[] =
  ownershipStatus === null
    ? []
    : ownershipStatus === 'mixed'
      ? [
          ownershipRecord(0, 'active', Date.now()),
          ownershipRecord(1, 'idle', 1),
          ownershipRecord(2, 'idle', 1),
          ownershipRecord(3, 'idle', 1),
        ]
      : [
          ownershipRecord(
            0,
            ownershipStatus === 'idle' ? 'idle' : 'active',
            ownershipStatus === 'unreachable' ? 1 : Date.now(),
          ),
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
// 夹具 Agent 在两个房间里（公共大厅和一个私人房间），“带进房间”还能选两个私人房间和一个公共大厅；
// 其中 Board notes 里你没有“自动发言”，带进去只能看。?bring=none 时带进去回 0 个连接。
const privateRoom = (
  catalogId: string,
  name: string,
  capabilities: PrivateRoom['members'][number]['permissions']['capabilities'],
): PrivateRoom => ({
  catalogId,
  description: '',
  matrixRoomId: `!${name.replace(/\s/gu, '-').toLowerCase()}:matrix.test`,
  members: [
    { permissions: { capabilities }, principalId: principal.principalId, status: 'joined' },
  ],
  name,
  ownerPrincipalId: principal.principalId,
  retentionDays: null,
  roomInstanceId: catalogId,
  status: 'active',
  version: 1,
});
const fixturePrivateRooms: readonly PrivateRoom[] = [
  privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e60', 'Weekly review', [
    'view',
    'speak',
    'invite',
    'manage',
    'automate',
  ]),
  privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e61', 'Design crit', ['view', 'speak', 'automate']),
  privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e62', 'Board notes', ['view', 'speak']),
];
const nightMarket = {
  ...lobby,
  catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e63',
  name: 'Night Market',
  slug: 'night-market',
};
let agentRooms: AgentRoom[] = [
  { archived: false, catalogId: lobby.catalogId, kind: 'public_lobby', name: lobby.name },
  {
    archived: false,
    catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e60',
    kind: 'private_room',
    name: 'Weekly review',
  },
];
const bringNothing = new URLSearchParams(location.search).get('bring') === 'none';
// ?presence=online|unavailable|offline 给夹具 Agent 一个 Matrix 在线状态。它的连接上一直记着在线
// （发过自动回复的就这样），?presence=offline 时卡片照在线状态说上次在线。
const presenceParam = new URLSearchParams(location.search).get('presence');
const fixturePresence: AgentPresenceSource | undefined =
  presenceParam === 'online' || presenceParam === 'unavailable' || presenceParam === 'offline'
    ? {
        observe: (userIds) =>
          new Map(
            userIds
              .filter((userId) => userId === agent.matrixUserId)
              .map((userId) => [
                userId,
                presenceParam === 'offline'
                  ? { offlineSeenAtUnixMs: Date.now() - 3 * 60 * 60 * 1000, state: presenceParam }
                  : { state: presenceParam },
              ]),
          ),
        subscribe: () => () => undefined,
      }
    : undefined;
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
    ...(fixturePresence === undefined ? {} : { agentPresence: fixturePresence }),
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
      listAgentRooms: () => ready([...agentRooms]),
      bringAgentIntoRoom: (_agentId: string, catalogId: string) => {
        if (bringNothing) return ready(0);
        const known = [
          ...fixturePrivateRooms.map((room) => ({ ...room, kind: 'private_room' as const })),
          { ...nightMarket, kind: 'public_lobby' as const },
          { ...lobby, kind: 'public_lobby' as const },
        ].find((room) => room.catalogId === catalogId);
        if (known !== undefined && !agentRooms.some((room) => room.catalogId === catalogId)) {
          agentRooms = [
            ...agentRooms,
            { archived: false, catalogId, kind: known.kind, name: known.name },
          ];
        }
        return ready(1);
      },
      leaveAgentRoom: (_agentId: string, catalogId: string) => {
        const before = agentRooms.length;
        agentRooms = agentRooms.filter((room) => room.catalogId !== catalogId);
        return ready(agentRooms.length < before);
      },
    },
    privateRooms: { ...runtime.services.privateRooms, list: () => ready(fixturePrivateRooms) },
    roomDirectory: { list: () => ready([lobby, nightMarket]) },
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
        matrixState === 'failed'
          ? Promise.resolve(
              err({
                boundary: 'matrix' as const,
                code: 'matrix_session.unavailable',
                offline: false,
                retryable: true,
              }),
            )
          : ready({ kind: 'session-established' as const }),
      logout: () => ready(undefined),
      restore: () =>
        matrixState === 'failed'
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
