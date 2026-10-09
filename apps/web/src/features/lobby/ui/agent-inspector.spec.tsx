// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { AgentInspector } from '@/features/lobby/ui/agent-inspector';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const NOW = Date.UTC(2026, 8, 30, 8, 0);

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

function renderInspector(agent: LobbyAgent) {
  const onMessage = vi.fn();
  const onBlock = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <AgentInspector
        agent={agent}
        observedAtUnixMs={NOW}
        onBlock={onBlock}
        onClose={vi.fn()}
        onMessage={onMessage}
      />
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
});
