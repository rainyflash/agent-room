// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AgentOwnerStore } from '@/features/lobby/application/agent-owner-store';
import type { AgentOwnerRecord } from '@/features/lobby/domain/agent-owners';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { AgentOwnersProvider, AgentOwnersScope } from '@/features/lobby/ui/agent-owners';
import { RoomRoster, type RoomRosterProps } from '@/features/lobby/ui/room-roster';
import type { RoomPeople } from '@/features/private-rooms/domain/room-people';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const NOW = 1_700_000_000_000;
const CATALOG = '01990d9e-8400-7000-8000-000000000401';
const YOU = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
type Lifecycle = NonNullable<LobbyAgent['lifecycle']>;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

/** 服务器只回答长得像 Agent ID 的：问“谁的”的用例用这个。 */
const agentId = (index: number) => `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;

/** 连着的：在等、下次运行时读、没说。 */
const online = (reception: 'waiting' | 'on_resume' | 'unknown'): Lifecycle => ({
  archiveReason: null,
  archived: false,
  connection: 'online',
  offlineSinceUnixMs: null,
  reception,
});
/** 没连着的：重连中，或者离线了多久；`archived` 是离线太久收进归档的。 */
const away = (
  connection: 'reconnecting' | 'offline',
  offlineForMs: number | null = null,
  archived = false,
): Lifecycle => ({
  archiveReason: archived ? 'expired' : null,
  archived,
  connection,
  offlineSinceUnixMs: offlineForMs === null ? null : NOW - offlineForMs,
  reception: 'unavailable',
});

function agent(name: string, lifecycle: Lifecycle = online('on_resume')): LobbyAgent {
  return {
    agentId: name.toLocaleLowerCase().replaceAll(/[^a-z0-9]/gu, '-'),
    displayName: name,
    instanceIds: [`instance-${name}`],
    lifecycle,
    matrixUserId: `@${name.toLocaleLowerCase().replaceAll(/[^a-z0-9]/gu, '-')}:agent-room.test`,
    status: 'idle',
    statusExpiresAtUnixMs: NOW + 60_000,
    trust: 'unknown',
    visibility: 'coarse',
  };
}

const waiter = agent('Waiter', online('waiting'));
const sleeper = agent('Sleeper');
const gone = agent('Gone', away('offline', 3 * 86_400_000));
const old = agent('Old', away('offline', 30 * 86_400_000, true));

const nobody: RoomPeople = { listed: true, people: [] };

function renderRoster(
  props: Partial<RoomRosterProps> & Pick<RoomRosterProps, 'agents'>,
  owners?: readonly AgentOwnerRecord[],
) {
  const onSelectAgent = vi.fn();
  const onInviteAgent = vi.fn();
  const roster = (
    <RoomRoster
      observedAtUnixMs={NOW}
      onInviteAgent={onInviteAgent}
      onSelectAgent={onSelectAgent}
      people={nobody}
      selectedAgentId={null}
      wakeableAgentIds={new Set()}
      {...props}
    />
  );
  const store =
    owners === undefined
      ? null
      : new AgentOwnerStore(
          { lookup: () => Promise.resolve(ok([...owners])) },
          {
            schedule: (task) => {
              task();
            },
          },
        );
  render(
    <I18nextProvider i18n={i18n}>
      {store === null ? (
        roster
      ) : (
        <AgentOwnersProvider store={store}>
          <AgentOwnersScope catalogId={CATALOG} selfPrincipalId={YOU}>
            {roster}
          </AgentOwnersScope>
        </AgentOwnersProvider>
      )}
    </I18nextProvider>,
  );
  return { onInviteAgent, onSelectAgent };
}

const row = (name: RegExp) => {
  const element = screen.getByRole('button', { name }).closest('tr');
  if (element === null) throw new Error('这个 Agent 不在表格里。');
  return element;
};

describe('RoomRoster 的 Agent 表格', () => {
  it('两个标签带着个数；Agent 下面是汇总、搜索和三列的表格', () => {
    renderRoster({
      agents: [gone, sleeper, waiter],
      people: {
        listed: true,
        people: [{ key: YOU, displayName: 'Ada', self: true, role: 'owner', principalId: YOU }],
      },
    });

    expect(screen.getByRole('tab', { name: 'Agents 3' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'People 1' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByText('1 waiting')).toBeVisible();
    expect(screen.getByText('1 offline')).toBeVisible();
    // 没有叫得醒的就不写那一段。
    expect(screen.queryByText(/wakes? on mention/u)).not.toBeInTheDocument();
    expect(screen.getByRole('searchbox', { name: 'Search agents' })).toBeVisible();
    expect(screen.getAllByRole('columnheader').map((header) => header.textContent)).toEqual([
      'Name',
      'Owner',
      'Status',
    ]);
  });

  it('在等的在前、离线的在后，状态一列一直带字；离线的写离线了多久', () => {
    renderRoster({ agents: [gone, sleeper, waiter] });

    const rows = screen.getAllByRole('row').slice(1);
    expect(rows.map((entry) => entry.getAttribute('data-state'))).toEqual([
      'waiting',
      'on_resume',
      'offline',
    ]);
    expect(within(row(/^Waiter/u)).getByText('Waiting')).toBeVisible();
    expect(within(row(/^Sleeper/u)).getByText('Reads on next run')).toBeVisible();
    // 看得见的是“3 天前”，“离线”两个字留给读屏。
    expect(row(/^Gone/u)).toHaveTextContent('Offline · 3 days ago');
    expect(row(/^Waiter/u).querySelector('.ar-presence-dot--waiting')).not.toBeNull();
    expect(row(/^Sleeper/u).querySelector('.ar-presence-dot')).toBeNull();
    expect(row(/^Gone/u).querySelector('.ar-avatar--dimmed')).not.toBeNull();
  });

  it('开着后台回复的标“叫得醒”，头像上是圈；汇总里也数上', () => {
    renderRoster({ agents: [gone, sleeper, waiter], wakeableAgentIds: new Set([gone.agentId]) });

    expect(row(/^Gone/u)).toHaveAttribute('data-state', 'wakes');
    expect(within(row(/^Gone/u)).getByText('Wakes on mention')).toBeVisible();
    expect(row(/^Gone/u).querySelector('.ar-presence-dot--wakes')).not.toBeNull();
    expect(screen.getByText('1 wakes on mention')).toBeVisible();
    // 排在在等的后面、别的前面。
    expect(
      screen
        .getAllByRole('row')
        .slice(1)
        .map((entry) => entry.getAttribute('data-state')),
    ).toEqual(['waiting', 'wakes', 'on_resume']);
  });

  it('“谁的”一列：你、主人的昵称、网络', async () => {
    const mine = { ...agent('Mine'), agentId: agentId(1) };
    const hers = { ...agent('Hers'), agentId: agentId(2) };
    const net = { ...agent('Net'), agentId: agentId(3) };
    renderRoster({ agents: [mine, hers, net] }, [
      { agentId: mine.agentId, kind: 'local', owner: { displayName: 'Ada', principalId: YOU } },
      { agentId: hers.agentId, kind: 'local', owner: { displayName: 'Grace', principalId: GRACE } },
      { agentId: net.agentId, kind: 'network' },
    ]);

    expect(await within(row(/^Mine/u)).findAllByText('You')).not.toHaveLength(0);
    expect(row(/^Hers/u).querySelector('td.agent-table__owner')).toHaveTextContent('Grace');
    expect(row(/^Net/u).querySelector('td.agent-table__owner')).toHaveTextContent('Network');
  });

  it('名字后面的编号单独一段：地方不够时截前面、留着编号', () => {
    renderRoster({ agents: [agent('codex #2316')] });

    const name = screen.getByRole('button', { name: /^codex/u });
    expect(name).toHaveTextContent('codex #2316');
    const label = name.querySelector('.agent-table__label');
    expect([...(label?.children ?? [])].map((part) => part.textContent)).toEqual([
      'codex',
      ' #2316',
    ]);
  });

  it('点一行、点名字都是选中这个 Agent，只算一次', async () => {
    const user = userEvent.setup();
    const { onSelectAgent } = renderRoster({ agents: [waiter, sleeper] });

    await user.click(screen.getByRole('button', { name: /^Waiter/u }));
    expect(onSelectAgent).toHaveBeenCalledTimes(1);
    expect(onSelectAgent).toHaveBeenLastCalledWith(waiter.agentId);

    await user.click(within(row(/^Sleeper/u)).getByText('Reads on next run'));
    expect(onSelectAgent).toHaveBeenCalledTimes(2);
    expect(onSelectAgent).toHaveBeenLastCalledWith(sleeper.agentId);
  });

  it('选中的那一行按钮是按下的', () => {
    renderRoster({ agents: [waiter, sleeper], selectedAgentId: sleeper.agentId });

    expect(screen.getByRole('button', { name: /^Sleeper/u })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(row(/^Sleeper/u)).toHaveAttribute('data-selected', 'true');
    expect(screen.getByRole('button', { name: /^Waiter/u })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  it('方向键、Home 和 End 在表格里上下走，不顺手打开详情', async () => {
    const user = userEvent.setup();
    const { onSelectAgent } = renderRoster({ agents: [waiter, sleeper, gone] });
    const first = screen.getByRole('button', { name: /^Waiter/u });
    first.focus();

    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: /^Sleeper/u })).toHaveFocus();
    await user.keyboard('{End}');
    expect(screen.getByRole('button', { name: /^Gone/u })).toHaveFocus();
    await user.keyboard('{Home}');
    expect(first).toHaveFocus();
    expect(onSelectAgent).not.toHaveBeenCalled();
  });

  it('按名字搜；找不到时说一句', async () => {
    const user = userEvent.setup();
    renderRoster({ agents: [waiter, sleeper] });

    await user.type(screen.getByRole('searchbox', { name: 'Search agents' }), 'sleep');
    expect(screen.getByRole('button', { name: /^Sleeper/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /^Waiter/u })).not.toBeInTheDocument();

    await user.type(screen.getByRole('searchbox', { name: 'Search agents' }), 'zzz');
    expect(screen.getByText('No agent matches that.')).toBeVisible();
  });

  it('归档的平时不列、不算在个数里；点“已归档”才看，能点回来', async () => {
    const user = userEvent.setup();
    renderRoster({ agents: [waiter, old] });

    expect(screen.getByRole('tab', { name: 'Agents 1' })).toBeVisible();
    expect(screen.queryByRole('button', { name: /^Old/u })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Archived (1)' }));
    expect(screen.getByRole('button', { name: /^Old/u })).toBeVisible();
    expect(screen.queryByRole('button', { name: /^Waiter/u })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Members (1)' }));
    expect(screen.getByRole('button', { name: /^Waiter/u })).toBeVisible();
  });

  it('只剩归档的时算没有 Agent，归档的照样点得开', async () => {
    const user = userEvent.setup();
    renderRoster({ agents: [old] });

    expect(screen.getByRole('tab', { name: 'Agents 0' })).toBeVisible();
    expect(screen.getByText('No agents here yet')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Archived (1)' }));
    expect(screen.getByRole('button', { name: /^Old/u })).toBeVisible();
  });

  it('正看着归档时归档的都回来了：自己回到成员那一页，不停在空表上', async () => {
    const user = userEvent.setup();
    const props = {
      observedAtUnixMs: NOW,
      onInviteAgent: null,
      onSelectAgent: vi.fn(),
      people: nobody,
      selectedAgentId: null,
      wakeableAgentIds: new Set<string>(),
    };
    const { rerender } = render(
      <I18nextProvider i18n={i18n}>
        <RoomRoster {...props} agents={[waiter, old]} />
      </I18nextProvider>,
    );
    await user.click(screen.getByRole('button', { name: 'Archived (1)' }));
    expect(screen.queryByRole('button', { name: /^Waiter/u })).not.toBeInTheDocument();

    rerender(
      <I18nextProvider i18n={i18n}>
        <RoomRoster {...props} agents={[waiter, agent('Old', online('waiting'))]} />
      </I18nextProvider>,
    );
    expect(screen.getByRole('button', { name: /^Waiter/u })).toBeVisible();
    expect(screen.getByRole('button', { name: /^Old/u })).toBeVisible();
    expect(
      screen.queryByRole('button', { name: /^(Archived|Members) \(/u }),
    ).not.toBeInTheDocument();
  });

  it('一页最多 100 个，多了翻页', async () => {
    const user = userEvent.setup();
    renderRoster({
      agents: Array.from({ length: 230 }, (_, index) =>
        agent(`Agent ${String(index).padStart(3, '0')}`),
      ),
    });

    expect(screen.getAllByRole('row')).toHaveLength(101);
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByText('2 / 3')).toBeVisible();
    expect(screen.getByRole('button', { name: /^Agent 100/u })).toBeVisible();
  });

  it('没有 Agent：一句话加“接入 Agent”；这间房不能接入时不给按钮', async () => {
    const user = userEvent.setup();
    const { onInviteAgent } = renderRoster({ agents: [] });

    expect(screen.getByText('No agents here yet')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Bring an agent' }));
    expect(onInviteAgent).toHaveBeenCalledTimes(1);

    cleanup();
    renderRoster({ agents: [], onInviteAgent: null });
    expect(screen.getByText('No agents here yet')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Bring an agent' })).not.toBeInTheDocument();
  });

  it('表格下面能挂别的东西（离线归档的规则）', () => {
    renderRoster({ agents: [waiter], footer: <p>Offline archiving</p> });

    expect(screen.getByText('Offline archiving')).toBeVisible();
  });
});

const personRow = (name: string) => {
  const element = screen
    .getByRole('rowheader', { name: new RegExp(`^${name}`, 'u') })
    .closest('tr');
  if (element === null) throw new Error('这个人不在表格里。');
  return element;
};

describe('RoomRoster 的“人”', () => {
  const people: RoomPeople = {
    listed: true,
    people: [
      { key: YOU, displayName: 'Ada', self: true, role: 'owner', principalId: YOU },
      { key: GRACE, displayName: 'Grace', self: false, role: 'admin', principalId: GRACE },
      { key: 'zed', displayName: 'Zed', self: false, role: null, principalId: 'zed' },
    ],
  };

  it('昵称、身份、带了几个 Agent', async () => {
    const user = userEvent.setup();
    const mine = { ...agent('Mine'), agentId: agentId(1) };
    const hers = { ...agent('Hers'), agentId: agentId(2) };
    const alsoHers = { ...agent('Also hers'), agentId: agentId(3) };
    renderRoster({ agents: [mine, hers, alsoHers], people }, [
      { agentId: mine.agentId, kind: 'local', owner: { displayName: 'Ada', principalId: YOU } },
      { agentId: hers.agentId, kind: 'local', owner: { displayName: 'Grace', principalId: GRACE } },
      {
        agentId: alsoHers.agentId,
        kind: 'local',
        owner: { displayName: 'Grace', principalId: GRACE },
      },
    ]);

    await user.click(screen.getByRole('tab', { name: 'People 3' }));
    expect(screen.getAllByRole('columnheader').map((header) => header.textContent)).toEqual([
      'Name',
      'Role',
      'Agents',
    ]);
    const cells = (name: string) =>
      within(personRow(name))
        .getAllByRole('cell')
        .map((cell) => cell.textContent);
    expect(screen.getByRole('rowheader', { name: /^Ada/u })).toHaveTextContent('Ada you');
    expect(cells('Ada')).toEqual(['Owner', '1']);
    expect(cells('Grace')).toEqual(['Admin', '2']);
    expect(cells('Zed')).toEqual(['', '0']);
  });

  it('没问到 Agent 是谁的时不写个数，免得写成 0', async () => {
    const user = userEvent.setup();
    renderRoster({ agents: [agent('Mine')], people });

    await user.click(screen.getByRole('tab', { name: 'People 3' }));
    expect(
      within(personRow('Grace'))
        .getAllByRole('cell')
        .map((cell) => cell.textContent),
    ).toEqual(['Admin', '']);
  });

  it('没有成员名单的房间说一句：列的是说过话的人', async () => {
    const user = userEvent.setup();
    renderRoster({
      agents: [],
      people: {
        listed: false,
        people: [
          { key: '@you', displayName: 'Ada', self: true, role: null, principalId: null },
          { key: '@guest', displayName: 'Guest', self: false, role: null, principalId: null },
        ],
      },
    });

    await user.click(screen.getByRole('tab', { name: 'People 2' }));
    expect(screen.getByRole('rowheader', { name: /^Guest/u })).toBeVisible();
    expect(screen.getByText(/has no member list/u)).toBeVisible();
  });

  it('标签能用左右键换', async () => {
    const user = userEvent.setup();
    renderRoster({ agents: [waiter], people });
    screen.getByRole('tab', { name: 'Agents 1' }).focus();

    await user.keyboard('{ArrowRight}');
    expect(screen.getByRole('tab', { name: 'People 3' })).toHaveFocus();
    expect(screen.getByRole('tab', { name: 'People 3' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel', { name: 'People 3' })).toBeVisible();
    await user.keyboard('{ArrowLeft}');
    expect(screen.getByRole('tab', { name: 'Agents 1' })).toHaveFocus();
  });
});
