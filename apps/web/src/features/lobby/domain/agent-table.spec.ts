import { describe, expect, it } from 'vitest';

import {
  agentJoinedAtUnixMs,
  agentNameParts,
  agentOrigin,
  agentRowForKey,
  agentRowState,
  agentRowSummary,
  agentRows,
  filterAgentRows,
  noAgentRowFilter,
  searchAgentRows,
  type AgentRowFacts,
} from './agent-table';
import type { LobbyAgent } from './lobby';

const NOW = 1_700_000_000_000;
type Lifecycle = NonNullable<LobbyAgent['lifecycle']>;

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
    agentId: name.toLocaleLowerCase(),
    displayName: name,
    instanceIds: [`instance-${name}`],
    lifecycle,
    matrixUserId: `@${name.toLocaleLowerCase()}:agent-room.test`,
    status: 'idle',
    statusExpiresAtUnixMs: NOW + 60_000,
    trust: 'unknown',
    visibility: 'coarse',
  };
}

const waiting = agent('Waiter', online('waiting'));
const resume = agent('Resume');
const unknown = agent('Unknown', online('unknown'));
const reconnecting = agent('Reconnecting', away('reconnecting'));
const offline = (name: string, minutesAgo: number, archived = false) =>
  agent(name, away('offline', minutesAgo * 60_000, archived));

describe('agentRowState', () => {
  it('没在等消息、开着后台回复的是叫得醒；连着没在等、重连中、离线都算', () => {
    expect(agentRowState(resume, NOW, true)).toBe('wakes');
    expect(agentRowState(unknown, NOW, true)).toBe('wakes');
    expect(agentRowState(reconnecting, NOW, true)).toBe('wakes');
    expect(agentRowState(offline('Gone', 5), NOW, true)).toBe('wakes');
  });

  it('正在等消息的照旧算在等：它自己就会看到', () => {
    expect(agentRowState(waiting, NOW, true)).toBe('waiting');
  });

  it('没开后台回复的照名单上的五种说', () => {
    expect(agentRowState(waiting, NOW, false)).toBe('waiting');
    expect(agentRowState(resume, NOW, false)).toBe('on_resume');
    expect(agentRowState(unknown, NOW, false)).toBe('unknown');
    expect(agentRowState(reconnecting, NOW, false)).toBe('reconnecting');
    expect(agentRowState(offline('Gone', 5), NOW, false)).toBe('offline');
  });
});

describe('agentRows', () => {
  it('在等的在前，然后叫得醒、下次运行时读、在线、重连中；离线的最近离线的在前', () => {
    const early = offline('Early', 50);
    const late = offline('Late', 5);
    const woken = offline('Woken', 30);
    const rows = agentRows(
      [early, reconnecting, unknown, late, resume, woken, waiting],
      NOW,
      new Set([woken.agentId]),
    );
    expect(rows.map((row) => `${row.agent.displayName}:${row.state}`)).toEqual([
      'Waiter:waiting',
      'Woken:wakes',
      'Resume:on_resume',
      'Unknown:unknown',
      'Reconnecting:reconnecting',
      'Late:offline',
      'Early:offline',
    ]);
  });

  it('同一种状态里按名字排，离线的带着离线的时刻', () => {
    const rows = agentRows([agent('Beta'), agent('Alpha'), offline('Gone', 5)], NOW, new Set());
    expect(rows.map((row) => row.agent.displayName)).toEqual(['Alpha', 'Beta', 'Gone']);
    expect(rows.map((row) => row.offlineSinceUnixMs)).toEqual([null, null, NOW - 5 * 60_000]);
  });

  it('归档的照样排进去，带着标记：平时不列，点“已归档”才看', () => {
    const rows = agentRows([offline('Old', 20_000, true), waiting], NOW, new Set());
    expect(rows.map((row) => row.archived)).toEqual([false, true]);
  });
});

describe('agentRowSummary', () => {
  it('数在等的、叫得醒的、离线的；归档的不算，别的状态不在这一行里', () => {
    const woken = offline('Woken', 30);
    const rows = agentRows(
      [waiting, resume, unknown, woken, offline('Gone', 5), offline('Old', 20_000, true)],
      NOW,
      new Set([woken.agentId]),
    );
    expect(agentRowSummary(rows)).toEqual({ waiting: 1, wakes: 1, offline: 1 });
  });
});

describe('searchAgentRows', () => {
  const rows = agentRows([waiting, resume], NOW, new Set());

  it('按名字或 Matrix ID 找，不分大小写', () => {
    expect(searchAgentRows(rows, 'WAIT').map((row) => row.agent.displayName)).toEqual(['Waiter']);
    expect(searchAgentRows(rows, '@resume:').map((row) => row.agent.displayName)).toEqual([
      'Resume',
    ]);
  });

  it('什么都没填时原样给回去', () => {
    expect(searchAgentRows(rows, '  ')).toBe(rows);
  });
});

