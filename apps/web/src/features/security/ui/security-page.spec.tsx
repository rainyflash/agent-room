// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
  MatrixVerificationSession,
  MatrixVerificationSnapshot,
} from '@/features/security/domain/matrix-security';
import { SecurityWorkspace } from '@/features/security/ui/security-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(cleanup);

describe('SecurityWorkspace', () => {
  it('展示真实 Matrix 安全姿态与设备账本', async () => {
    const gateway = securityGateway(readySnapshot());

    renderWorkspace(gateway);

    expect(await screen.findByRole('heading', { name: 'This account is protected' })).toBeVisible();
    expect(screen.getByText('@alice:agent-room.test')).toBeVisible();
    expect(screen.getByText('Alice browser')).toBeVisible();
    expect(screen.getByText('ED25519 CURRENT FINGERPRINT')).toBeVisible();
    expect(screen.getAllByText('Verified')).toHaveLength(1);
  });

  it('只能通过官方 SAS 会话确认未验证设备', async () => {
    const user = userEvent.setup();
    const session = verificationSession({
      sas: {
        decimals: [1024, 2048, 4096],
        emojis: [
          { label: 'Dog', symbol: '🐶' },
          { label: 'Rocket', symbol: '🚀' },
        ],
      },
      stage: 'comparing',
    });
    const beginVerification = vi.fn(async () => ok(session.value));
    const gateway = securityGateway(
      {
        ...blockedSnapshot(),
        blockers: ['cross_signing_not_ready', 'current_device_unverified'],
        crossSigningReady: false,
      },
      { beginVerification },
    );

    renderWorkspace(gateway);
    await user.click((await screen.findAllByRole('button', { name: 'Verify' }))[0]!);

    const dialog = await screen.findByRole('dialog', { name: 'Verify a Matrix device' });
    expect(within(dialog).getByText('🐶')).toBeVisible();
    expect(within(dialog).getByText('🚀')).toBeVisible();
    expect(beginVerification).toHaveBeenCalledWith({ targetDeviceId: 'ALICE-WEB' });

    await user.click(within(dialog).getByRole('button', { name: 'They match' }));
    expect(session.confirm).toHaveBeenCalledOnce();
  });

  it('全新账户先建立交叉签名身份，不把首次设备引向无解的 SAS 请求', async () => {
    const user = userEvent.setup();
    const establishIdentity = vi.fn(async () => ok(undefined));
    const gateway = securityGateway(
      {
        ...blockedSnapshot(),
        blockers: ['cross_signing_missing', 'current_device_unverified'] as const,
        crossSigningIdentityExists: false,
        crossSigningReady: false,
      },
      { establishIdentity },
    );

    renderWorkspace(gateway);
    await user.click(await screen.findByRole('button', { name: 'Establish encrypted identity' }));

    expect(establishIdentity).toHaveBeenCalledOnce();
    expect(screen.queryByRole('button', { name: 'Verify' })).not.toBeInTheDocument();
  });

  it('恢复密钥只在当前界面显示一次且不会写入浏览器存储', async () => {
    const user = userEvent.setup();
    const recoveryKey = 'EsTc r7Cy 4abc one-time recovery key';
    const setupRecovery = vi.fn(async () => ok({ recoveryKey }));
    const gateway = securityGateway(missingRecoverySnapshot(), { setupRecovery });

    renderWorkspace(gateway);
    await user.click(await screen.findByRole('button', { name: 'Set up recovery' }));
    await user.type(screen.getByLabelText('Recovery passphrase'), 'correct horse battery staple');
    await user.type(screen.getByLabelText('Confirm passphrase'), 'correct horse battery staple');
    await user.click(screen.getByRole('button', { name: 'Create recovery' }));

    expect(await screen.findByText(recoveryKey)).toBeVisible();
    expect(setupRecovery).toHaveBeenCalledWith({ passphrase: 'correct horse battery staple' });
    expect(storageValues(window.localStorage)).not.toContain(recoveryKey);

    await user.click(screen.getByRole('button', { name: 'I saved the recovery key' }));
    await waitFor(() => expect(screen.queryByText(recoveryKey)).not.toBeInTheDocument());
  });
});

function renderWorkspace(gateway: MatrixSecurityGateway) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <SecurityWorkspace gateway={gateway} onBack={() => undefined} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function securityGateway(
  snapshot: MatrixSecuritySnapshot,
  overrides: Partial<MatrixSecurityGateway> = {},
): MatrixSecurityGateway {
  const base: MatrixSecurityGateway = {
    acceptIncomingVerification: async () =>
      err({ code: 'security.verification_unavailable', retryable: false }),
    beginVerification: async () =>
      err({ code: 'security.verification_unavailable', retryable: false }),
    declineIncomingVerification: async () =>
      err({ code: 'security.verification_unavailable', retryable: false }),
    establishIdentity: async () =>
      err({ code: 'security.identity_bootstrap_failed', retryable: true }),
    getIncomingVerification: () => null,
    inspect: async () => ok(snapshot),
    recover: async () => err({ code: 'security.recovery_failed', retryable: true }),
    setupRecovery: async () => err({ code: 'security.recovery_setup_failed', retryable: true }),
    subscribe: () => () => undefined,
  };
  return { ...base, ...overrides };
}

function readySnapshot(): MatrixSecuritySnapshot {
  return Object.freeze({
    backup: 'ready',
    blockers: [],
    crossSigningIdentityExists: true,
    crossSigningReady: true,
    cryptoVersion: 'Rust SDK 1.0',
    currentDeviceId: 'ALICE-WEB',
    devices: [
      {
        current: true,
        deviceId: 'ALICE-WEB',
        displayName: 'Alice browser',
        fingerprint: 'ED25519 CURRENT FINGERPRINT',
        trust: 'verified' as const,
        userId: '@alice:agent-room.test',
      },
    ],
    excludedDeviceCount: 0,
    kind: 'ready',
    roomEncryption: 'not_checked',
    secretStorageReady: true,
    sendAllowed: true,
    userId: '@alice:agent-room.test',
  });
}

function blockedSnapshot(): MatrixSecuritySnapshot {
  return Object.freeze({
    ...readySnapshot(),
    blockers: ['current_device_unverified'] as const,
    devices: [
      {
        ...readySnapshot().devices[0]!,
        trust: 'unverified' as const,
      },
    ],
    kind: 'blocked',
    sendAllowed: false,
  });
}

function missingRecoverySnapshot(): MatrixSecuritySnapshot {
  return Object.freeze({
    ...readySnapshot(),
    backup: 'missing',
    blockers: ['backup_missing', 'secret_storage_missing'] as const,
    kind: 'action_required',
    secretStorageReady: false,
  });
}

function verificationSession(snapshot: MatrixVerificationSnapshot) {
  const listeners = new Set<() => void>();
  const confirm = vi.fn(async () => ok(undefined));
  const value: MatrixVerificationSession = {
    activate: vi.fn(),
    cancel: async () => ok(undefined),
    confirm,
    deactivate: vi.fn(),
    getSnapshot: () => snapshot,
    mismatch: vi.fn(),
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  return { confirm, value };
}

function storageValues(storage: Storage): readonly string[] {
  return Array.from({ length: storage.length }, (_, index) => storage.key(index))
    .filter((key): key is string => key !== null)
    .map((key) => storage.getItem(key) ?? '');
}
