// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  AgentModerationGateway,
  GrantReversal,
  RoomModerationGrant,
} from '@/features/moderation/domain/agent-moderation';
import type { ModerationAction, ModerationGateway } from '@/features/moderation/domain/moderation';
import { ModerationSettings } from '@/features/moderation/ui/moderation-settings';
import type {
  AccessManagementGateway,
  AgentInstance,
} from '@/features/security/domain/access-management';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const ROOM_ID = '01990d9e-8400-7000-8000-000000000031';
const ACTION_ID = '01990d9e-8400-7000-8000-000000000033';
const OWNER_ID = '01990d9e-8400-7000-8000-000000000034';
const ADMIN_ID = '01990d9e-8400-7000-8000-000000000035';
const AGENT_ID = '01990d9e-8400-7000-8000-000000000036';
const GRANT_ID = '01990d9e-8400-7000-8000-000000000037';
const OTHER_AGENT_ID = '01990d9e-8400-7000-8000-000000000038';
const OTHER_GRANT_ID = '01990d9e-8400-7000-8000-000000000039';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('交给 Agent 管', () => {
  it('选自己的 Agent、写规则、勾能做的交给它，同一个 Agent 在几台电脑上只列一次，撤掉的不列', async () => {
    const user = userEvent.setup();
    const agents = agentGateway({ grants: [] });
    renderSettings({ agents });

    const agent = await screen.findByLabelText('Agent');
    expect(
      within(agent)
        .getAllByRole('option')
        .map((option) => option.textContent),
    ).toEqual(['Moderator Agent']);
    await user.type(screen.getByLabelText('Rules'), '  Mute anyone posting ads.  ');
    await user.click(screen.getByRole('checkbox', { name: 'Remove from room' }));
    await user.click(screen.getByRole('button', { name: 'Hand it over' }));

    await waitFor(() => {
      expect(agents.grant).toHaveBeenCalledOnce();
    });
    expect(agents.grant).toHaveBeenCalledWith(ROOM_ID, {
      actions: ['mute', 'kick'],
      agentId: AGENT_ID,
      lifetimeSeconds: 30 * 24 * 60 * 60,
      rules: 'Mute anyone posting ads.',
    });
  });

  it('没写规则不交，说清楚要写什么', async () => {
    const user = userEvent.setup();
    const agents = agentGateway({ grants: [] });
    renderSettings({ agents });

    await user.click(await screen.findByRole('button', { name: 'Hand it over' }));

    expect(await screen.findByText(/^Write the rules/u)).toBeVisible();
    expect(agents.grant).not.toHaveBeenCalled();
  });

  it('列出在用的授权、谁给的，撤掉时说之前做的照旧生效', async () => {
    const user = userEvent.setup();
    const agents = agentGateway({
      grants: [grant({ grantedBy: OWNER_ID }), grant({ grantedBy: ADMIN_ID, id: OTHER_GRANT_ID })],
    });
    renderSettings({ agents });

    const list = await screen.findByRole('region', { name: 'Agents moderating this room' });
    expect(within(list).getByText(/given by you/u)).toBeVisible();
    expect(within(list).getByText(/given by Ada/u)).toBeVisible();
    expect(within(list).getAllByText('Mute, Ban')).toHaveLength(2);
    expect(within(list).getByText(/stays in effect until you undo it/u)).toBeVisible();

    const [revoke] = within(list).getAllByRole('button', { name: 'Revoke' });
    if (revoke === undefined) throw new Error('列表里要有撤掉的按钮。');
    await user.click(revoke);

    await waitFor(() => {
      expect(agents.revokeGrant).toHaveBeenCalledWith(ROOM_ID, GRANT_ID);
    });
  });

  it('台账里写是哪个 Agent 照规则做的、理由，一份授权下还生效着的问过一句再一次全撤', async () => {
    const user = userEvent.setup();
    const agents = agentGateway({ grants: [] });
    renderSettings({ actions: [appliedMute()], agents });

    expect(
      await screen.findByText('Moderator Agent, following the rules: Posted five ad links.'),
    ).toBeVisible();
    const inEffect = screen.getByRole('region', { name: 'Still in effect from agents' });
    expect(within(inEffect).getByText('1 action still in effect')).toBeVisible();

    await user.click(within(inEffect).getByRole('button', { name: 'Undo all' }));
    expect(
      within(inEffect).getByText('Undo the 1 action Moderator Agent took that is still in effect?'),
    ).toBeVisible();
    expect(agents.reverseGrant).not.toHaveBeenCalled();
    await user.click(within(inEffect).getByRole('button', { name: 'Undo all' }));

    await waitFor(() => {
      expect(agents.reverseGrant).toHaveBeenCalledWith(ROOM_ID, GRANT_ID);
    });
    expect(await screen.findByText('Undid 1 action.')).toBeVisible();
  });

  it('一次全撤有没撤成的说过一会儿再点，没最近登录时先请他重新登录', async () => {
    const user = userEvent.setup();
    const agents = agentGateway({
      grants: [],
      reversal: {
        failed: [{ actionId: ACTION_ID, code: 'moderation.dependency_unavailable' }],
        reversed: [],
      },
    });
    const view = renderSettings({ actions: [appliedMute()], agents });

    const inEffect = await screen.findByRole('region', { name: 'Still in effect from agents' });
    await user.click(within(inEffect).getByRole('button', { name: 'Undo all' }));
    await user.click(within(inEffect).getByRole('button', { name: 'Undo all' }));
    expect(
      await screen.findByText('Undid 0 actions; 1 could not be undone. Try again in a moment.'),
    ).toBeVisible();

    view.unmount();
    renderSettings({ actions: [appliedMute()], agents, recentlyAuthenticated: false });
    const stale = await screen.findByRole('region', { name: 'Still in effect from agents' });
    expect(within(stale).getByRole('button', { name: 'Undo all' })).toBeDisabled();
    expect(within(stale).getByRole('button', { name: 'Sign in again' })).toBeVisible();
    expect(await screen.findByRole('button', { name: 'Hand it over' })).toBeDisabled();
  });
});

