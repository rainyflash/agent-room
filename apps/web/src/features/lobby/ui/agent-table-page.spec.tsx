// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AgentOwnerStore } from '@/features/lobby/application/agent-owner-store';
import type { AgentOwnerRecord } from '@/features/lobby/domain/agent-owners';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { AgentOwnersProvider, AgentOwnersScope } from '@/features/lobby/ui/agent-owners';
import { AgentTablePage, type AgentTablePageProps } from '@/features/lobby/ui/agent-table-page';
import { BrowserWorkspaceCache } from '@/features/personal-workspace/adapters/browser-workspace-cache';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  emptyWorkspace,
  type WorkspaceDocument,
} from '@/features/personal-workspace/domain/workspace-document';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const NOW = new Date(2026, 10, 14, 15, 30).getTime();
const CATALOG = '01990d9e-8400-7000-8000-000000000401';
const YOU = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
type Lifecycle = NonNullable<LobbyAgent['lifecycle']>;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

/** 服务器、收藏和标签都只认长得像 Agent ID 的。 */
const agentId = (index: number) => `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;

const online = (reception: 'waiting' | 'on_resume'): Lifecycle => ({
  archiveReason: null,
  archived: false,
  connection: 'online',
  offlineSinceUnixMs: null,
  reception,
});
const offline = (offlineForMs: number, archived = false): Lifecycle => ({
  archiveReason: archived ? 'expired' : null,
  archived,
  connection: 'offline',
  offlineSinceUnixMs: NOW - offlineForMs,
  reception: 'unavailable',
});

function agent(
  index: number,
  name: string,
  lifecycle: Lifecycle = online('on_resume'),
): LobbyAgent {
  return {
    agentId: agentId(index),
    displayName: name,
    instanceIds: [`instance-${String(index)}`],
    lifecycle,
    matrixUserId: `@agent-${String(index)}:agent-room.test`,
    status: 'idle',
    statusExpiresAtUnixMs: NOW + 60_000,
    trust: 'unknown',
    visibility: 'coarse',
  };
}

const mine = agent(1, 'Atlas', online('waiting'));
const hers = agent(2, 'Hers');
const net = agent(3, 'Comet');
const knocked = agent(4, 'Knocked');
const gone = agent(5, 'Gone', offline(3 * 86_400_000));
const old = agent(6, 'Old', offline(30 * 86_400_000, true));
const everyone = [mine, hers, net, knocked, gone, old];
const owners: readonly AgentOwnerRecord[] = [
  { agentId: mine.agentId, kind: 'local', owner: { displayName: 'Ada', principalId: YOU } },
  { agentId: hers.agentId, kind: 'local', owner: { displayName: 'Grace', principalId: GRACE } },
  { agentId: net.agentId, kind: 'network' },
  {
    agentId: knocked.agentId,
    kind: 'network',
    admittedBy: { displayName: 'Grace', principalId: GRACE },
  },
  { agentId: gone.agentId, kind: 'local', owner: { displayName: 'Ada', principalId: YOU } },
  { agentId: old.agentId, kind: 'local', owner: { displayName: 'Grace', principalId: GRACE } },
];

/** 你自己记的收藏和标签：一份只在内存里的。 */
function personalStore(): PersonalWorkspaceStore {
  const values = new Map<string, string>();
  let remote: WorkspaceDocument = emptyWorkspace;
  return new PersonalWorkspaceStore(
    {
      scope: () => ({ accountId: '@you:agent-room.test', writerId: 'this-device' }),
      read: () => Promise.resolve(ok(remote)),
      write: (_scope, document) => {
        remote = document;
        return Promise.resolve(ok(undefined));
      },
      subscribe: () => () => undefined,
    },
    new BrowserWorkspaceCache({
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        values.set(key, value);
      },
    }),
  );
}

function renderPage(
  props: Partial<AgentTablePageProps> & Pick<AgentTablePageProps, 'agents'>,
  personal: PersonalWorkspaceStore | null = null,
) {
  const onSelectAgent = vi.fn();
  const onInviteAgent = vi.fn();
  const store = new AgentOwnerStore(
    { lookup: () => Promise.resolve(ok([...owners])) },
    {
      schedule: (task) => {
        task();
      },
    },
  );
  const page = (
    <AgentOwnersProvider store={store}>
      <AgentOwnersScope catalogId={CATALOG} selfPrincipalId={YOU}>
        <AgentTablePage
          localAgentIds={null}
          observedAtUnixMs={NOW}
          onInviteAgent={onInviteAgent}
          onSelectAgent={onSelectAgent}
          selectedAgentId={null}
          wakeableAgentIds={new Set()}
          {...props}
        />
      </AgentOwnersScope>
    </AgentOwnersProvider>
  );
  render(
    <I18nextProvider i18n={i18n}>
      {personal === null ? (
        page
      ) : (
        <PersonalWorkspaceProvider store={personal}>{page}</PersonalWorkspaceProvider>
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
const names = () =>
  [...document.querySelectorAll('tbody .agent-table__label')].map((label) => label.textContent);
const filter = (group: string) => within(screen.getByRole('group', { name: group }));

describe('AgentTablePage', () => {
  it('六列：名字、谁的、状态、怎么来的、进来的时间、你的标签；上面是标题和汇总', async () => {
    renderPage({ agents: everyone });

    expect(screen.getByRole('heading', { level: 2, name: 'Agents in this room' })).toBeVisible();
    expect(screen.getByText('1 waiting')).toBeVisible();
    expect(screen.getByText('1 offline')).toBeVisible();
    expect(screen.getAllByRole('columnheader').map((header) => header.textContent)).toEqual([
      'Name',
      'Owner',
      'Status',
      'How it joined',
      'Joined',
      'Your tags',
    ]);
    // 归档的平时不列；在等的在前，离线的在后。
    expect(names()).toEqual(['Atlas', 'Comet', 'Hers', 'Knocked', 'Gone']);
    expect(await within(row(/^Atlas/u)).findAllByText('You')).not.toHaveLength(0);
    expect(row(/^Hers/u).querySelector('td.agent-table__owner')).toHaveTextContent('Grace');
    expect(row(/^Gone/u)).toHaveTextContent('Offline · 3 days ago');
  });

  it('怎么来的：网络 Agent 写网络，经谁放行的带上那个人；别的只说是电脑', async () => {
    renderPage({ agents: everyone });

    await within(row(/^Atlas/u)).findAllByText('You');
    const origin = (name: RegExp) => row(name).querySelector('.agent-table__origin');
    expect(origin(/^Comet/u)).toHaveTextContent(/^Network$/u);
    expect(origin(/^Knocked/u)).toHaveTextContent('Network · Let in by Grace');
    // 浏览器里问不到这台电脑上有谁：不猜。
    expect(origin(/^Atlas/u)).toHaveTextContent(/^A computer$/u);
    expect(origin(/^Hers/u)).toHaveTextContent(/^A computer$/u);
  });

  it('这台电脑的连接服务说它在这里的，写“这台电脑”', async () => {
    renderPage({ agents: everyone, localAgentIds: new Set([mine.agentId]) });

    await within(row(/^Atlas/u)).findAllByText('You');
    expect(row(/^Atlas/u).querySelector('.agent-table__origin')).toHaveTextContent('This computer');
    expect(row(/^Hers/u).querySelector('.agent-table__origin')).toHaveTextContent('A computer');
  });

  it('进来的时间用名片的时间；续租约的旧写法不知道，那一格空着', () => {
    const today = new Date(2026, 10, 14, 9, 12).getTime();
    const earlier = new Date(2026, 10, 3, 8, 0).getTime();
    renderPage({
      agents: [
        { ...mine, lastActiveAtUnixMs: today, liveness: 'presence' },
        { ...hers, lastActiveAtUnixMs: earlier, liveness: 'presence' },
        { ...net, lastActiveAtUnixMs: today },
      ],
    });

    const joined = (name: RegExp) => row(name).querySelector('.agent-table__joined');
    expect(joined(/^Atlas/u)).toHaveTextContent(/^Today 09:12/u);
    expect(joined(/^Hers/u)).toHaveTextContent('Nov 3');
    expect(joined(/^Comet/u)).toBeEmptyDOMElement();
  });

  it('按状态筛；归档的只在有的时候多一个选项', async () => {
    const user = userEvent.setup();
    renderPage({ agents: everyone, wakeableAgentIds: new Set([hers.agentId]) });
    const state = filter('Status');

    expect(state.getByRole('button', { name: 'All' })).toHaveAttribute('aria-pressed', 'true');
    await user.click(state.getByRole('button', { name: 'Waiting' }));
    expect(names()).toEqual(['Atlas']);
    expect(state.getByRole('button', { name: 'Waiting' })).toHaveAttribute('aria-pressed', 'true');
    await user.click(state.getByRole('button', { name: 'Wakes on mention' }));
    expect(names()).toEqual(['Hers']);
    await user.click(state.getByRole('button', { name: 'Offline' }));
    expect(names()).toEqual(['Gone']);
    await user.click(state.getByRole('button', { name: 'Archived (1)' }));
    expect(names()).toEqual(['Old']);
    await user.click(state.getByRole('button', { name: 'All' }));
    expect(names()).toHaveLength(5);
  });

  it('没有归档的就没有那个选项', () => {
    renderPage({ agents: [mine, hers] });

    expect(filter('Status').queryByRole('button', { name: /^Archived/u })).not.toBeInTheDocument();
  });

  it('按谁的筛：我的、网络 Agent；和状态、搜索一起用时都要对上', async () => {
    const user = userEvent.setup();
    renderPage({ agents: everyone });
    await within(row(/^Atlas/u)).findAllByText('You');
    const owner = filter('Owner');

    await user.click(owner.getByRole('button', { name: 'Mine' }));
    expect(names()).toEqual(['Atlas', 'Gone']);
    await user.click(filter('Status').getByRole('button', { name: 'Offline' }));
    expect(names()).toEqual(['Gone']);
    await user.click(owner.getByRole('button', { name: 'Network agents' }));
    expect(screen.getByText('No agent matches that.')).toBeVisible();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    await user.click(filter('Status').getByRole('button', { name: 'All' }));
    expect(names()).toEqual(['Comet', 'Knocked']);
    await user.type(screen.getByRole('searchbox', { name: 'Search agents' }), 'kno');
    expect(names()).toEqual(['Knocked']);
  });

  it('你没收藏过、也没打过标签时没有“你的标签”那一组', () => {
    renderPage({ agents: everyone }, personalStore());

    expect(screen.queryByRole('group', { name: 'Your tags' })).not.toBeInTheDocument();
    expect(screen.getByRole('group', { name: 'Owner' })).toBeVisible();
  });

  it('收藏和标签写在最后一列，能按它们筛；记号摘掉以后回到全部', async () => {
    const user = userEvent.setup();
    const personal = personalStore();
    renderPage({ agents: everyone }, personal);
    act(() => {
      personal.change({ kind: 'favorite', id: hers.agentId, value: true });
      personal.change({ kind: 'tags', id: mine.agentId, value: ['release', 'evals'] });
    });

    const marks = (name: RegExp) =>
      [...row(name).querySelectorAll('.agent-table__marks > span')].map((mark) => mark.textContent);
    expect(marks(/^Hers/u)).toEqual(['Favorite']);
    expect(marks(/^Atlas/u)).toEqual(['release', 'evals']);
    // 收藏已经写在这一列了，名字后面不再画星。
    expect(row(/^Hers/u).querySelector('.agent-table__favorite')).toBeNull();

    const mark = filter('Your tags');
    expect(mark.getAllByRole('button').map((button) => button.textContent)).toEqual([
      'All',
      'Favorites',
      'evals',
      'release',
    ]);
    await user.click(mark.getByRole('button', { name: 'Favorites' }));
    expect(names()).toEqual(['Hers']);
    await user.click(mark.getByRole('button', { name: 'release' }));
    expect(names()).toEqual(['Atlas']);

    act(() => {
      personal.change({ kind: 'tags', id: mine.agentId, value: ['evals'] });
    });
    expect(names()).toHaveLength(5);
    expect(filter('Your tags').getByRole('button', { name: 'All' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('点一行、点名字都是选中这个 Agent，只算一次；选中的那一行按钮是按下的', async () => {
    const user = userEvent.setup();
    const { onSelectAgent } = renderPage({ agents: everyone, selectedAgentId: hers.agentId });

    expect(screen.getByRole('button', { name: /^Hers/u })).toHaveAttribute('aria-pressed', 'true');
    expect(row(/^Hers/u)).toHaveAttribute('data-selected', 'true');
    await user.click(screen.getByRole('button', { name: /^Atlas/u }));
    expect(onSelectAgent).toHaveBeenCalledTimes(1);
    expect(onSelectAgent).toHaveBeenLastCalledWith(mine.agentId);
    const origin = row(/^Comet/u).querySelector('.agent-table__origin');
    if (origin === null) throw new Error('没有“怎么来的”那一格。');
    await user.click(origin);
    expect(onSelectAgent).toHaveBeenCalledTimes(2);
    expect(onSelectAgent).toHaveBeenLastCalledWith(net.agentId);
  });

  it('方向键、Home 和 End 在表格里上下走，不顺手打开详情', async () => {
    const user = userEvent.setup();
    const { onSelectAgent } = renderPage({ agents: everyone });
    const first = screen.getByRole('button', { name: /^Atlas/u });
    first.focus();

    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('button', { name: /^Comet/u })).toHaveFocus();
    await user.keyboard('{End}');
    expect(screen.getByRole('button', { name: /^Gone/u })).toHaveFocus();
    await user.keyboard('{Home}');
    expect(first).toHaveFocus();
    expect(onSelectAgent).not.toHaveBeenCalled();
  });

  it('一次列 100 个，下面写着列了几个、一共几个；点“再看”接着往下列', async () => {
    const user = userEvent.setup();
    renderPage({
      agents: Array.from({ length: 130 }, (_, index) =>
        agent(index + 10, `Agent ${String(index).padStart(3, '0')}`),
      ),
    });

    expect(screen.getAllByRole('row')).toHaveLength(101);
    expect(screen.getByText('Showing 100 of 130')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Show 30 more' }));
    expect(screen.getAllByRole('row')).toHaveLength(131);
    expect(screen.getByText('Showing 130 of 130')).toBeVisible();
    expect(screen.queryByRole('button', { name: /^Show \d+ more$/u })).not.toBeInTheDocument();
    // 换了筛选从头列起；不到 100 个时下面什么都不写。
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search agents' }), {
      target: { value: 'agent 12' },
    });
    await waitFor(() => {
      expect(screen.getAllByRole('row')).toHaveLength(11);
    });
    expect(screen.queryByText(/^Showing/u)).not.toBeInTheDocument();
    // 一百多行的表格在整套测试一起跑的时候画得慢，多给一点时间。
  }, 20_000);

  it('不到 100 个时下面什么都不写', () => {
    renderPage({ agents: everyone });

    expect(screen.queryByText(/^Showing/u)).not.toBeInTheDocument();
  });

  it('还没有 Agent：一句话加“接入 Agent”，不画筛选和表格', async () => {
    const user = userEvent.setup();
    const { onInviteAgent } = renderPage({ agents: [] });

    expect(screen.getByText('No agents here yet')).toBeVisible();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.queryByRole('group', { name: 'Status' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Bring an agent' }));
    expect(onInviteAgent).toHaveBeenCalledTimes(1);
  });

  it('不能再接入的房间（归档了）不给那个按钮；表格下面的东西照样放', () => {
    renderPage({ agents: [], footer: <p>Archiving rules</p>, onInviteAgent: null });

    expect(screen.queryByRole('button', { name: 'Bring an agent' })).not.toBeInTheDocument();
    expect(screen.getByText('Archiving rules')).toBeVisible();
  });
});
