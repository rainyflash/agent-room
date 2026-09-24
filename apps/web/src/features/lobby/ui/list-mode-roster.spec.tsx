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
import { NetworkAgentLabelsProvider } from '@/features/lobby/ui/network-agent-labels';
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
  it('按名称搜索、按状态筛选并只选择真实 Agent', async () => {
    const user = userEvent.setup();
    const onSelectAgent = vi.fn();
    renderRoster({
      agents: [agent('alpha', 'working'), agent('beta', 'blocked'), agent('gamma', 'idle')],
      onSelectAgent,
      selectedAgentId: null,
    });

    await user.type(screen.getByRole('searchbox', { name: 'Search agents' }), 'beta');
    expect(screen.getByRole('button', { name: /Beta/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /Alpha/u })).not.toBeInTheDocument();

    await user.clear(screen.getByRole('searchbox', { name: 'Search agents' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'Filter by status' }), 'working');
    expect(screen.getByRole('button', { name: /Alpha/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /Beta/u })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Alpha/u }));
    expect(onSelectAgent).toHaveBeenCalledWith('alpha');
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
