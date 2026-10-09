/** Room-wide roster rules. See fixtures/agent-lifecycle.json for cross-client conformance. */
export const agentLifecyclePolicy = Object.freeze({
  reconnectGraceMs: 30_000,
  receptionFreshnessMs: 15_000,
  /** The latest `waitingUntil` may be after the event's `createdAt`. Outlives a lease renewal. */
  waitingLeaseMs: 180_000,
  archiveAfterDays: 7,
  recentOfflineLimit: 100,
  pageSize: 100,
});

export type AgentConnection = 'online' | 'reconnecting' | 'offline';
export type AgentReceptionState = 'waiting' | 'on_resume' | 'unknown' | 'unavailable';
export type AgentArchiveReason = 'expired' | 'capacity';
/** Matrix 自带的在线状态（presence），按用户算。 */
export type MatrixPresenceState = 'online' | 'unavailable' | 'offline';
/** 读的一边对一个 Agent 的 Matrix 在线状态知道多少。 */
export type MatrixPresenceObservation = {
  readonly state: MatrixPresenceState;
  /** 亲眼看到它从在线或离开变成离线的时刻。 */
  readonly offlineSeenAtUnixMs?: number;
  /** 在线状态里的“上次活动”：拿到时的时刻减去 `last_active_ago`。 */
  readonly lastActiveAtUnixMs?: number;
};
export type AgentPresenceEvidence = {
  readonly agentId: string;
  readonly reportedStatus: string;
  readonly leaseExpiresAtUnixMs: number;
  /** 状态事件的时间；名片就是写名片的时间。 */
  readonly lastActiveAtUnixMs: number;
  readonly lastPolledAtUnixMs?: number;
  readonly listeningUntilUnixMs?: number | null;
  /**
   * `presence`：状态事件只是名片，在不在线、在不在等消息都看 Matrix 的在线状态。
   * 不写就是旧的租约写法。
   */
  readonly liveness?: 'presence';
  /** 名片对应的 Matrix 在线状态；还没拿到就不写。 */
  readonly presence?: MatrixPresenceObservation;
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
  if (evidence.liveness === 'presence') {
    // 名片不看工作状态和租约，也没有“重连中”：Synapse 自己已经等了约 30 秒才说离线。
    const state = evidence.presence?.state;
    return state === 'online' || state === 'unavailable' ? 'online' : 'offline';
  }
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
  const offlineSinceUnixMs = connection === 'offline' ? offlineSince(evidence, now) : null;
  const archived =
    offlineSinceUnixMs !== null && now - offlineSinceUnixMs >= archiveAfterDays * 86_400_000;
  const listeningUntil = evidence.listeningUntilUnixMs;
  const shared = {
    offlineSinceUnixMs,
    archived,
    archiveReason: archived ? ('expired' as const) : null,
  };
  if (connection !== 'online') return { ...shared, connection, reception: 'unavailable' };
  if (evidence.liveness === 'presence') {
    return {
      ...shared,
      connection,
      reception: evidence.presence?.state === 'online' ? 'waiting' : 'on_resume',
    };
  }
  return {
    ...shared,
    connection,
    reception:
      listeningUntil !== undefined && listeningUntil !== null && now < listeningUntil
        ? 'waiting'
        : listeningUntil === undefined
          ? 'unknown'
          : 'on_resume',
  };
}

function offlineSince(evidence: AgentPresenceEvidence, now: number): number {
  if (evidence.liveness === 'presence') {
    // 看到它变离线的，从那一刻算；没看到就用在线状态里的上次活动，但不早于名片本身
    // （写名片时它一定在）。都不晚于现在。
    const presence = evidence.presence;
    const since =
      presence?.offlineSeenAtUnixMs ??
      Math.max(
        presence?.lastActiveAtUnixMs ?? evidence.lastActiveAtUnixMs,
        evidence.lastActiveAtUnixMs,
      );
    return Math.min(since, now);
  }
  return evidence.reportedStatus === 'offline'
    ? evidence.lastActiveAtUnixMs
    : evidence.leaseExpiresAtUnixMs + agentLifecyclePolicy.reconnectGraceMs;
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
  // 最近离线的名额按最后一次见到它排：租约写法是最后一条状态事件的时间；名片是进房间时写的，
  // 按它离线的那一刻排。
  const lastSeen = (agent: AgentPresenceEvidence): number =>
    agent.liveness === 'presence'
      ? (result.get(agent.agentId)?.offlineSinceUnixMs ?? agent.lastActiveAtUnixMs)
      : agent.lastActiveAtUnixMs;
  const offline = agents
    .filter((agent) => {
      const state = result.get(agent.agentId);
      return state?.connection === 'offline' && !state.archived;
    })
    .toSorted((a, b) => lastSeen(b) - lastSeen(a) || a.agentId.localeCompare(b.agentId));
  for (const agent of offline.slice(agentLifecyclePolicy.recentOfflineLimit)) {
    const state = result.get(agent.agentId);
    if (state) result.set(agent.agentId, { ...state, archived: true, archiveReason: 'capacity' });
  }
  return result;
}
