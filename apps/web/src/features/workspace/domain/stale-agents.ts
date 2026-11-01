import { agentActivity, isNetworkInstance, liveInstances } from './agent-card';
import type { AgentInstanceActivity } from './agent-directory';
import type { FleetAgent } from './agent-fleet';

/** 连续这么久没上线，就算不用了（维护者 2026-11-01 定 30 天）。 */
export const STALE_AGENT_AFTER_MS = 30 * 24 * 60 * 60 * 1_000;

export type StaleAgent = {
  readonly entry: FleetAgent;
  /** 能看到的最后一次在线：设备上次用、连接上记的、在线状态里的，取最晚的。 */
  readonly lastOnlineAtUnixMs: number;
};

/**
 * 30 天没上线、可以一键删的 Agent，最久没上线的排前面。
 *
 * 宁可漏掉，不能删错：
 * - 正在线、正在连接、连接不稳的不算；
 * - 还在用的连接里有一个没问到设备上次用的时间，这个 Agent 就先不算（问不到不等于没用）；
 * - 网络 Agent 不算：它自己决定进出，一般也不在“我的 Agent”里；
 * - 每样时间都取最晚的那个，包括注册的时间，刚建的不会被当成不用了。
 */
export function staleAgents(input: {
  readonly agents: readonly FleetAgent[];
  /** 还没问到时是 `null`，这时一个也不给。 */
  readonly activity: AgentInstanceActivity | null;
  readonly canDelete: (entry: FleetAgent) => boolean;
  readonly now: number;
}): readonly StaleAgent[] {
  const { activity } = input;
  if (activity === null) return [];
  return input.agents
    .flatMap((entry): StaleAgent[] => {
      if (!input.canDelete(entry)) return [];
      const lastOnline = lastOnlineAt(entry, activity);
      return lastOnline !== null && input.now - lastOnline >= STALE_AGENT_AFTER_MS
        ? [{ entry, lastOnlineAtUnixMs: lastOnline }]
        : [];
    })
    .sort((left, right) => left.lastOnlineAtUnixMs - right.lastOnlineAtUnixMs);
}

/**
 * 不看设备时已经够 30 天的那些。设备上次用的时间只会让它更晚，所以这里一个都没有时，不用去问设备
 * （问一次服务器要替每个还在用的连接问一遍聊天服务器）。
 */
export function staleAgentCandidates(input: {
  readonly agents: readonly FleetAgent[];
  readonly canDelete: (entry: FleetAgent) => boolean;
  readonly now: number;
}): readonly StaleAgent[] {
  const unknown = new Map(
    input.agents.flatMap((entry) =>
      liveInstances(entry).map((instance) => [instance.agentInstanceId, null] as const),
    ),
  );
  return staleAgents({ ...input, activity: unknown });
}

function lastOnlineAt(entry: FleetAgent, activity: AgentInstanceActivity): number | null {
  const { kind } = agentActivity(entry);
  if (kind === 'online' || kind === 'connecting' || kind === 'degraded') return null;
  const live = liveInstances(entry);
  if (live.some(isNetworkInstance)) return null;
  const observed = [entry.agent.registeredAtUnixMs];
  if (entry.lastSeenAtUnixMs !== null) observed.push(entry.lastSeenAtUnixMs);
  for (const instance of entry.instances) {
    observed.push(instance.createdAtUnixMs);
    if (instance.lastSeenAtUnixMs !== null) observed.push(instance.lastSeenAtUnixMs);
  }
  for (const instance of live) {
    // 设备已经注销了的不用问，服务器也不会去问。
    if (instance.matrixDeviceRevokedAtUnixMs !== null) continue;
    if (!activity.has(instance.agentInstanceId)) return null;
    const seen = activity.get(instance.agentInstanceId);
    if (seen !== null && seen !== undefined) observed.push(seen);
  }
  return Math.max(...observed);
}
