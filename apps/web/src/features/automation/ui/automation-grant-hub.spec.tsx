// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  AutomationGrant,
  AutomationGrantGateway,
  CreateAutomationGrantInput,
} from '@/features/automation/domain/automation-grant';
import { AutomationGrantHub } from '@/features/automation/ui/automation-grant-hub';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const INSTANCE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const ROOM_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const GRANT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('AutomationGrantHub', () => {
  it('默认关闭，明确确认影响后才提交精确实例授权', async () => {
    const user = userEvent.setup();
    const gateway = automationGateway([]);
    renderHub(gateway.value, true);

    await user.click(screen.getByRole('button', { name: 'Automation' }));
    expect(await screen.findByRole('heading', { name: 'New bounded grant' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create grant' })).toBeDisabled();

    await user.click(
      screen.getByRole('checkbox', {
        name: 'I understand this Agent can send without per-message approval.',
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Create grant' }));

    await waitFor(() => expect(gateway.create).toHaveBeenCalledOnce());
    const input = gateway.create.mock.calls[0]?.[1];
    expect(input).toMatchObject({
      agentId: AGENT_ID,
      agentInstanceId: INSTANCE_ID,
      audience: 'known_room_members',
      impactAcknowledged: true,
      maxMessagesPerMinute: 6,
      maxTotalMessages: 100,
      messageKinds: ['room_message'],
      requiresRiskScan: true,
      roomCatalogId: ROOM_ID,
    });
    expect(await screen.findByText('Local Agent')).toBeInTheDocument();
  });

  it('近期认证缺失时不展示可提交写入并提供重新验证入口', async () => {
    const user = userEvent.setup();
    const reauthenticate = vi.fn();
    renderHub(automationGateway([]).value, false, reauthenticate);

    await user.click(screen.getByRole('button', { name: 'Automation' }));
    const action = await screen.findByRole('button', { name: 'Verify identity again' });
    expect(screen.queryByRole('button', { name: 'Create grant' })).not.toBeInTheDocument();

    await user.click(action);
    expect(reauthenticate).toHaveBeenCalledOnce();
  });

  it('控制平面返回不确定状态时显示失败且不伪造授权', async () => {
    const gateway = automationGateway([]);
    gateway.value.list = vi.fn(async () =>
      err({ code: 'automation.unreachable', retryable: true }),
    );
    const user = userEvent.setup();
    renderHub(gateway.value, true);

    await user.click(screen.getByRole('button', { name: 'Automation' }));

    expect(await screen.findByText('Automation grants could not be read.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Create grant' })).not.toBeInTheDocument();
  });
});

function renderHub(
  automation: AutomationGrantGateway,
  recentlyAuthenticated: boolean,
  onReauthenticate: () => void = vi.fn(),
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <AutomationGrantHub
          accessManagement={accessManagement}
          automation={automation}
          catalogId={ROOM_ID}
          onReauthenticate={onReauthenticate}
          recentlyAuthenticated={recentlyAuthenticated}
          roomName="Builders"
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function automationGateway(initial: readonly AutomationGrant[]) {
  let grants = [...initial];
  const create = vi.fn(async (grantId: string, input: CreateAutomationGrantInput) => {
    const created: AutomationGrant = {
      agentId: input.agentId,
      agentInstanceId: input.agentInstanceId ?? null,
      audience: input.audience,
      expiresAtUnixMs: Date.now() + input.lifetimeSeconds * 1_000,
      grantId: grantId === '' ? GRANT_ID : grantId,
      maxMessagesPerMinute: input.maxMessagesPerMinute,
      maxTotalMessages: input.maxTotalMessages ?? null,
      messageKinds: input.messageKinds,
      messagesInCurrentMinute: 0,
      requiresRiskScan: input.requiresRiskScan,
      revokedAtUnixMs: null,
      roomCatalogId: input.roomCatalogId,
      startsAtUnixMs: Date.now(),
      status: 'active',
      totalMessages: 0,
    };
    grants = [created, ...grants];
    return ok(created);
  });
  const value: AutomationGrantGateway = {
    create,
    list: vi.fn(async () => ok(grants)),
    revoke: vi.fn(async () => err({ code: 'automation.test_not_available', retryable: false })),
  };
  return { create, value };
}

const accessManagement: AccessManagementGateway = {
  listAgentInstances: async () =>
    ok([
      {
        adapterType: 'codex',
        agentAvatarContentId: null,
        agentDisplayName: 'Local Agent',
        agentId: AGENT_ID,
        agentInstanceId: INSTANCE_ID,
        capabilityVersion: '1',
        createdAtUnixMs: 1_700_000_000_000,
        device: {
          deviceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43',
          label: 'Workstation',
          platform: 'windows',
          trustState: 'verified',
        },
        lastSeenAtUnixMs: 1_700_000_100_000,
        matrixDeviceId: 'MATRIX-DEVICE',
        matrixDeviceRevokedAtUnixMs: null,
        revokedAtUnixMs: null,
        status: 'online',
      },
    ]),
  listProductDevices: async () => ok([]),
  revokeAgentInstance: async () => err({ code: 'access.test_not_available', retryable: false }),
  revokeProductDevice: async () => err({ code: 'access.test_not_available', retryable: false }),
};
