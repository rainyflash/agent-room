// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, createRef } from 'react';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { LobbyAgent, LobbyAgentStatus } from '@/features/lobby/domain/lobby';
import { NetworkAgentLabelStore } from '@/features/lobby/application/network-agent-label-store';
import { ListModeRoster, type ListModeRosterHandle } from '@/features/lobby/ui/list-mode-roster';
import {
  NetworkAgentLabelsProvider,
  NetworkAgentRelayProvider,
} from '@/features/lobby/ui/network-agent-labels';
import { ok } from '@/shared/result';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { projectAgentLifecycles } from '@agent-room/protocol';
import { presenceEvidence } from '../domain/agent-attendance';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('ListModeRoster', () => {
  it('separates actual waiters, next-run readers, offline ages and recoverable archives', async () => {
    const now = 1_700_000_000_000;
    const entries = [
      { ...agent('waiting', 'idle'), lastPolledAtUnixMs: now, listeningUntilUnixMs: now + 15000 },
      { ...agent('resume', 'completed'), lastPolledAtUnixMs: now, listeningUntilUnixMs: null },
      { ...agent('hour', 'offline'), lastActiveAtUnixMs: now - 60000 },
      { ...agent('day', 'offline'), lastActiveAtUnixMs: now - 4 * 3600000 },
      { ...agent('week', 'offline'), lastActiveAtUnixMs: now - 2 * 86400000 },
      { ...agent('old', 'offline'), lastActiveAtUnixMs: now - 8 * 86400000 },
    ];
    const user = userEvent.setup();
    renderRoster({ agents: entries, onSelectAgent: vi.fn(), selectedAgentId: null });
    for (const name of [
      'Online · waiting for messages',
      'Online · reads on next run',
      'Offline · under 1 hour',
      'Offline · 1–24 hours',
      'Offline · 1–7 days',
    ]) {
      expect(screen.getByRole('heading', { name })).toBeVisible();
    }
    expect(screen.queryByRole('button', { name: /^Old/u })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Archived (1)' }));
    expect(screen.getByRole('button', { name: /^Old/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /^Waiting/u })).not.toBeInTheDocument();
  });
  it('paginates a large archive without rendering every historical member', async () => {
    const now = 1_700_000_000_000;
    const entries = Array.from({ length: 500 }, (_, index) => ({
      ...agent(`a${String(index).padStart(3, '0')}`, 'offline'),
      lastActiveAtUnixMs: now - 1000 - index,
    }));
    const lifecycle = projectAgentLifecycles(entries.map(presenceEvidence), now);
    const projected = entries.map((entry) => {
      const state = lifecycle.get(entry.agentId);
      if (!state) throw new Error('missing fixture lifecycle');
      return { ...entry, lifecycle: state };
    });
    const user = userEvent.setup();
    renderRoster({ agents: projected, onSelectAgent: vi.fn(), selectedAgentId: null });
    expect(screen.getByRole('button', { name: 'Members (100)' })).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Archived (400)' }));
    expect(document.querySelectorAll('.roster-agent')).toHaveLength(100);
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByText('2 / 4')).toBeVisible();
  });
  it('按名称搜索、按在不在等消息筛选并只选择真实 Agent', async () => {
    const now = 1_700_000_000_000;
    const user = userEvent.setup();
    const onSelectAgent = vi.fn();
    renderRoster({
      agents: [
        { ...agent('alpha', 'working'), lastPolledAtUnixMs: now, listeningUntilUnixMs: now + 9000 },
        { ...agent('beta', 'blocked'), lastPolledAtUnixMs: now, listeningUntilUnixMs: null },
        agent('gamma', 'idle'),
      ],
      onSelectAgent,
      selectedAgentId: null,
    });

    await user.type(screen.getByRole('searchbox', { name: 'Search agents' }), 'beta');
    expect(screen.getByRole('button', { name: /Beta/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /Alpha/u })).not.toBeInTheDocument();

    await user.clear(screen.getByRole('searchbox', { name: 'Search agents' }));
    const filter = screen.getByRole('combobox', { name: 'Filter by status' });
    expect(within(filter).queryByRole('option', { name: 'Working' })).not.toBeInTheDocument();
    await user.selectOptions(filter, 'Online · waiting for messages');
    expect(screen.getByRole('button', { name: /Alpha/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /Beta/u })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Gamma/u })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Alpha/u }));
    expect(onSelectAgent).toHaveBeenCalledWith('alpha');
  });

  it('名单上只说在不在等消息，不说工作状态和任务摘要', () => {
    const now = 1_700_000_000_000;
    renderRoster({
      agents: [
        { ...agent('alpha', 'working'), lastPolledAtUnixMs: now, listeningUntilUnixMs: now + 9000 },
      ],
      onSelectAgent: vi.fn(),
      selectedAgentId: null,
    });

    const row = screen.getByRole('button', { name: /Alpha/u });
    expect(row).toHaveTextContent('Online · waiting for messages');
    expect(row).not.toHaveTextContent('Working');
  });

  it('写名片的 Agent 按 Matrix 在线状态分组，离线的按离线时刻排', () => {
    const now = 1_700_000_000_000;
    const card = (agentId: string, presence: LobbyAgent['presence']): LobbyAgent => ({
      ...agent(agentId, 'idle'),
      // 名片是很久以前进房间时写的，租约早就过了。
      lastActiveAtUnixMs: now - 3 * 86_400_000,
      statusExpiresAtUnixMs: now - 3 * 86_400_000 + 300_000,
      liveness: 'presence',
      ...(presence === undefined ? {} : { presence }),
    });
    renderRoster({
      agents: [
        card('waiter', { state: 'online' }),
        card('worker', { state: 'unavailable' }),
        card('early', { state: 'offline', offlineSeenAtUnixMs: now - 50 * 60_000 }),
        card('late', { state: 'offline', offlineSeenAtUnixMs: now - 5 * 60_000 }),
      ],
      onSelectAgent: vi.fn(),
      selectedAgentId: null,
    });

    const rows = screen.getAllByRole('button', { name: /^(Waiter|Worker|Early|Late)/u });
    expect(rows.map((row) => row.querySelector('strong')?.textContent)).toEqual([
      'Waiter',
      'Worker',
      'Late',
      'Early',
    ]);
    expect(screen.getByRole('heading', { name: 'Online · waiting for messages' })).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Online · reads on next run' })).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Offline · under 1 hour' })).toBeVisible();
  });

  it('抽屉关闭时可把焦点还给已选中的列表项', () => {
    const rosterRef = createRef<ListModeRosterHandle>();
    renderRoster(
      {
        agents: [agent('alpha', 'working')],
        onSelectAgent: vi.fn(),
        selectedAgentId: 'alpha',
      },
      rosterRef,
    );

    act(() => {
      rosterRef.current?.focusSelected();
    });

    expect(screen.getByRole('button', { name: /Alpha/u })).toHaveFocus();
  });

  it('方向键、Home 和 End 在可见列表内移动焦点而不隐式打开详情', async () => {
    const user = userEvent.setup();
    const onSelectAgent = vi.fn();
    renderRoster({
      agents: [agent('alpha', 'working'), agent('beta', 'blocked'), agent('gamma', 'idle')],
      onSelectAgent,
      selectedAgentId: null,
    });
    const alpha = screen.getByRole('button', { name: /Alpha/u });
    alpha.focus();

    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: /Beta/u })).toHaveFocus();
    await user.keyboard('{End}');
    expect(screen.getByRole('button', { name: /Gamma/u })).toHaveFocus();
    await user.keyboard('{Home}');
    expect(alpha).toHaveFocus();
    expect(onSelectAgent).not.toHaveBeenCalled();
  });
});

