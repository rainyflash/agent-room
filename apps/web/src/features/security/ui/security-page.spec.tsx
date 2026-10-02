// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type {
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import { SecurityWorkspace } from '@/features/security/ui/security-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import {
  DeviceSigningStatus,
  type DeviceSigningState,
} from '@/shared/matrix/device-signing-status';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('SecurityWorkspace', () => {
  it('这台设备签好了只说一句已就绪；设备列出名字、是不是这台、签没签名，ID 和指纹在详情里', async () => {
    renderWorkspace(securityGateway(snapshot('verified')), signing({ kind: 'ready' }));

    await visible(screen.findByText('This device is ready'));
    // 设备列表淡入，等它显示出来。
    await visible(screen.findByText('Alice browser'));
    expect(screen.getByText('This device')).toBeVisible();
    expect(screen.getByText('Signed by you')).toBeVisible();
    // 排查用的信息默认收起。
    expect(screen.getByText('ED25519 CURRENT FINGERPRINT')).not.toBeVisible();
    expect(screen.getByText('@alice:agent-room.test')).not.toBeVisible();
    // 不再有恢复密钥、核对。
    expect(screen.queryByText(/recovery key/iu)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /verify/iu })).not.toBeInTheDocument();
  });

  it('服务器上由你签过名就算就绪，不管这台设备的自动签名记的是什么', async () => {
    renderWorkspace(securityGateway(snapshot('signed')), signing({ kind: 'idle' }));

    await visible(screen.findByText('This device is ready'));
  });

  it('自动签名进行中只说正在准备，不让人做任何选择', async () => {
    renderWorkspace(securityGateway(snapshot('unverified')), signing({ kind: 'working' }));

    await visible(screen.findByText('Getting this device ready'));
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
  });

  it('自动签名出错时给一个重试，签好后变成已就绪', async () => {
    const user = userEvent.setup();
    const status = signing({ kind: 'failed' });
    const retry = vi.fn(() => {
      status.set({ kind: 'working' });
    });
    status.setRetry(retry);
    renderWorkspace(securityGateway(snapshot('unverified')), status);

    await visible(screen.findByText('This device isn’t ready yet'));
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(retry).toHaveBeenCalledOnce();
    await visible(screen.findByText('Getting this device ready'));

    act(() => {
      status.set({ kind: 'ready' });
    });
    await visible(screen.findByText('This device is ready'));
  });

  it('别的设备没签名时只列状态，不给核对按钮', async () => {
    renderWorkspace(
      securityGateway({
        ...snapshot('verified'),
        devices: [...snapshot('verified').devices, { ...laptop(), trust: 'unverified' as const }],
      }),
      signing({ kind: 'ready' }),
    );

    const laptopRow = (await screen.findByText('Alice laptop')).closest('li');
    expect(laptopRow).toBeInstanceOf(HTMLElement);
    await visible(within(laptopRow as HTMLElement).findByText('Not signed'));
    expect(
      within(laptopRow as HTMLElement).queryByRole('button', { name: /verify/iu }),
    ).not.toBeInTheDocument();
  });

  it('查不到设备时照样说这台设备的状态，并给刷新', async () => {
    renderWorkspace(
      securityGateway(snapshot('verified'), {
        inspect: () =>
          Promise.resolve(err({ code: 'security.inspection_failed', retryable: true })),
      }),
      signing({ kind: 'ready' }),
    );

    await visible(screen.findByText('Couldn’t check your devices'));
    expect(screen.getByText('This device is ready')).toBeVisible();
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

    renderWorkspace(
      securityGateway(snapshot('verified')),
      signing({ kind: 'ready' }),
      accessManagement,
    );

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
  deviceSigning: DeviceSigningStatus = signing({ kind: 'ready' }),
  accessManagement: AccessManagementGateway = accessManagementGateway(),
) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <QueryClientProvider client={queryClient}>
          <SecurityWorkspace
            accessManagement={accessManagement}
            deviceSigning={deviceSigning}
            gateway={gateway}
          />
        </QueryClientProvider>
      </RouterTestProvider>
    </I18nextProvider>,
  );
}

function signing(state: DeviceSigningState): DeviceSigningStatus {
  const status = new DeviceSigningStatus();
  status.set(state);
  return status;
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
  value: MatrixSecuritySnapshot,
  overrides: Partial<MatrixSecurityGateway> = {},
): MatrixSecurityGateway {
  return {
    inspect: () => Promise.resolve(ok(value)),
    subscribe: () => () => undefined,
    ...overrides,
  };
}

function snapshot(trust: 'signed' | 'unverified' | 'verified'): MatrixSecuritySnapshot {
  return Object.freeze({
    currentDeviceId: 'ALICE-WEB',
    devices: [
      {
        current: true,
        deviceId: 'ALICE-WEB',
        displayName: 'Alice browser',
        fingerprint: 'ED25519 CURRENT FINGERPRINT',
        trust,
        userId: '@alice:agent-room.test',
      },
    ],
    userId: '@alice:agent-room.test',
  });
}
