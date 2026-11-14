import { agentLifecycle, agentStateKey } from './agent-attendance';
import type { LobbyAgent } from './lobby';

/**
 * 表格里一行的状态：名单上的五种，再加“叫得醒”（`specs/redesign/room-page.md` 的“Agent 表格”）。
 * 顺序就是表格从上到下排的顺序。
 */
export const agentRowStates = [
  'waiting',
  'wakes',
  'on_resume',
  'unknown',
  'reconnecting',
  'offline',
] as const;
export type AgentRowState = (typeof agentRowStates)[number];

export type AgentRow = {
  readonly agent: LobbyAgent;
  readonly state: AgentRowState;
  /** 离线的：从什么时候起。别的状态没有。 */
  readonly offlineSinceUnixMs: number | null;
  /** 离线太久、收进归档的。平时不列，点“已归档”才看。 */
  readonly archived: boolean;
};

/**
 * 叫得醒：它自己没在等消息（连着没在等、重连中、离线都算），但有电脑开着后台回复替它收着，
 * @ 它会回。正在等消息的照旧算在等。
 */
export function agentRowState(agent: LobbyAgent, now: number, wakeable: boolean): AgentRowState {
  const key = agentStateKey(agent, now);
  return key !== 'waiting' && wakeable ? 'wakes' : key;
}

/** 排好的行：在等的在前，然后叫得醒、下次运行时读、在线、重连中；离线的最近离线的在前。 */
export function agentRows(
  agents: readonly LobbyAgent[],
  now: number,
  wakeableAgentIds: ReadonlySet<string>,
): readonly AgentRow[] {
  return agents
    .map((agent): AgentRow => {
      const lifecycle = agentLifecycle(agent, now);
      return {
        agent,
        state: agentRowState(agent, now, wakeableAgentIds.has(agent.agentId)),
        offlineSinceUnixMs: lifecycle.offlineSinceUnixMs,
        archived: lifecycle.archived,
      };
    })
    .toSorted(
      (a, b) =>
        agentRowStates.indexOf(a.state) - agentRowStates.indexOf(b.state) ||
        // 名片的时间是进房间那一刻，不能拿来排；离线的按离线那一刻排。
        (b.offlineSinceUnixMs ?? 0) - (a.offlineSinceUnixMs ?? 0) ||
        a.agent.displayName.localeCompare(b.agent.displayName),
    );
}

/** 表格上面那一行汇总：几个在等消息、几个叫得醒、几个离线。归档的不算。 */
export function agentRowSummary(rows: readonly AgentRow[]): {
  readonly waiting: number;
  readonly wakes: number;
  readonly offline: number;
} {
  const count = (state: AgentRowState): number =>
    rows.filter((row) => !row.archived && row.state === state).length;
  return { waiting: count('waiting'), wakes: count('wakes'), offline: count('offline') };
}

/** Agent 名字后面的四位编号（`specs/agent-name-tag/design.md`），同名的靠它分开。 */
const nameTag = / #\d{4}$/u;

/**
 * 把名字拆成前面的名字和后面的编号：地方不够时截前面、留着编号，不然同名的又分不开了。
 * 编号那一段带着前面的空格，两段接起来就是原来的名字。没有编号的整个算名字。
 */
export function agentNameParts(displayName: string): {
  readonly base: string;
  readonly tag: string;
} {
  const tag = nameTag.exec(displayName)?.[0] ?? '';
  return { base: tag === '' ? displayName : displayName.slice(0, -tag.length), tag };
}

/** 按名字或 Matrix ID 找，不分大小写。 */
export function searchAgentRows(rows: readonly AgentRow[], query: string): readonly AgentRow[] {
  const needle = query.trim().toLocaleLowerCase();
  if (needle === '') return rows;
  return rows.filter(
    ({ agent }) =>
      agent.displayName.toLocaleLowerCase().includes(needle) ||
      agent.matrixUserId.toLocaleLowerCase().includes(needle),
  );
}

/**
 * 整页表格上按状态筛的几项。“全部”不算归档的；“已归档”只看归档的，别的几项都不看归档的。
 * 只挑了人会去找的三种状态，下次运行时读、在线、重连中在“全部”里看。
 */
