import type { LobbyAgent } from './lobby';
import {
  agentLifecyclePolicy,
  evaluateAgentLifecycle,
  type AgentPresenceEvidence,
} from '@agent-room/protocol';

export const reconnectGraceMs = agentLifecyclePolicy.reconnectGraceMs;
export type AgentAttendance = 'present' | 'reconnecting' | 'away';
export type AgentReception = 'recent' | 'waiting' | 'unknown' | 'reconnecting' | 'away';

export function presenceEvidence(agent: LobbyAgent): AgentPresenceEvidence {
  return {
    agentId: agent.agentId,
    reportedStatus: agent.reportedStatus ?? agent.status,
    leaseExpiresAtUnixMs: agent.statusExpiresAtUnixMs,
    lastActiveAtUnixMs: agent.lastActiveAtUnixMs ?? Math.max(0, agent.statusExpiresAtUnixMs),
    ...(agent.lastPolledAtUnixMs === undefined
      ? {}
      : { lastPolledAtUnixMs: agent.lastPolledAtUnixMs }),
    ...(agent.listeningUntilUnixMs === undefined
      ? {}
      : { listeningUntilUnixMs: agent.listeningUntilUnixMs }),
    ...(agent.liveness === undefined ? {} : { liveness: agent.liveness }),
    ...(agent.presence === undefined ? {} : { presence: agent.presence }),
  };
}

export function agentLifecycle(agent: LobbyAgent, now: number) {
  return agent.lifecycle ?? evaluateAgentLifecycle(presenceEvidence(agent), now);
}

/**
 * 详情里的“上次连接”。租约写法每两分钟续一次，最近一条就是上次连接；名片是进房间时写的，
 * 不是上次连接，所以在线时不说，离线时说它离线的那一刻。
 */
export function lastConnectedAt(agent: LobbyAgent, now: number): number | null {
  if (agent.liveness !== 'presence') return agent.lastActiveAtUnixMs ?? null;
  const state = agentLifecycle(agent, now);
  return state.connection === 'offline' ? state.offlineSinceUnixMs : null;
}

/** 名单上一个 Agent 的状态：在线时是在不在等消息，不在线时是重连中或离线。 */
export type AgentStateKey = 'waiting' | 'on_resume' | 'unknown' | 'reconnecting' | 'offline';
export const agentStateKeys: readonly AgentStateKey[] = [
  'waiting',
  'on_resume',
  'unknown',
  'reconnecting',
  'offline',
];
export function agentStateKey(agent: LobbyAgent, now: number): AgentStateKey {
  const state = agentLifecycle(agent, now);
  return state.connection === 'online' ? state.reception : state.connection;
}

export type AgentRosterGroup =
  | 'waiting'
  | 'on_resume'
  | 'unknown'
  | 'reconnecting'
  | 'offline_hour'
  | 'offline_today'
  | 'offline_week'
  | 'offline_older';
export const agentRosterGroups: readonly AgentRosterGroup[] = [
  'waiting',
  'on_resume',
  'unknown',
  'reconnecting',
  'offline_hour',
  'offline_today',
  'offline_week',
  'offline_older',
];
export function agentRosterGroup(agent: LobbyAgent, now: number): AgentRosterGroup {
  const state = agentLifecycle(agent, now);
  if (state.connection === 'reconnecting') return 'reconnecting';
  if (state.connection === 'online')
    return state.reception === 'waiting'
      ? 'waiting'
      : state.reception === 'on_resume'
        ? 'on_resume'
        : 'unknown';
  const elapsed = now - (state.offlineSinceUnixMs ?? now);
  return elapsed < 3_600_000
    ? 'offline_hour'
    : elapsed < 86_400_000
      ? 'offline_today'
      : elapsed < 7 * 86_400_000
        ? 'offline_week'
        : 'offline_older';
}

/** A task result, a transport lease and an inbox read are different evidence. */
export function agentAttendance(agent: LobbyAgent, now: number): AgentAttendance {
  const connection = agentLifecycle(agent, now).connection;
  return connection === 'online'
    ? 'present'
    : connection === 'reconnecting'
      ? 'reconnecting'
      : 'away';
}

export function agentReception(agent: LobbyAgent, now: number): AgentReception {
  const attendance = agentAttendance(agent, now);
  if (attendance !== 'present') return attendance;
  const reception = agentLifecycle(agent, now).reception;
  return reception === 'waiting' ? 'recent' : reception === 'unknown' ? 'unknown' : 'waiting';
}

export function attendanceCounts(agents: readonly LobbyAgent[], now: number) {
  return agents.reduce(
    (counts, agent) => {
      if (agentLifecycle(agent, now).archived) return counts;
      counts[agentAttendance(agent, now)] += 1;
      return counts;
    },
    { present: 0, reconnecting: 0, away: 0 },
  );
}
