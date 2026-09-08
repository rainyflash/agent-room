import type { LobbyAgent } from './lobby';

export const reconnectGraceMs = 30_000;
export const receptionFreshnessMs = 35_000;
export type AgentAttendance = 'present' | 'reconnecting' | 'away';
export type AgentReception = 'recent' | 'waiting' | 'unknown' | 'reconnecting' | 'away';

/** A task result, a transport lease and an inbox read are different evidence. */
export function agentAttendance(agent: LobbyAgent, now: number): AgentAttendance {
  if (agent.status !== 'offline' && now < agent.statusExpiresAtUnixMs) return 'present';
  if (
    (agent.reportedStatus ?? agent.status) !== 'offline' &&
    now >= agent.statusExpiresAtUnixMs &&
    now < agent.statusExpiresAtUnixMs + reconnectGraceMs
  )
    return 'reconnecting';
  return 'away';
}

export function agentReception(agent: LobbyAgent, now: number): AgentReception {
  const attendance = agentAttendance(agent, now);
  if (attendance !== 'present') return attendance;
  if (agent.lastPolledAtUnixMs === undefined) return 'unknown';
  return agent.lastPolledAtUnixMs <= now && now - agent.lastPolledAtUnixMs < receptionFreshnessMs
    ? 'recent'
    : 'waiting';
}

export function attendanceCounts(agents: readonly LobbyAgent[], now: number) {
  return agents.reduce(
    (counts, agent) => {
      counts[agentAttendance(agent, now)] += 1;
      return counts;
    },
    { present: 0, reconnecting: 0, away: 0 },
  );
}
