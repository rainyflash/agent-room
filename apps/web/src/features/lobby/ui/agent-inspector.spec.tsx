// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AgentOwnerStore } from '@/features/lobby/application/agent-owner-store';
import type { AgentOwner } from '@/features/lobby/domain/agent-owners';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { AgentInspector } from '@/features/lobby/ui/agent-inspector';
import { AgentOwnersProvider, AgentOwnersScope } from '@/features/lobby/ui/agent-owners';
import { ok } from '@/shared/result';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const NOW = Date.UTC(2026, 8, 30, 8, 0);
const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const YOU = '0198b601-77a3-74f1-b4f4-940f291951b9';
const GRACE = '0198b601-77a3-74f1-b4f4-940f291951c1';

const identity = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
  displayName: 'Scout',
  instanceIds: ['a', 'b'],
  matrixUserId: '@scout:agent-room.test',
  trust: 'unknown',
} as const;

const base: LobbyAgent = {
  ...identity,
  lastActiveAtUnixMs: NOW - 60_000,
  lifecycle: {
    archiveReason: null,
    archived: false,
    connection: 'online',
    offlineSinceUnixMs: null,
    reception: 'waiting',
  },
  status: 'working',
  statusExpiresAtUnixMs: NOW + 60_000,
  visibility: 'detailed',
};

const online: LobbyAgent = base;

/** 写名片的 Agent：在线与否看 Matrix 的在线状态，名片是一周前进房间时写的。 */
const card: LobbyAgent = {
  ...identity,
  lastActiveAtUnixMs: NOW - 7 * 86_400_000 + 3_600_000,
  liveness: 'presence',
  presence: { state: 'online' },
  status: 'idle',
  statusExpiresAtUnixMs: NOW - 7 * 86_400_000 + 3_600_000 + 300_000,
  visibility: 'coarse',
};