describe('ListModeRoster 的网络 Agent 标记', () => {
  it('只在服务器确认的网络 Agent 名字旁标出“网络 Agent”', async () => {
    const network = '01990d9e-8400-7000-8000-000000000011';
    const local = '01990d9e-8400-7000-8000-000000000012';
    const store = new NetworkAgentLabelStore(
      { lookup: (ids) => Promise.resolve(ok(new Set(ids.filter((id) => id === network)))) },
      {
        schedule: (task) => {
          task();
        },
      },
    );
    render(
      <I18nextProvider i18n={i18n}>
        <NetworkAgentLabelsProvider store={store}>
          <ListModeRoster
            agents={[
              { ...agent('scout', 'idle'), agentId: network },
              { ...agent('pilot', 'idle'), agentId: local },
            ]}
            observedAtUnixMs={1_700_000_000_000}
            onSelectAgent={vi.fn()}
            selectedAgentId={null}
          />
        </NetworkAgentLabelsProvider>
      </I18nextProvider>,
    );

    const scout = screen.getByRole('button', { name: /^Scout/u });
    expect(await within(scout).findByText('Network agent')).toBeVisible();
    expect(
      within(screen.getByRole('button', { name: /^Pilot/u })).queryByText('Network agent'),
    ).not.toBeInTheDocument();
  });

  it('私人房间里标成“服务器代收发”', async () => {
    const network = '01990d9e-8400-7000-8000-000000000011';
    const store = new NetworkAgentLabelStore(
      { lookup: (ids) => Promise.resolve(ok(new Set(ids))) },
      {
        schedule: (task) => {
          task();
        },
      },
    );
    render(
      <I18nextProvider i18n={i18n}>
        <NetworkAgentLabelsProvider store={store}>
          <NetworkAgentRelayProvider relayed>
            <ListModeRoster
              agents={[{ ...agent('scout', 'idle'), agentId: network }]}
              observedAtUnixMs={1_700_000_000_000}
              onSelectAgent={vi.fn()}
              selectedAgentId={null}
            />
          </NetworkAgentRelayProvider>
        </NetworkAgentLabelsProvider>
      </I18nextProvider>,
    );

    const label = await within(screen.getByRole('button', { name: /^Scout/u })).findByText(
      'Network agent · relayed by the server',
    );
    expect(label).toHaveAttribute('title', expect.stringContaining('the server can read'));
  });
});

type RenderRosterOptions = {
  readonly agents: readonly LobbyAgent[];
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
};

function renderRoster(
  options: RenderRosterOptions,
  ref?: React.RefObject<ListModeRosterHandle | null>,
) {
  return render(
    <I18nextProvider i18n={i18n}>
      <ListModeRoster {...options} observedAtUnixMs={1_700_000_000_000} ref={ref} />
    </I18nextProvider>,
  );
}

function agent(agentId: string, status: LobbyAgentStatus): LobbyAgent {
  return {
    agentId,
    displayName: `${agentId[0]?.toLocaleUpperCase() ?? ''}${agentId.slice(1)}`,
    instanceIds: [`instance-${agentId}`],
    matrixUserId: `@${agentId}:agent-room.test`,
    status,
    statusExpiresAtUnixMs: 1_800_000_000_000,
    trust: 'unknown',
    visibility: 'coarse',
  };
}
