import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type {
  AgentRecoveryGateway,
  AgentRecoveryResult,
} from '@/features/security/domain/agent-recovery';
import type {
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
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
const accessManagement: AccessManagementGateway = {
  listAgentInstances: () =>
    Promise.resolve(
      ok([
        {
          adapterType: 'codex',
          agentAvatarContentId: null,
          agentDisplayName: 'Release architect',
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
          agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
          capabilityVersion: '1.0',
          createdAtUnixMs: 1_756_118_400_000,
          device: {
            deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
            label: 'Studio workstation',
            platform: 'windows',
            trustState: 'verified',
          },
          lastSeenAtUnixMs: 1_756_122_000_000,
          matrixDeviceId: 'AR_CODEX_STUDIO',
          matrixDeviceRevokedAtUnixMs: null,
          revokedAtUnixMs: null,
          status: 'online',
        },
        {
          adapterType: 'claude-code',
          agentAvatarContentId: null,
          agentDisplayName: 'Research scout',
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e45',
          agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
          capabilityVersion: '1.0',
          createdAtUnixMs: 1_756_032_000_000,
          device: {
            deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
            label: 'Travel notebook',
            platform: 'macos',
            trustState: 'verified',
          },
          lastSeenAtUnixMs: 1_756_121_400_000,
          matrixDeviceId: 'AR_CLAUDE_TRAVEL',
          matrixDeviceRevokedAtUnixMs: null,
          revokedAtUnixMs: null,
          status: 'offline',
        },
      ]),
    ),
  listProductDevices: () =>
    Promise.resolve(
      ok([
        {
          createdAtUnixMs: 1_756_032_000_000,
          deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
          label: 'Studio workstation',
          lastSeenAtUnixMs: 1_756_122_000_000,
          matrixDeviceId: 'WEB_DEVICE',
          platform: 'windows',
          revokedAtUnixMs: null,
          trustState: 'verified',
        },
        {
          createdAtUnixMs: 1_756_118_400_000,
          deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
          label: 'Travel notebook',
          lastSeenAtUnixMs: null,
          matrixDeviceId: null,
          platform: 'macos',
          revokedAtUnixMs: null,
          trustState: 'pending',
        },
      ]),
    ),
  revokeAgentInstance: () => Promise.resolve(err({ code: 'access.fixture', retryable: false })),
  revokeProductDevice: () => Promise.resolve(err({ code: 'access.fixture', retryable: false })),
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
                deviceSigning={deviceSigning}
                gateway={security}
              />
            </SettingsLayout>
          </QueryClientProvider>
        </RouterTestProvider>
      </I18nextProvider>
    </StrictMode>,
  );
}

void bootstrapFixture();
