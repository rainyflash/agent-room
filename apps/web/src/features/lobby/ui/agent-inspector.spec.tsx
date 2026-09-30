// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { AgentInspector } from '@/features/lobby/ui/agent-inspector';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const NOW = Date.UTC(2026, 8, 30, 8, 0);

const base: LobbyAgent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
  displayName: 'Scout',
  instanceIds: ['a', 'b'],
  lastActiveAtUnixMs: NOW - 60_000,
  lifecycle: {
    archiveReason: null,
    archived: false,
    connection: 'online',
    offlineSinceUnixMs: null,
    reception: 'waiting',
  },
  matrixUserId: '@scout:agent-room.test',
  status: 'working',
  statusExpiresAtUnixMs: NOW + 60_000,
  trust: 'unknown',
  visibility: 'detailed',
};

const online: LobbyAgent = { ...base, summary: 'Reviewing the release checklist' };

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
  it('先是私聊和屏蔽，再说它会不会回复、在做什么', () => {
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
    const doing = within(panel).getByRole('region', { name: 'What it is doing' });
    expect(doing).toHaveTextContent('Working');
    expect(doing).toHaveTextContent('Reviewing the release checklist');
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

  it('离线又没说过在做什么时，不重复“离线”，只说离线多久', () => {
    renderInspector(offline);

    expect(screen.getByRole('button', { name: 'Leave a message' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'What it is doing' })).not.toBeInTheDocument();
    expect(screen.getByText('Time offline').nextElementSibling).toHaveTextContent('1–24 hours');
    expect(screen.getByRole('region', { name: 'Will it reply' })).toHaveTextContent(
      'It is offline.',
    );
  });
});
