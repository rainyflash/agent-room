import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import type {
  AccessManagementGateway,
  AgentInstance,
  ProductDevice,
} from '@/features/security/domain/access-management';
import type {
  AgentRecoveryGateway,
  AgentRecoveryResult,
} from '@/features/security/domain/agent-recovery';
import type {
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import type { BlockedAgent } from '@/features/direct-sessions/domain/direct-session';
import type { BlockedAgentsSource } from '@/features/security/ui/blocked-agents-panel';
import { SecurityWorkspace } from '@/features/security/ui/security-page';
import { SettingsLayout } from '@/features/settings/ui/settings-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { DeviceSigningStatus } from '@/shared/matrix/device-signing-status';
import { err, ok } from '@/shared/result';

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
let agentRecoveryState: AgentRecoveryResult['state'] = {
  userId: '@builder:agent-room.test',
  deviceId: 'BUILDER',
  identity: 'ready',
  recoveryAvailable: false,
  backupEnabled: false,
};
const agentRecovery: AgentRecoveryGateway = {
  sessions: () =>
    Promise.resolve(
      ok([
        {
          sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
          displayName: 'Builder test task',
          state: 'ready',
        },
      ]),
    ),
  execute: (_sessionId, request) => {
    if (request.action === 'enable')
      agentRecoveryState = { ...agentRecoveryState, recoveryAvailable: true, backupEnabled: true };
    return Promise.resolve(
      ok({
        state: agentRecoveryState,
        recoveryKey:
          request.action === 'enable' ? 'EsTc test-only recovery-key save-outside-this-app' : null,
      }),
    );
  },
};
const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;
const fixtureNow = Date.now();
/**
 * 两台在用的电脑，加上以前发布验收留下的：一台 10 天没用、一台 40 天没用（授权已过期）、
 * 一台撤销了的。撤销照真的走：撤完挪进“已撤销的”。`?reauth` 是没有最近登录过。
 */
let fixtureDevices: readonly ProductDevice[] = [
  {
    createdAtUnixMs: fixtureNow - 90 * DAY,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
    label: 'Studio workstation',
    lastSeenAtUnixMs: fixtureNow - 5 * MINUTE,
    matrixDeviceId: null,
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
  {
    createdAtUnixMs: fixtureNow - 60 * DAY,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
    label: 'Travel notebook',
    lastSeenAtUnixMs: fixtureNow - 2 * DAY,
    matrixDeviceId: null,
    platform: 'macos',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
  {
    createdAtUnixMs: fixtureNow - 12 * DAY,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
    label: 'Alpha 70 fresh-device acceptance',
    lastSeenAtUnixMs: fixtureNow - 10 * DAY,
    matrixDeviceId: null,
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
  {
    createdAtUnixMs: fixtureNow - 45 * DAY,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e51',
    label: 'Alpha 60 fresh-device acceptance',
    lastSeenAtUnixMs: fixtureNow - 40 * DAY,
    matrixDeviceId: null,
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  },
  {
    createdAtUnixMs: fixtureNow - 50 * DAY,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e52',
    label: 'Alpha 55 fresh-device acceptance',
    lastSeenAtUnixMs: fixtureNow - 48 * DAY,
    matrixDeviceId: null,
    platform: 'windows',
    revokedAtUnixMs: fixtureNow - 20 * DAY,
    trustState: 'revoked',
  },
];

function fixtureInstance(
  index: number,
  name: string,
  status: AgentInstance['status'],
  computer: ProductDevice,
  lastSeenAgoMs: number,
): AgentInstance {
  const suffix = String(index).padStart(2, '0');
  return {
    adapterType: 'cli',
    agentAvatarContentId: null,
    agentDisplayName: name,
    agentId: `0198b601-77a1-7bb8-83eb-a8fe68c97f${suffix}`,
    agentInstanceId: `0198b601-77a1-7bb8-83eb-a8fe68c97d${suffix}`,
    capabilityVersion: '1.0',
    createdAtUnixMs: computer.createdAtUnixMs,
    device: {
      deviceId: computer.deviceId,
      label: computer.label,
      platform: computer.platform,
      trustState: computer.trustState,
    },
    lastSeenAtUnixMs: fixtureNow - lastSeenAgoMs,
    matrixDeviceId: `AR_FIXTURE_${suffix}`,
    matrixDeviceRevokedAtUnixMs: status === 'revoked' ? fixtureNow - 20 * DAY : null,
    revokedAtUnixMs: status === 'revoked' ? fixtureNow - 20 * DAY : null,
    status,
  };
}

const [studio, travel, alpha70, alpha60, alpha55] = fixtureDevices as [
  ProductDevice,
  ProductDevice,
  ProductDevice,
  ProductDevice,
  ProductDevice,
];
const fixtureInstances: readonly AgentInstance[] = [
  fixtureInstance(1, 'Release architect', 'online', studio, MINUTE),
  fixtureInstance(2, 'Research scout', 'offline', travel, 2 * DAY),
  fixtureInstance(3, 'Alpha 70 acceptance agent', 'offline', alpha70, 10 * DAY),
  fixtureInstance(4, 'Alpha 60 acceptance agent', 'offline', alpha60, 40 * DAY),
  fixtureInstance(5, 'Alpha 55 acceptance agent', 'revoked', alpha55, 48 * DAY),
];

const accessManagement: AccessManagementGateway = {
  listAgentInstances: () => Promise.resolve(ok(fixtureInstances)),
  listProductDevices: () => Promise.resolve(ok(fixtureDevices)),
  revokeAgentInstance: () => Promise.resolve(err({ code: 'access.fixture', retryable: false })),
  revokeProductDevice: (deviceId) => {
    fixtureDevices = fixtureDevices.map((device) =>
      device.deviceId === deviceId
        ? { ...device, revokedAtUnixMs: Date.now(), trustState: 'revoked' }
        : device,
    );
    return Promise.resolve(ok({ matrixCleanup: 'complete', pendingAgentInstanceCount: 0 }));
  },
};
const search = new URLSearchParams(window.location.search);
/** `?signing=failed`：这台设备的自动签名出错了，点“重试”后先“正在准备”，再就绪。 */
const signingFailed = search.get('signing') === 'failed';
let currentDeviceSigned = !signingFailed;
const deviceSigning = new DeviceSigningStatus();
deviceSigning.set({ kind: signingFailed ? 'failed' : 'ready' });
deviceSigning.setRetry(() => {
  deviceSigning.set({ kind: 'working' });
  window.setTimeout(() => {
    currentDeviceSigned = true;
    deviceSigning.set({ kind: 'ready' });
  }, 1_000);
});

/**
 * 屏蔽的 Agent：一个服务器记着的，一个只在聊天账户忽略名单里、这台设备不知道名字的
 * （以前服务器没记上的那种）。`?blocks=none` 是一个也没有。
 */
let fixtureBlocks: readonly BlockedAgent[] =
  search.get('blocks') === 'none'
    ? []
    : [
        {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e61',
          blockedAtUnixMs: fixtureNow - 2 * DAY,
          displayName: 'Noisy helper #4821',
          matrixUserId: '@_agent_noisy:agent-room.test',
        },
        {
          agentId: null,
          blockedAtUnixMs: null,
          displayName: null,
          matrixUserId: '@_agent_0198b60177a17bb8:agent-room.test',
        },
      ];
const blockedAgents: BlockedAgentsSource = {
  listBlocked: () => Promise.resolve(ok(fixtureBlocks)),
  unblock: (entry) => {
    fixtureBlocks = fixtureBlocks.filter((block) => block.matrixUserId !== entry.matrixUserId);
    return Promise.resolve(ok(undefined));
  },
};

const security: MatrixSecurityGateway = {
  inspect: () => Promise.resolve(ok(securitySnapshot())),
  subscribe: (listener) => deviceSigning.subscribe(listener),
};

function securitySnapshot(): MatrixSecuritySnapshot {
  return Object.freeze({
    currentDeviceId: 'ALICE-WEB-2026',
    devices: Object.freeze([
      Object.freeze({
        current: true,
        deviceId: 'ALICE-WEB-2026',
        displayName: 'Edge on studio workstation',
        fingerprint: 'ZKLM YRQU QJHU FJNC YDZX FYNP DVXS KQNP WQJT AQKE',
        trust: currentDeviceSigned ? ('verified' as const) : ('unverified' as const),
        userId: '@alice:agent-room.test',
      }),
      Object.freeze({
        current: false,
        deviceId: 'ALICE-MACBOOK',
        displayName: 'MacBook Pro',
        fingerprint: 'EDAR HHKF XPUL VLMR LMGH YNFD KLQX VCHT YVUE AFAT',
        trust: 'signed' as const,
        userId: '@alice:agent-room.test',
      }),
      Object.freeze({
        current: false,
        deviceId: 'ALICE-TRAVEL',
        displayName: 'Travel browser',
        fingerprint: 'BXJC AVPU RYXP KQDR EWFA HGMP XTLM AZVC YHUT NPLQ',
        trust: 'unverified' as const,
        userId: '@alice:agent-room.test',
      }),
    ]),
    userId: '@alice:agent-room.test',
  });
}

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('安全中心测试根节点不存在。');
  }
  createRoot(root).render(
    <StrictMode>
      <I18nextProvider i18n={i18n}>
        <RouterTestProvider>
          <QueryClientProvider client={queryClient}>
            <SettingsLayout section="security">
              <SecurityWorkspace
                accessManagement={accessManagement}
                {...(search.has('agentRecovery') ? { agentRecovery } : {})}
                blockedAgents={blockedAgents}
                deviceSigning={deviceSigning}
                gateway={security}
                reauthentication={{
                  recentlyAuthenticated: !search.has('reauth'),
                  signInAgain: () => {
                    document.body.dataset.signInRequested = 'true';
                  },
                }}
              />
            </SettingsLayout>
          </QueryClientProvider>
        </RouterTestProvider>
      </I18nextProvider>
    </StrictMode>,
  );
}

void bootstrapFixture();
