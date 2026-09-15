/** Room-wide roster rules. See fixtures/agent-lifecycle.json for cross-client conformance. */
export const agentLifecyclePolicy = Object.freeze({
  reconnectGraceMs: 30_000,
  receptionFreshnessMs: 15_000,
  archiveAfterDays: 7,
  recentOfflineLimit: 100,
  pageSize: 100,
});

export type AgentConnection = 'online' | 'reconnecting' | 'offline';
export type AgentReceptionState = 'waiting' | 'on_resume' | 'unknown' | 'unavailable';
export type AgentArchiveReason = 'expired' | 'capacity';
export type AgentPresenceEvidence = {
  readonly agentId: string;
  readonly reportedStatus: string;
  readonly leaseExpiresAtUnixMs: number;
  readonly lastActiveAtUnixMs: number;
  readonly lastPolledAtUnixMs?: number;
  readonly listeningUntilUnixMs?: number | null;
};
export type AgentLifecycle = {
  readonly offlineSinceUnixMs: number | null;
  readonly archived: boolean;
  readonly archiveReason: AgentArchiveReason | null;
} & (
  | {
      readonly connection: 'online';
      readonly reception: Exclude<AgentReceptionState, 'unavailable'>;
    }
  | { readonly connection: 'reconnecting' | 'offline'; readonly reception: 'unavailable' }
);

export function agentConnection(evidence: AgentPresenceEvidence, now: number): AgentConnection {
  if (evidence.reportedStatus === 'offline') return 'offline';
  if (now < evidence.leaseExpiresAtUnixMs) return 'online';
  return now < evidence.leaseExpiresAtUnixMs + agentLifecyclePolicy.reconnectGraceMs
    ? 'reconnecting'
    : 'offline';
}

export function evaluateAgentLifecycle(
  evidence: AgentPresenceEvidence,
  now: number,
  archiveAfterDays: number = agentLifecyclePolicy.archiveAfterDays,
): AgentLifecycle {
  const connection = agentConnection(evidence, now);
  const offlineSinceUnixMs =
    connection === 'offline'
      ? evidence.reportedStatus === 'offline'
        ? evidence.lastActiveAtUnixMs
        : evidence.leaseExpiresAtUnixMs + agentLifecyclePolicy.reconnectGraceMs
      : null;
  const archived =
    offlineSinceUnixMs !== null && now - offlineSinceUnixMs >= archiveAfterDays * 86_400_000;
  const listeningUntil = evidence.listeningUntilUnixMs;
  const shared = {
    offlineSinceUnixMs,
    archived,
    archiveReason: archived ? ('expired' as const) : null,
  };
  return connection === 'online'
    ? {
        ...shared,
        connection,
        reception:
          listeningUntil !== undefined && listeningUntil !== null && now < listeningUntil
            ? 'waiting'
            : listeningUntil === undefined
              ? 'unknown'
              : 'on_resume',
      }
    : { ...shared, connection, reception: 'unavailable' };
}

/** Apply capacity before searching or paging so clients cannot disagree by filter. */
export function projectAgentLifecycles(
  agents: readonly AgentPresenceEvidence[],
  now: number,
  archiveAfterDays: number = agentLifecyclePolicy.archiveAfterDays,
): ReadonlyMap<string, AgentLifecycle> {
  const result = new Map(
    agents.map((agent) => [agent.agentId, evaluateAgentLifecycle(agent, now, archiveAfterDays)]),
  );
  const offline = agents
    .filter((agent) => {
      const state = result.get(agent.agentId);
      return state?.connection === 'offline' && !state.archived;
    })
    .toSorted(
      (a, b) => b.lastActiveAtUnixMs - a.lastActiveAtUnixMs || a.agentId.localeCompare(b.agentId),
    );
  for (const agent of offline.slice(agentLifecyclePolicy.recentOfflineLimit)) {
    const state = result.get(agent.agentId);
    if (state) result.set(agent.agentId, { ...state, archived: true, archiveReason: 'capacity' });
  }
  return result;
}