export const agentStateFilters = ['all', 'waiting', 'wakes', 'offline', 'archived'] as const;
export type AgentStateFilter = (typeof agentStateFilters)[number];

/** 按“谁的”筛：所有人的、你自己的、网络 Agent。 */
export const agentOwnerFilters = ['all', 'mine', 'network'] as const;
export type AgentOwnerFilter = (typeof agentOwnerFilters)[number];

/** 按你自己做的记号筛：不筛、你收藏的、带某个标签的。 */
export type AgentMarkFilter =
  | { readonly kind: 'all' }
  | { readonly kind: 'favorites' }
  | { readonly kind: 'tag'; readonly tag: string };

export type AgentRowFilter = {
  readonly state: AgentStateFilter;
  readonly owner: AgentOwnerFilter;
  readonly mark: AgentMarkFilter;
};

export const noAgentRowFilter: AgentRowFilter = {
  state: 'all',
  owner: 'all',
  mark: { kind: 'all' },
};

/** 筛的时候要问的几件事，都不在 `LobbyAgent` 上：谁的由服务器说，收藏和标签是你自己记的。 */
export type AgentRowFacts = {
  readonly mine: (agentId: string) => boolean;
  readonly network: (agentId: string) => boolean;
  readonly favorite: (agentId: string) => boolean;
  readonly tags: (agentId: string) => readonly string[];
};

export function filterAgentRows(
  rows: readonly AgentRow[],
  filter: AgentRowFilter,
  facts: AgentRowFacts,
): readonly AgentRow[] {
  return rows.filter(({ agent, archived, state }) => {
    if (archived !== (filter.state === 'archived')) return false;
    if (filter.state !== 'all' && filter.state !== 'archived' && state !== filter.state)
      return false;
    if (filter.owner === 'mine' && !facts.mine(agent.agentId)) return false;
    if (filter.owner === 'network' && !facts.network(agent.agentId)) return false;
    if (filter.mark.kind === 'favorites') return facts.favorite(agent.agentId);
    if (filter.mark.kind === 'tag') return facts.tags(agent.agentId).includes(filter.mark.tag);
    return true;
  });
}

/**
 * 怎么来的。网络 Agent 是服务器说的；“这台电脑”要这台电脑上的连接服务说它在这里（只有桌面端问得到）。
 * 别的只知道它是从某台电脑上接进来的：说不出“别的电脑”，这台电脑上已经关掉的会话读不到。
 */
export const agentOrigins = ['this_computer', 'computer', 'network'] as const;
export type AgentOrigin = (typeof agentOrigins)[number];

export function agentOrigin(
  agentId: string,
  network: boolean,
  localAgentIds: ReadonlySet<string> | null,
): AgentOrigin {
  if (network) return 'network';
  return localAgentIds?.has(agentId) === true ? 'this_computer' : 'computer';
}

/**
 * 进来的时间。写名片的 Agent 只在进房间时写一次，名片的时间就是它（改名、离开又回来会重写）；
 * 旧的租约写法隔两分钟续一次，那个时间不是进来的时间，不知道就不写。
 */
export function agentJoinedAtUnixMs(agent: LobbyAgent): number | null {
  return agent.liveness === 'presence' ? (agent.lastActiveAtUnixMs ?? null) : null;
}

/** 表格里上下键、Home、End 走到哪一行；别的键、找不到这一行时为空。 */
export function agentRowForKey(
  rows: readonly AgentRow[],
  currentAgentId: string,
  key: string,
): string | null {
  const index = rows.findIndex((row) => row.agent.agentId === currentAgentId);
  if (index < 0) return null;
  const targets: Readonly<Record<string, number>> = {
    ArrowDown: Math.min(rows.length - 1, index + 1),
    ArrowUp: Math.max(0, index - 1),
    End: rows.length - 1,
    Home: 0,
  };
  return Object.hasOwn(targets, key) ? (rows[targets[key] ?? -1]?.agent.agentId ?? null) : null;
}
