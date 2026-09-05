import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type {
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
  MatrixVerificationSession,
  MatrixVerificationSnapshot,
} from '@/features/security/domain/matrix-security';
import { SecurityWorkspace } from '@/features/security/ui/security-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
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
const security: MatrixSecurityGateway = {
  acceptIncomingVerification: () =>
    Promise.resolve(err({ code: 'security.verification_unavailable', retryable: false })),
  beginVerification: ({ targetDeviceId } = {}) =>
    Promise.resolve(
      targetDeviceId === undefined
        ? err({ code: 'security.verification_unavailable', retryable: false })
        : ok(createVerificationSession()),
    ),
  declineIncomingVerification: () =>
    Promise.resolve(err({ code: 'security.verification_unavailable', retryable: false })),
  establishIdentity: () => Promise.resolve(ok(undefined)),
  getIncomingVerification: () => null,
  inspect: () => Promise.resolve(ok(securitySnapshot)),
  recover: (_request, onProgress) => {
    onProgress?.({ stage: 'fetching' });
    onProgress?.({ failures: 0, imported: 248, stage: 'importing', total: 248 });
    return Promise.resolve(ok({ imported: 248, total: 248 }));
  },
  setupRecovery: () =>
    Promise.resolve(err({ code: 'security.recovery_already_configured', retryable: false })),
  subscribe: () => () => undefined,
};

const securitySnapshot: MatrixSecuritySnapshot = Object.freeze({
  backup: 'locked',
  blockers: ['backup_locked'] as const,
  crossSigningIdentityExists: true,
  crossSigningReady: true,
  cryptoVersion: 'Rust SDK 0.12.0',
  currentDeviceId: 'ALICE-WEB-2026',
  devices: Object.freeze([
    Object.freeze({
      current: true,
      deviceId: 'ALICE-WEB-2026',
      displayName: 'Edge on studio workstation',
      fingerprint: 'ZKLM YRQU QJHU FJNC YDZX FYNP DVXS KQNP WQJT AQKE',
      trust: 'verified' as const,
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
  excludedDeviceCount: 1,
  kind: 'action_required',
  roomEncryption: 'not_checked',
  secretStorageReady: true,
  sendAllowed: true,
  userId: '@alice:agent-room.test',
});

function createVerificationSession(): MatrixVerificationSession {
  const listeners = new Set<() => void>();
  let snapshot: MatrixVerificationSnapshot = Object.freeze({
    sas: Object.freeze({
      decimals: Object.freeze([1265, 6842, 4519] as const),
      emojis: Object.freeze([
        Object.freeze({ label: 'Dog', symbol: '🐶' }),
        Object.freeze({ label: 'Tree', symbol: '🌳' }),
        Object.freeze({ label: 'Rocket', symbol: '🚀' }),
        Object.freeze({ label: 'Headphones', symbol: '🎧' }),
        Object.freeze({ label: 'Moon', symbol: '🌙' }),
        Object.freeze({ label: 'Book', symbol: '📕' }),
        Object.freeze({ label: 'Bicycle', symbol: '🚲' }),
      ]),
    }),
    stage: 'comparing',
  });
  const publish = (next: MatrixVerificationSnapshot): void => {
    snapshot = next;
    for (const listener of listeners) {
      listener();
    }
  };

  return {
    activate: () => undefined,
    cancel: () => {
      publish(Object.freeze({ cancellationCode: 'm.user', stage: 'cancelled' }));
      return Promise.resolve(ok(undefined));
    },
    confirm: () => {
      publish(Object.freeze({ stage: 'verified' }));
      return Promise.resolve(ok(undefined));
    },
    deactivate: () => undefined,
    getSnapshot: () => snapshot,
    mismatch: () => {
      publish(Object.freeze({ cancellationCode: 'm.mismatched_sas', stage: 'cancelled' }));
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
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
        <QueryClientProvider client={queryClient}>
          <SecurityWorkspace
            accessManagement={accessManagement}
            gateway={security}
            onBack={() => undefined}
          />
        </QueryClientProvider>
      </I18nextProvider>
    </StrictMode>,
  );
}

void bootstrapFixture();
