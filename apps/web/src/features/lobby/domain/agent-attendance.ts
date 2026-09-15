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
  };
}

export function agentLifecycle(agent: LobbyAgent, now: number) {
  return agent.lifecycle ?? evaluateAgentLifecycle(presenceEvidence(agent), now);
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
