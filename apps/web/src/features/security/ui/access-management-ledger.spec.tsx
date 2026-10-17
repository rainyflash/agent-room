// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  AccessManagementGateway,
  AgentInstance,
  ProductDevice,
} from '@/features/security/domain/access-management';
import { AccessManagementLedger } from '@/features/security/ui/access-management-ledger';
import type { Reauthentication } from '@/features/security/ui/bulk-device-revocation-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const DAY = 24 * 60 * 60 * 1_000;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('AccessManagementLedger', () => {
  it('最近 7 天用过的摊开；更久没用的、撤销了的收起来写着有几个，30 天没用的写已过期', async () => {
    show(
      gateway({
        devices: [
          device(1, 'Windows desktop', 1),
          device(2, 'Alpha 70 acceptance', 10),
          device(3, 'Alpha 60 acceptance', 40),
          { ...device(4, 'Old laptop', 50), trustState: 'revoked', revokedAtUnixMs: Date.now() },
        ],
      }),
    );

    const computers = await panel('Computers and browsers');
    expect((await within(computers).findByText('Windows desktop')).closest('details')).toBeNull();
    const idle = within(computers).getByText('Not used for 7 days or more · 2').closest('details');
    expect(idle).not.toHaveAttribute('open');
    expect(within(computers).getByText('Alpha 70 acceptance').closest('details')).toBe(idle);
    const expired = within(computers).getByText('Alpha 60 acceptance').closest('li');
    expect(expired).toHaveTextContent('Expired');
    const revoked = within(computers).getByText('Revoked · 1').closest('details');
    expect(within(computers).getByText('Old laptop').closest('details')).toBe(revoked);
    // 标题旁的数只算还登录着的：撤销了的、过期了的不算。
    expect(within(computers).getByText('02')).toBeInTheDocument();
  });

  it('一键撤销列出更久没用的每一台、默认都勾上，取消勾选的留着，撤完关上', async () => {
    const user = userEvent.setup();
    const revokeProductDevice = vi.fn<AccessManagementGateway['revokeProductDevice']>(() =>
      Promise.resolve(ok({ matrixCleanup: 'complete' as const, pendingAgentInstanceCount: 0 })),
    );
    show(
      gateway({
        devices: [
          device(1, 'Windows desktop', 1),
          device(2, 'Alpha 70 acceptance', 10),
          device(3, 'Alpha 74 acceptance', 12),
          device(4, 'Alpha 60 acceptance', 40),
        ],
        revokeProductDevice,
      }),
    );

    await user.click(await screen.findByRole('button', { name: 'Revoke all' }));
    const dialog = await screen.findByRole('dialog', {
      name: 'Revoke computers not used for 7 days or more',
    });
    const boxes = within(dialog).getAllByRole('checkbox');
    expect(boxes).toHaveLength(3);
    for (const box of boxes) expect(box).toBeChecked();
    expect(within(dialog).queryByText('Windows desktop')).not.toBeInTheDocument();

    await user.click(within(dialog).getByRole('checkbox', { name: /Alpha 74 acceptance/u }));
    await user.click(within(dialog).getByRole('button', { name: 'Revoke 2 computers' }));

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    expect(revokeProductDevice.mock.calls.map(([id]) => id)).toEqual([
      device(2, '', 0).deviceId,
      device(4, '', 0).deviceId,
    ]);
  });

  it('没有最近登录先请人重新登录，不去撤销', async () => {
    const user = userEvent.setup();
    const revokeProductDevice = vi.fn<AccessManagementGateway['revokeProductDevice']>();
    const signInAgain = vi.fn();
    show(gateway({ devices: [device(2, 'Alpha 70 acceptance', 10)], revokeProductDevice }), {
      recentlyAuthenticated: false,
      signInAgain,
    });

    await user.click(await screen.findByRole('button', { name: 'Revoke all' }));
    const dialog = await screen.findByRole('dialog');
    expect(
      within(dialog).getByText('Revoking computers needs a fresh sign-in first.'),
    ).toBeVisible();
    expect(within(dialog).queryByRole('button', { name: /^Revoke \d/u })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Sign in again' }));
    expect(signInAgain).toHaveBeenCalledOnce();
    expect(revokeProductDevice).not.toHaveBeenCalled();
  });

  it('撤到一半服务器说要重新登录：停下，改成请人重新登录', async () => {
    const user = userEvent.setup();
    const revokeProductDevice = vi.fn<AccessManagementGateway['revokeProductDevice']>(() =>
      Promise.resolve(err({ code: 'authentication.reauthentication_required', retryable: false })),
    );
    show(
      gateway({
        devices: [device(2, 'Alpha 70 acceptance', 10), device(3, 'Alpha 60 acceptance', 40)],
        revokeProductDevice,
      }),
      { recentlyAuthenticated: true, signInAgain: vi.fn() },
    );

    await user.click(await screen.findByRole('button', { name: 'Revoke all' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Revoke 2 computers' }));

    expect(
      await within(dialog).findByRole('button', { name: 'Sign in again' }),
    ).toBeInTheDocument();
    expect(revokeProductDevice).toHaveBeenCalledOnce();
  });

  it('没撤成的留在对话框里、还勾着，说清有几台', async () => {
    const user = userEvent.setup();
    const revokeProductDevice = vi.fn<AccessManagementGateway['revokeProductDevice']>(() =>
      Promise.resolve(err({ code: 'network.unavailable', retryable: true })),
    );
    show(gateway({ devices: [device(2, 'Alpha 70 acceptance', 10)], revokeProductDevice }));

    await user.click(await screen.findByRole('button', { name: 'Revoke all' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Revoke 1 computer' }));

    expect(
      await within(dialog).findByText('1 computer could not be revoked. Try again.'),
    ).toBeVisible();
    expect(within(dialog).getByRole('checkbox')).toBeChecked();
  });

  it('Agent 连着的摊开，不在线的、已断开的收起来写着有几个', async () => {
    show(
      gateway({
        devices: [device(1, 'Windows desktop', 1)],
        instances: [
          instance(1, 'Build agent', 'online'),
          instance(2, 'Release QA', 'offline'),
          instance(3, 'Old QA', 'offline'),
          instance(4, 'Gone', 'revoked'),
        ],
      }),
    );

    const agents = await panel('Agents');
    expect((await within(agents).findByText('Build agent')).closest('details')).toBeNull();
    const offline = within(agents).getByText('Offline · 2').closest('details');
    expect(offline).not.toHaveAttribute('open');
    expect(within(agents).getByText('Release QA').closest('details')).toBe(offline);
    const disconnected = within(agents).getByText('Disconnected · 1').closest('details');
    expect(within(agents).getByText('Gone').closest('details')).toBe(disconnected);
  });
});

function show(accessManagement: AccessManagementGateway, reauthentication?: Reauthentication) {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>
        <AccessManagementLedger gateway={accessManagement} reauthentication={reauthentication} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

async function panel(name: string): Promise<HTMLElement> {
  const heading = await screen.findByRole('heading', { name });
  const article = heading.closest('article');
  if (!(article instanceof HTMLElement)) throw new Error(`没有 ${name} 这一栏`);
  return article;
}

function gateway({
  devices,
  instances = [],
  revokeProductDevice,
}: {
  readonly devices: readonly ProductDevice[];
  readonly instances?: readonly AgentInstance[];
  readonly revokeProductDevice?: AccessManagementGateway['revokeProductDevice'];
}): AccessManagementGateway {
  return {
    listAgentInstances: () => Promise.resolve(ok(instances)),
    listProductDevices: () => Promise.resolve(ok(devices)),
    revokeAgentInstance: () =>
      Promise.resolve(err({ code: 'access.not_configured', retryable: false })),
    revokeProductDevice:
      revokeProductDevice ??
      (() => Promise.resolve(err({ code: 'access.not_configured', retryable: false }))),
  };
}

/** 第 `index` 台电脑，`daysAgo` 天前用过。 */
function device(index: number, label: string, daysAgo: number): ProductDevice {
  return {
    createdAtUnixMs: Date.now() - 400 * DAY,
    deviceId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${String(index).padStart(2, '0')}`,
    label,
    lastSeenAtUnixMs: Date.now() - daysAgo * DAY,
    matrixDeviceId: null,
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  };
}

function instance(index: number, name: string, status: AgentInstance['status']): AgentInstance {
  const computer = device(1, 'Windows desktop', 1);
  return {
    adapterType: 'cli',
    agentAvatarContentId: null,
    agentDisplayName: name,
    agentId: `0198b601-77a1-7bb8-83eb-a8fe68c97f${String(index).padStart(2, '0')}`,
    agentInstanceId: `0198b601-77a1-7bb8-83eb-a8fe68c97d${String(index).padStart(2, '0')}`,
    capabilityVersion: '1.0',
    createdAtUnixMs: Date.now() - 30 * DAY,
    device: {
      deviceId: computer.deviceId,
      label: computer.label,
      platform: computer.platform,
      trustState: computer.trustState,
    },
    lastSeenAtUnixMs: Date.now() - index * 60_000,
    matrixDeviceId: `AR_INSTANCE_${String(index)}`,
    matrixDeviceRevokedAtUnixMs: status === 'revoked' ? Date.now() : null,
    revokedAtUnixMs: status === 'revoked' ? Date.now() : null,
    status,
  };
}
