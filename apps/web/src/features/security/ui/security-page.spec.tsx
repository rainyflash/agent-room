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
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
  // jsdom 没有 scrollIntoView；“输入恢复密钥”会滚到恢复密钥一节。
  Element.prototype.scrollIntoView = vi.fn();
});

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(cleanup);

describe('SecurityWorkspace', () => {
  it('这台设备已由你签名时只说一句；设备列出名字、是不是这台、签没签名，ID 和指纹在详情里', async () => {
    renderWorkspace(securityGateway(readySnapshot()));

    await visible(screen.findByText('This device is signed by you'));
    expect(screen.getByText('Alice browser')).toBeVisible();
    expect(screen.getByText('This device')).toBeVisible();
    expect(screen.getByText('Signed by you')).toBeVisible();
    // 排查用的信息默认收起；不再显示加密引擎版本。
    expect(screen.getByText('ED25519 CURRENT FINGERPRINT')).not.toBeVisible();
    expect(screen.getByText('@alice:agent-room.test')).not.toBeVisible();
    expect(screen.queryByText(/Crypto engine/u)).not.toBeInTheDocument();
  });

  it('这台设备在服务器上由你签过名、本机没有签名私钥时，也算签好了，不再提示要签名', async () => {
    renderWorkspace(
      securityGateway({
        ...readySnapshot(),
        crossSigningReady: false,
        devices: readySnapshot().devices.map((device) => ({
          ...device,
          trust: 'signed' as const,
        })),
      }),
    );

    await visible(screen.findByText('This device is signed by you'));
    expect(
      screen.queryByRole('region', { name: 'This device needs to be signed' }),
    ).not.toBeInTheDocument();
  });

  it('这台设备要签名时给两条路：用另一台设备核对，走官方 SAS 会话', async () => {
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
        devices: [...blockedSnapshot().devices, laptop()],
      },
      { beginVerification },
    );

    renderWorkspace(gateway);
    await visible(screen.findByText('This device needs to be signed'));
    expect(
      screen.getByText('Either way, the result is the same: this device is signed by you.'),
    ).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Start verifying' }));

    const dialog = await screen.findByRole('dialog', { name: 'Verify a device' });
    await waitFor(() => {
      expect(within(dialog).getByText('🐶')).toBeVisible();
    });
    expect(within(dialog).getByText('🚀')).toBeVisible();
    expect(beginVerification).toHaveBeenCalledWith({ targetDeviceId: 'ALICE-WEB' });

    await user.click(within(dialog).getByRole('button', { name: 'They match' }));
    expect(session.confirm).toHaveBeenCalledOnce();
  });

  it('另一条路：点“输入恢复密钥”直接打开恢复密钥的输入框；只有一台设备时不能用别的设备核对', async () => {
    const user = userEvent.setup();
    renderWorkspace(securityGateway(blockedSnapshot()));

    expect(await screen.findByRole('button', { name: 'Start verifying' })).toBeDisabled();
    await visible(screen.findByText('You’re not signed in on any other device right now.'));
    const signing = screen.getByRole('region', { name: 'This device needs to be signed' });
    await user.click(within(signing).getByRole('button', { name: 'Enter recovery key' }));
    await visible(screen.findByLabelText('Passphrase or recovery key'));
  });

  it('别的设备没签名时可以在列表里核对它', async () => {
    const user = userEvent.setup();
    const session = verificationSession({ stage: 'waiting' });
    const beginVerification = vi.fn(() => Promise.resolve(ok(session.value)));
    const gateway = securityGateway(
      {
        ...readySnapshot(),
        devices: [...readySnapshot().devices, { ...laptop(), trust: 'unverified' as const }],
      },
      { beginVerification },
    );

    renderWorkspace(gateway);
    await visible(screen.findByText('Not signed'));
    await user.click(screen.getByRole('button', { name: 'Verify' }));
    await waitFor(() => {
      expect(beginVerification).toHaveBeenCalledWith({ targetDeviceId: 'ALICE-LAPTOP' });
    });
  });

  it('全新账户先建立加密身份，不把首次设备引向无解的核对', async () => {
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
    await user.click(await screen.findByRole('button', { name: 'Set up now' }));

    expect(establishIdentity).toHaveBeenCalledOnce();
    expect(screen.queryByRole('button', { name: 'Verify' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Start verifying' })).not.toBeInTheDocument();
  });

  it('恢复密钥只在当前界面显示一次且不会写入浏览器存储', async () => {
    const user = userEvent.setup();
    const recoveryKey = 'EsTc r7Cy 4abc one-time recovery key';
    const setupRecovery = vi.fn(() => Promise.resolve(ok({ recoveryKey })));
    const gateway = securityGateway(missingRecoverySnapshot(), { setupRecovery });

    const { queryClient } = renderWorkspace(gateway);
    // 没设置恢复密钥只是一条建议。
    await visible(screen.findByText('Not set up yet'));
    await user.click(screen.getByRole('button', { name: 'Set up recovery key' }));
    await user.type(screen.getByLabelText('Recovery passphrase'), 'correct horse battery staple');
    await user.type(screen.getByLabelText('Confirm passphrase'), 'correct horse battery staple');
    await user.click(screen.getByRole('button', { name: 'Create recovery' }));

    await visible(screen.findByText(recoveryKey));
    expect(setupRecovery).toHaveBeenCalledWith({ passphrase: 'correct horse battery staple' });
    expect(storageValues(window.localStorage)).not.toContain(recoveryKey);
    expect(
      JSON.stringify(
        queryClient
          .getMutationCache()
          .getAll()
          .map((mutation) => mutation.state),
      ),
    ).not.toContain(recoveryKey);

    await user.click(screen.getByRole('button', { name: 'I saved the recovery key' }));
    await waitFor(() => {
      expect(screen.queryByText(recoveryKey)).not.toBeInTheDocument();
    });
  });

  it('已登录的电脑和 Agent 分开列，撤销电脑要确认并说清会断开上面的 Agent', async () => {
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

    const computers = await panelForHeading('Computers and browsers');
    const agents = await panelForHeading('Agents');
    expect(within(computers).getByText('Studio workstation')).toBeVisible();
    expect(within(agents).getByText('Build agent')).toBeVisible();
    // 内部 ID 不再列在名字下面。
    expect(within(computers).queryByText(productDevice().deviceId)).not.toBeInTheDocument();
    await user.click(within(computers).getByRole('button', { name: 'Revoke' }));
    await waitFor(() => {
      expect(screen.getByText('Sign out this computer?')).toBeVisible();
    });
    await user.click(screen.getByRole('button', { name: 'Yes, revoke' }));

    await waitFor(() => {
      expect(revokeProductDevice).toHaveBeenCalledWith(productDevice().deviceId);
    });
    await visible(screen.findByText(/Signed out\. The server is still finishing up/u));
  });
});

function laptop() {
  return {
    current: false,
    deviceId: 'ALICE-LAPTOP',
    displayName: 'Alice laptop',
    fingerprint: 'ED25519 LAPTOP FINGERPRINT',
    trust: 'signed' as const,
    userId: '@alice:agent-room.test',
  };
}

/** 内容淡入（从透明开始），jsdom 里要等它显示出来。 */
async function visible(found: Promise<HTMLElement>): Promise<HTMLElement> {
  const element = await found;
  await waitFor(() => {
    expect(element).toBeVisible();
  });
  return element;
}

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
  const view = render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <QueryClientProvider client={queryClient}>
          <SecurityWorkspace accessManagement={accessManagement} gateway={gateway} />
        </QueryClientProvider>
      </RouterTestProvider>
    </I18nextProvider>,
  );
  return { ...view, queryClient };
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