function renderSettings({
  actions = [],
  agents,
  recentlyAuthenticated = true,
}: {
  readonly actions?: readonly ModerationAction[];
  readonly agents: AgentModerationGateway;
  readonly recentlyAuthenticated?: boolean;
}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <ModerationSettings
          agents={{
            accessManagement,
            gateway: agents,
            nameOf: (principalId) => (principalId === ADMIN_ID ? 'Ada' : undefined),
            principalId: OWNER_ID,
            roomName: 'Book club',
          }}
          catalogId={ROOM_ID}
          gateway={moderationGateway(actions)}
          onReauthenticate={vi.fn()}
          recentlyAuthenticated={recentlyAuthenticated}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function agentGateway({
  grants,
  reversal = { failed: [], reversed: [ACTION_ID] },
}: {
  readonly grants: readonly RoomModerationGrant[];
  readonly reversal?: GrantReversal;
}) {
  return {
    grant: vi.fn(() => Promise.resolve(ok(grant({ grantedBy: OWNER_ID })))),
    listAgentActions: vi.fn(() =>
      Promise.resolve(
        ok({
          actions: [
            {
              actionId: ACTION_ID,
              agentId: AGENT_ID,
              agentName: 'Moderator Agent',
              grantId: GRANT_ID,
              note: 'Posted five ad links.',
            },
          ],
          reversals: [],
        }),
      ),
    ),
    listGrants: vi.fn(() => Promise.resolve(ok(grants))),
    reverseGrant: vi.fn(() => Promise.resolve(ok(reversal))),
    revokeGrant: vi.fn(() => Promise.resolve(ok(undefined))),
  } satisfies AgentModerationGateway;
}

function grant(overrides: Partial<RoomModerationGrant>): RoomModerationGrant {
  return {
    actions: ['mute', 'ban'],
    agentId: AGENT_ID,
    agentName: 'Moderator Agent',
    createdAtUnixMs: 1_800_000_000_000,
    expiresAtUnixMs: 1_802_592_000_000,
    grantedBy: OWNER_ID,
    id: GRANT_ID,
    rules: 'Mute anyone posting ads.',
    ...overrides,
  };
}

function moderationGateway(actions: readonly ModerationAction[]): ModerationGateway {
  const forbidden = err({ code: 'moderation.forbidden', retryable: false });
  return {
    applyAction: () => Promise.resolve(forbidden),
    inspectCapabilities: () => Promise.resolve(ok({ canModerateRoom: true, canReadAudit: false })),
    listActions: () => Promise.resolve(ok(actions)),
    listAudit: () => Promise.resolve(forbidden),
    listCases: () => Promise.resolve(ok([])),
    listRoomCases: () => Promise.resolve(ok([])),
    report: () => Promise.resolve(forbidden),
    reverseAction: () => Promise.resolve(forbidden),
  };
}

function appliedMute(): ModerationAction {
  return {
    actionId: ACTION_ID,
    actorPrincipalId: OWNER_ID,
    caseId: null,
    expiresAtUnixMs: null,
    failureCode: null,
    kind: 'mute',
    reason: 'spam',
    reversedAtUnixMs: null,
    roomCatalogId: ROOM_ID,
    startsAtUnixMs: 1_800_000_000_100,
    status: 'applied',
    targetKind: 'principal',
    targetReference: ADMIN_ID,
  };
}

function instance(overrides: Partial<AgentInstance>): AgentInstance {
  return {
    adapterType: 'codex',
    agentAvatarContentId: null,
    agentDisplayName: 'Moderator Agent',
    agentId: AGENT_ID,
    agentInstanceId: '01990d9e-8400-7000-8000-000000000041',
    capabilityVersion: '1',
    createdAtUnixMs: 1_700_000_000_000,
    device: {
      deviceId: '01990d9e-8400-7000-8000-000000000051',
      label: 'Workstation',
      platform: 'windows',
      trustState: 'verified',
    },
    lastSeenAtUnixMs: 1_700_000_100_000,
    matrixDeviceId: 'MATRIX-DEVICE',
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status: 'online',
    ...overrides,
  };
}

const accessManagement: AccessManagementGateway = {
  listAgentInstances: () =>
    Promise.resolve(
      ok([
        instance({}),
        instance({ agentInstanceId: '01990d9e-8400-7000-8000-000000000042', status: 'offline' }),
        instance({
          agentDisplayName: 'Retired Agent',
          agentId: OTHER_AGENT_ID,
          agentInstanceId: '01990d9e-8400-7000-8000-000000000043',
          status: 'revoked',
        }),
      ]),
    ),
  listProductDevices: () => Promise.resolve(ok([])),
  revokeAgentInstance: () =>
    Promise.resolve(err({ code: 'access.test_not_available', retryable: false })),
  revokeProductDevice: () =>
    Promise.resolve(err({ code: 'access.test_not_available', retryable: false })),
};
