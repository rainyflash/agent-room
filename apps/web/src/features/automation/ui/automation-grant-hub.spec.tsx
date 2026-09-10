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
import { readAutomationGrantDraft } from '@/features/automation/adapters/automation-grant-draft';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const INSTANCE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const ROOM_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const GRANT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  window.sessionStorage.clear();
});

describe('AutomationGrantHub', () => {
  it('验证返回后恢复同一账户的精确授权草稿，重新确认后才创建并清理草稿', async () => {
    const user = userEvent.setup();
    const gateway = automationGateway([]);
    const reauthenticate = vi.fn();
    renderHub(gateway.value, false, reauthenticate);
    await user.click(screen.getByRole('button', { name: 'Automation' }));
    await screen.findByRole('heading', { name: 'New bounded grant' });
    await user.click(screen.getByRole('checkbox', { name: 'New room messages' }));
    await user.click(screen.getByRole('checkbox', { name: 'Replies' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'Audience' }), 'any_room_member');
    await user.clear(screen.getByRole('spinbutton', { name: 'Messages per minute' }));
    await user.type(screen.getByRole('spinbutton', { name: 'Messages per minute' }), '3');
    await user.clear(screen.getByRole('spinbutton', { name: /Maximum total messages/u }));
    await user.type(screen.getByRole('spinbutton', { name: /Maximum total messages/u }), '20');
    await user.selectOptions(screen.getByRole('combobox', { name: /Lifetime/u }), '3600');
    await user.click(screen.getByRole('button', { name: 'Verify identity again' }));
    expect(reauthenticate).toHaveBeenCalledOnce();
    expect(readAutomationGrantDraft(GRANT_ID)).toEqual(ok(null));
    cleanup();
    renderHub(gateway.value, true);
    await screen.findByRole('heading', { name: 'New bounded grant' });
    expect(screen.getByRole('checkbox', { name: 'Replies' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'New room messages' })).not.toBeChecked();
    expect(screen.getByRole('combobox', { name: 'Audience' })).toHaveValue('any_room_member');
    expect(screen.getByRole('spinbutton', { name: 'Messages per minute' })).toHaveValue(3);
    expect(screen.getByRole('spinbutton', { name: /Maximum total messages/u })).toHaveValue(20);
    expect(screen.getByRole('combobox', { name: /Lifetime/u })).toHaveValue('3600');
    expect(screen.getByRole('button', { name: 'Create grant' })).toBeDisabled();
    expect(gateway.create).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole('checkbox', {
        name: 'I understand this Agent can send without per-message approval.',
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Create grant' }));
    await waitFor(() => {
      expect(gateway.create).toHaveBeenCalledOnce();
    });
    expect(gateway.create.mock.calls[0]?.[1]).toMatchObject({
      agentInstanceId: INSTANCE_ID,
      messageKinds: ['reply'],
      maxMessagesPerMinute: 3,
      maxTotalMessages: 20,
      lifetimeSeconds: 3600,
    });
    expect(readAutomationGrantDraft(AGENT_ID)).toEqual(ok(null));
  });
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

    await waitFor(() => {
      expect(gateway.create).toHaveBeenCalledOnce();
    });
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

  it('公开发言必须保持风险扫描并解释受众范围', async () => {
    const user = userEvent.setup();
    renderHub(automationGateway([]).value, true);
    await user.click(screen.getByRole('button', { name: 'Automation' }));
    const riskScan = await screen.findByRole('checkbox', {
      name: 'Require a passing risk scan before every autonomous send',
    });
    await user.click(riskScan);
    expect(riskScan).not.toBeChecked();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Audience' }), 'any_room_member');
    expect(riskScan).toBeChecked();
    expect(riskScan).toBeDisabled();
    expect(screen.getByText(/Public lobby replies are visible to everyone/u)).toBeInTheDocument();
  });

  it('控制平面返回不确定状态时显示失败且不伪造授权', async () => {
    const gateway = automationGateway([]);
    gateway.value.list = vi.fn(() =>
      Promise.resolve(err({ code: 'automation.unreachable', retryable: true })),
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
          principalId={AGENT_ID}
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
  const create = vi.fn((grantId: string, input: CreateAutomationGrantInput) => {
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
    return Promise.resolve(ok(created));
  });
  const value: AutomationGrantGateway = {
    create,
    list: vi.fn(() => Promise.resolve(ok(grants))),
    revoke: vi.fn(() =>
      Promise.resolve(err({ code: 'automation.test_not_available', retryable: false })),
    ),
  };
  return { create, value };
}

const accessManagement: AccessManagementGateway = {
  listAgentInstances: () =>
    Promise.resolve(
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
    ),
  listProductDevices: () => Promise.resolve(ok([])),
  revokeAgentInstance: () =>
    Promise.resolve(err({ code: 'access.test_not_available', retryable: false })),
  revokeProductDevice: () =>
    Promise.resolve(err({ code: 'access.test_not_available', retryable: false })),
};
