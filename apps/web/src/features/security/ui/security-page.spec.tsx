// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

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

    const heading = await screen.findByRole('heading', { name: 'This account is protected' });
    await waitFor(() => {
      expect(heading).toBeVisible();
    });
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
    const beginVerification = vi.fn(() => Promise.resolve(ok(session.value)));
    const gateway = securityGateway(
      {
        ...blockedSnapshot(),
        blockers: ['cross_signing_not_ready', 'current_device_unverified'],
        crossSigningReady: false,
      },
      { beginVerification },
    );

    renderWorkspace(gateway);
    const [verifyButton] = await screen.findAllByRole('button', { name: 'Verify' });
    if (verifyButton === undefined) throw new Error('缺少验证设备按钮');
    await user.click(verifyButton);

    const dialog = await screen.findByRole('dialog', { name: 'Verify a Matrix device' });
    await waitFor(() => {
      expect(within(dialog).getByText('🐶')).toBeVisible();
    });
    expect(within(dialog).getByText('🚀')).toBeVisible();
    expect(beginVerification).toHaveBeenCalledWith({ targetDeviceId: 'ALICE-WEB' });

    await user.click(within(dialog).getByRole('button', { name: 'They match' }));
    expect(session.confirm).toHaveBeenCalledOnce();
  });

  it('全新账户先建立交叉签名身份，不把首次设备引向无解的 SAS 请求', async () => {
    const user = userEvent.setup();
    const establishIdentity = vi.fn(() => Promise.resolve(ok(undefined)));
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
    const setupRecovery = vi.fn(() => Promise.resolve(ok({ recoveryKey })));
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
    await waitFor(() => {
      expect(screen.queryByText(recoveryKey)).not.toBeInTheDocument();
    });
  });

  it('分开展示产品设备与 Agent 实例并二次确认级联撤销', async () => {
    const user = userEvent.setup();
    const revokeProductDevice = vi.fn(() =>
      Promise.resolve(ok({ matrixCleanup: 'pending' as const, pendingAgentInstanceCount: 1 })),
    );
    const accessManagement = accessManagementGateway({
      listAgentInstances: () => Promise.resolve(ok([agentInstance()])),
      listProductDevices: () => Promise.resolve(ok([productDevice()])),
      revokeProductDevice,
    });

    renderWorkspace(securityGateway(readySnapshot()), accessManagement);

    const productDevices = await panelForHeading('Product devices');
    const agentInstances = await panelForHeading('Agent instances');
    expect(within(productDevices).getByText('Studio workstation')).toBeVisible();
    expect(within(agentInstances).getByText('Build agent')).toBeVisible();
    await user.click(within(productDevices).getByRole('button', { name: 'Revoke device' }));
    await waitFor(() => {
      expect(screen.getByText('Revoke this product device?')).toBeVisible();
    });
    await user.click(screen.getByRole('button', { name: 'Confirm revocation' }));

    await waitFor(() => {
      expect(revokeProductDevice).toHaveBeenCalledWith(productDevice().deviceId);
    });
    expect(
      await screen.findByText(/Local access is revoked\. Matrix device cleanup is pending/u),
    ).toBeVisible();
  });
});

async function panelForHeading(name: string): Promise<HTMLElement> {
  const heading = await screen.findByRole('heading', { name });
  const panel = heading.closest('article');
  if (!(panel instanceof HTMLElement)) {
    throw new Error(`访问管理面板缺少 article 语义容器：${name}`);
  }
  return panel;
}

function renderWorkspace(
  gateway: MatrixSecurityGateway,
  accessManagement: AccessManagementGateway = accessManagementGateway(),
) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <SecurityWorkspace
          accessManagement={accessManagement}
          gateway={gateway}
          onBack={() => undefined}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function accessManagementGateway(
  overrides: Partial<AccessManagementGateway> = {},
): AccessManagementGateway {
  const base: AccessManagementGateway = {
    listAgentInstances: () => Promise.resolve(ok([])),
    listProductDevices: () => Promise.resolve(ok([])),
    revokeAgentInstance: () =>
      Promise.resolve(err({ code: 'access.not_configured', retryable: false })),
    revokeProductDevice: () =>
      Promise.resolve(err({ code: 'access.not_configured', retryable: false })),
  };
  return { ...base, ...overrides };
}

function productDevice() {
  return {
    createdAtUnixMs: 1_700_000_000_000,
    deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
    label: 'Studio workstation',
    lastSeenAtUnixMs: 1_700_000_010_000,
    matrixDeviceId: 'WEB_DEVICE',
    platform: 'windows' as const,
    revokedAtUnixMs: null,
    trustState: 'verified' as const,
  };
}

function agentInstance() {
  const device = productDevice();
  return {
    adapterType: 'codex',
    agentAvatarContentId: null,
    agentDisplayName: 'Build agent',
    agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
    agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
    capabilityVersion: '1.0',
    createdAtUnixMs: 1_700_000_000_000,
    device: {
      deviceId: device.deviceId,
      label: device.label,
      platform: device.platform,
      trustState: device.trustState,
    },
    lastSeenAtUnixMs: 1_700_000_010_000,
    matrixDeviceId: 'AR_INSTANCE',
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status: 'online' as const,
  };
}

function securityGateway(
  snapshot: MatrixSecuritySnapshot,
  overrides: Partial<MatrixSecurityGateway> = {},
): MatrixSecurityGateway {
  const base: MatrixSecurityGateway = {
    acceptIncomingVerification: () =>
      Promise.resolve(err({ code: 'security.verification_unavailable', retryable: false })),
    beginVerification: () =>
      Promise.resolve(err({ code: 'security.verification_unavailable', retryable: false })),
    declineIncomingVerification: () =>
      Promise.resolve(err({ code: 'security.verification_unavailable', retryable: false })),
    establishIdentity: () =>
      Promise.resolve(err({ code: 'security.identity_bootstrap_failed', retryable: true })),
    getIncomingVerification: () => null,
    inspect: () => Promise.resolve(ok(snapshot)),
    recover: () => Promise.resolve(err({ code: 'security.recovery_failed', retryable: true })),
    setupRecovery: () =>
      Promise.resolve(err({ code: 'security.recovery_setup_failed', retryable: true })),
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
    devices: readySnapshot().devices.map((device) => ({ ...device, trust: 'unverified' as const })),
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
  const confirm = vi.fn(() => Promise.resolve(ok(undefined)));
  const value: MatrixVerificationSession = {
    activate: vi.fn(),
    cancel: () => Promise.resolve(ok(undefined)),
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