describe('agentNameParts', () => {
  it('把后面的四位编号拆出来，两段接起来还是原来的名字', () => {
    expect(agentNameParts('codex #2316')).toEqual({ base: 'codex', tag: ' #2316' });
    expect(agentNameParts('Research helper for the long run #0042')).toEqual({
      base: 'Research helper for the long run',
      tag: ' #0042',
    });
  });

  it('没有编号、编号不在最后、位数不对的整个算名字', () => {
    expect(agentNameParts('Scout')).toEqual({ base: 'Scout', tag: '' });
    expect(agentNameParts('#2316 codex')).toEqual({ base: '#2316 codex', tag: '' });
    expect(agentNameParts('codex #231')).toEqual({ base: 'codex #231', tag: '' });
  });
});

describe('filterAgentRows', () => {
  const woken = offline('Woken', 30);
  const gone = offline('Gone', 5);
  const old = offline('Old', 20_000, true);
  const rows = agentRows([waiting, resume, woken, gone, old], NOW, new Set([woken.agentId]));
  const facts: AgentRowFacts = {
    favorite: (agentId) => agentId === resume.agentId,
    mine: (agentId) => agentId === waiting.agentId || agentId === old.agentId,
    network: (agentId) => agentId === gone.agentId,
    tags: (agentId) => (agentId === waiting.agentId ? ['release', 'evals'] : []),
  };
  const names = (filter: Partial<typeof noAgentRowFilter>) =>
    filterAgentRows(rows, { ...noAgentRowFilter, ...filter }, facts).map(
      (row) => row.agent.displayName,
    );

  it('什么都不筛时列没归档的；归档的只在筛“已归档”时列', () => {
    expect(names({})).toEqual(['Waiter', 'Woken', 'Resume', 'Gone']);
    expect(names({ state: 'archived' })).toEqual(['Old']);
  });

  it('按状态筛：在等消息、叫得醒、离线各是各的', () => {
    expect(names({ state: 'waiting' })).toEqual(['Waiter']);
    expect(names({ state: 'wakes' })).toEqual(['Woken']);
    // 开着后台回复的那个离线 Agent 算叫得醒，不在“离线”里再数一遍。
    expect(names({ state: 'offline' })).toEqual(['Gone']);
  });

  it('按谁的筛：我的、网络 Agent', () => {
    expect(names({ owner: 'mine' })).toEqual(['Waiter']);
    expect(names({ owner: 'network' })).toEqual(['Gone']);
    expect(names({ owner: 'mine', state: 'archived' })).toEqual(['Old']);
  });

  it('按你做的记号筛：收藏的、带某个标签的；几样一起筛时都要对上', () => {
    expect(names({ mark: { kind: 'favorites' } })).toEqual(['Resume']);
    expect(names({ mark: { kind: 'tag', tag: 'evals' } })).toEqual(['Waiter']);
    expect(names({ mark: { kind: 'tag', tag: 'nothing' } })).toEqual([]);
    expect(names({ mark: { kind: 'favorites' }, owner: 'mine' })).toEqual([]);
  });
});

describe('agentOrigin', () => {
  it('网络 Agent 是网络；这台电脑的连接服务说它在这里才写这台电脑', () => {
    expect(agentOrigin('a', true, new Set(['a']))).toBe('network');
    expect(agentOrigin('a', false, new Set(['a']))).toBe('this_computer');
    expect(agentOrigin('a', false, new Set(['b']))).toBe('computer');
  });

  it('问不到这台电脑上有谁（浏览器里、还没读到）时只说是电脑', () => {
    expect(agentOrigin('a', false, null)).toBe('computer');
  });
});

describe('agentJoinedAtUnixMs', () => {
  it('写名片的 Agent 用名片的时间；续租约的那种不知道', () => {
    const joined = NOW - 3_600_000;
    expect(
      agentJoinedAtUnixMs({ ...waiting, lastActiveAtUnixMs: joined, liveness: 'presence' }),
    ).toBe(joined);
    expect(agentJoinedAtUnixMs({ ...waiting, liveness: 'presence' })).toBeNull();
    expect(agentJoinedAtUnixMs({ ...waiting, lastActiveAtUnixMs: joined })).toBeNull();
  });
});

describe('agentRowForKey', () => {
  const rows = agentRows([agent('Alpha'), agent('Beta'), agent('Gamma')], NOW, new Set());

  it('上下键走一行，到头就停；Home、End 到两头', () => {
    expect(agentRowForKey(rows, 'beta', 'ArrowDown')).toBe('gamma');
    expect(agentRowForKey(rows, 'gamma', 'ArrowDown')).toBe('gamma');
    expect(agentRowForKey(rows, 'beta', 'ArrowUp')).toBe('alpha');
    expect(agentRowForKey(rows, 'alpha', 'ArrowUp')).toBe('alpha');
    expect(agentRowForKey(rows, 'beta', 'Home')).toBe('alpha');
    expect(agentRowForKey(rows, 'beta', 'End')).toBe('gamma');
  });

  it('别的键、不在表格里的行不动', () => {
    expect(agentRowForKey(rows, 'beta', 'Enter')).toBeNull();
    expect(agentRowForKey(rows, 'beta', 'toString')).toBeNull();
    expect(agentRowForKey(rows, 'nobody', 'ArrowDown')).toBeNull();
  });
});