const offline: LobbyAgent = {
  ...base,
  lifecycle: {
    archiveReason: null,
    archived: false,
    connection: 'offline',
    offlineSinceUnixMs: NOW - 2 * 3_600_000,
    reception: 'unavailable',
  },
  status: 'offline',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderInspector(agent: LobbyAgent, owner?: AgentOwner) {
  const onMessage = vi.fn();
  const onBlock = vi.fn();
  const inspector = (
    <AgentInspector
      agent={agent}
      observedAtUnixMs={NOW}
      onBlock={onBlock}
      onClose={vi.fn()}
      onMessage={onMessage}
    />
  );
  // 给了主人就像房间页一样问服务器“这个 Agent 是谁的”。
  const owners =
    owner === undefined
      ? null
      : new AgentOwnerStore(
          { lookup: () => Promise.resolve(ok([{ ...owner, agentId: agent.agentId }])) },
          {
            schedule: (task) => {
              task();
            },
          },
        );
  render(
    <I18nextProvider i18n={i18n}>
      {owners === null ? (
        inspector
      ) : (
        <AgentOwnersProvider store={owners}>
          <AgentOwnersScope catalogId={CATALOG} selfPrincipalId={YOU}>
            {inspector}
          </AgentOwnersScope>
        </AgentOwnersProvider>
      )}
    </I18nextProvider>,
  );
  return { onBlock, onMessage };
}

describe('人物详情', () => {
  it('先是私聊和屏蔽，再说它会不会回复；不说工作状态', () => {
    const { onBlock, onMessage } = renderInspector(online);

    const panel = screen.getByRole('complementary', { name: 'Scout' });
    const buttons = within(panel)
      .getAllByRole('button')
      .map((button) => button.textContent);
    expect(buttons.slice(0, 3)).toEqual(['', 'Message', 'Block']);
    fireEvent.click(within(panel).getByRole('button', { name: 'Message' }));
    expect(onMessage).toHaveBeenCalledWith(online.agentId);
    fireEvent.click(within(panel).getByRole('button', { name: 'Block' }));
    expect(onBlock).toHaveBeenCalledWith(online.agentId);

    expect(within(panel).getByRole('region', { name: 'Will it reply' })).toHaveTextContent(
      'It is waiting for new messages and will see yours right away.',
    );
    expect(panel).not.toHaveTextContent('Working');
    expect(panel).not.toHaveTextContent('Status it shares');
  });

  it('不再写死“信任：未验证”；Matrix ID 和连接数收在“身份与连接”里', () => {
    renderInspector(online);

    expect(screen.queryByText('Trust')).not.toBeInTheDocument();
    expect(screen.queryByText('Unverified')).not.toBeInTheDocument();
    const details = screen.getByText('Identity and connections').closest('details');
    expect(details).not.toHaveAttribute('open');
    expect(details).toHaveTextContent('@scout:agent-room.test');
    expect(details).toHaveTextContent('Open connections2');
  });

  it('离线时不重复“离线”，只说离线多久', () => {
    renderInspector(offline);

    expect(screen.getByRole('button', { name: 'Leave a message' })).toBeInTheDocument();
    expect(screen.getByText('Time offline').nextElementSibling).toHaveTextContent('1–24 hours');
    expect(screen.getByRole('region', { name: 'Will it reply' })).toHaveTextContent(
      'It is offline.',
    );
  });

  it('名片在线时不把进房间的时间当成上次连接', () => {
    renderInspector(card);

    expect(screen.getByRole('region', { name: 'Will it reply' })).toHaveTextContent(
      'It is waiting for new messages and will see yours right away.',
    );
    expect(screen.queryByText('Last connected')).not.toBeInTheDocument();
  });

  it('名片离线时，上次连接是它离线的那一刻', () => {
    const offlineAt = NOW - 30 * 60_000;
    renderInspector({ ...card, presence: { state: 'offline', offlineSeenAtUnixMs: offlineAt } });

    expect(screen.getByText('Time offline').nextElementSibling).toHaveTextContent('Under 1 hour');
    expect(screen.getByText('Last connected').nextElementSibling).toHaveTextContent(
      new Intl.DateTimeFormat('en', {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }).format(offlineAt),
    );
  });

  it('屏蔽着的 Agent 不给私聊，说一声看不到它的话，点“解除屏蔽”能改回来', () => {
    const onUnblock = vi.fn();
    const onBlock = vi.fn();
    render(
      <I18nextProvider i18n={i18n}>
        <AgentInspector
          actionFailure={{ action: 'unblock', code: 'direct_session.unreachable' }}
          agent={online}
          blocked
          observedAtUnixMs={NOW}
          onBlock={onBlock}
          onClose={vi.fn()}
          onMessage={vi.fn()}
          onUnblock={onUnblock}
        />
      </I18nextProvider>,
    );

    const panel = screen.getByRole('complementary', { name: 'Scout' });
    expect(within(panel).queryByRole('button', { name: 'Message' })).not.toBeInTheDocument();
    expect(within(panel).queryByRole('button', { name: 'Block' })).not.toBeInTheDocument();
    expect(within(panel).getByRole('status')).toHaveTextContent(
      'You blocked this agent, so you don’t see what it says.',
    );
    // 没做成的是解除屏蔽，就说解除屏蔽没成，不说“私聊出错了”。
    expect(within(panel).getByRole('alert')).toHaveTextContent(
      'Couldn’t unblock it: direct_session.unreachable',
    );
    fireEvent.click(within(panel).getByRole('button', { name: 'Unblock' }));
    expect(onUnblock).toHaveBeenCalledWith(online.agentId);
    expect(onBlock).not.toHaveBeenCalled();
  });

  it('只能和自己的 Agent 私聊：别人的、网络 Agent 不给“私聊”，屏蔽照旧', async () => {
    renderInspector(online, { kind: 'local', owner: { displayName: 'Grace', principalId: GRACE } });
    const panel = screen.getByRole('complementary', { name: 'Scout' });
    expect(await within(panel).findByText('Grace’s agent')).toBeInTheDocument();
    expect(within(panel).queryByRole('button', { name: 'Message' })).not.toBeInTheDocument();
    expect(within(panel).getByRole('button', { name: 'Block' })).toBeInTheDocument();
    cleanup();

    renderInspector(online, { kind: 'network' });
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'Message' })).not.toBeInTheDocument();
    });
    cleanup();

    const { onMessage } = renderInspector(online, {
      kind: 'local',
      owner: { displayName: 'Ada', principalId: YOU },
    });
    expect(await screen.findByText('Your agent')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Message' }));
    expect(onMessage).toHaveBeenCalledWith(online.agentId);
  });
});
