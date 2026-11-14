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
