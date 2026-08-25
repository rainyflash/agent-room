// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, createRef } from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { LobbyAgent, LobbyAgentStatus } from '@/features/lobby/domain/lobby';
import { ListModeRoster, type ListModeRosterHandle } from '@/features/lobby/ui/list-mode-roster';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('ListModeRoster', () => {
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
      <ListModeRoster {...options} ref={ref} />
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
