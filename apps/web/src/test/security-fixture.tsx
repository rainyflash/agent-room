import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

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
const security: MatrixSecurityGateway = {
  beginVerification: async ({ targetDeviceId } = {}) =>
    targetDeviceId === undefined
      ? err({ code: 'security.verification_unavailable', retryable: false })
      : ok(createVerificationSession()),
  inspect: async () => ok(securitySnapshot),
  recover: async (_request, onProgress) => {
    onProgress?.({ stage: 'fetching' });
    onProgress?.({ failures: 0, imported: 248, stage: 'importing', total: 248 });
    return ok({ imported: 248, total: 248 });
  },
  setupRecovery: async () =>
    err({ code: 'security.recovery_already_configured', retryable: false }),
  subscribe: () => () => undefined,
};

const securitySnapshot: MatrixSecuritySnapshot = Object.freeze({
  backup: 'locked',
  blockers: ['backup_locked'],
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
      decimals: Object.freeze([1265, 6842, 4519]),
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
    cancel: async () => {
      publish(Object.freeze({ cancellationCode: 'm.user', stage: 'cancelled' }));
      return ok(undefined);
    },
    confirm: async () => {
      publish(Object.freeze({ stage: 'verified' }));
      return ok(undefined);
    },
    dispose: () => listeners.clear(),
    getSnapshot: () => snapshot,
    mismatch: () =>
      publish(Object.freeze({ cancellationCode: 'm.mismatched_sas', stage: 'cancelled' })),
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
          <SecurityWorkspace gateway={security} onBack={() => undefined} />
        </QueryClientProvider>
      </I18nextProvider>
    </StrictMode>,
  );
}

void bootstrapFixture();
