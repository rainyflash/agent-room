export type AgentStatusLeaseEvent<TStatus extends string = string> = {
  readonly createdAt: string;
  readonly leaseExpiresAt: string;
  readonly status: TStatus;
};

export type AgentStatusLeasePolicy = {
  readonly maximumLeaseMs: number;
  readonly allowedClockSkewMs: number;
};

export type EffectiveAgentStatus<TStatus extends string = string> = {
  readonly status: TStatus | 'offline';
  readonly effectiveExpiresAtUnixMs: number;
};

export type AgentStatusLeaseEvaluationError =
  | 'invalid_policy'
  | 'invalid_observation_time'
  | 'invalid_event_time'
  | 'invalid_lease_duration'
  | 'future_event';

export type AgentStatusLeaseEvaluation<TStatus extends string = string> =
  | { readonly ok: true; readonly value: EffectiveAgentStatus<TStatus> }
  | { readonly ok: false; readonly error: AgentStatusLeaseEvaluationError };

/**
 * 使用本地观测锚点计算工作状态，不依赖服务端发送离线清理事件。
 * 调用方必须先用协议 Schema 校验事件结构。
 */
export function evaluateAgentStatusLease<TStatus extends string>(
  event: AgentStatusLeaseEvent<TStatus>,
  observedAtUnixMs: number,
  nowUnixMs: number,
  policy: AgentStatusLeasePolicy,
): AgentStatusLeaseEvaluation<TStatus> {
  if (!validPolicy(policy)) {
    return { ok: false, error: 'invalid_policy' };
  }
  if (!isUnixMillis(observedAtUnixMs) || !isUnixMillis(nowUnixMs)) {
    return { ok: false, error: 'invalid_observation_time' };
  }

  const createdAtUnixMs = Date.parse(event.createdAt);
  const claimedExpiresAtUnixMs = Date.parse(event.leaseExpiresAt);
  if (!isUnixMillis(createdAtUnixMs) || !isUnixMillis(claimedExpiresAtUnixMs)) {
    return { ok: false, error: 'invalid_event_time' };
  }

  const claimedLifetime = claimedExpiresAtUnixMs - createdAtUnixMs;
  if (claimedLifetime <= 0 || claimedLifetime > policy.maximumLeaseMs) {
    return { ok: false, error: 'invalid_lease_duration' };
  }
  if (createdAtUnixMs > observedAtUnixMs + policy.allowedClockSkewMs) {
    return { ok: false, error: 'future_event' };
  }

  const senderDeadline = claimedExpiresAtUnixMs + policy.allowedClockSkewMs;
  const localDeadline = observedAtUnixMs + policy.maximumLeaseMs + policy.allowedClockSkewMs;
  if (!Number.isSafeInteger(senderDeadline) || !Number.isSafeInteger(localDeadline)) {
    return { ok: false, error: 'invalid_event_time' };
  }
  const effectiveExpiresAtUnixMs = Math.min(senderDeadline, localDeadline);
  return {
    ok: true,
    value: {
      status:
        event.status === 'offline' || nowUnixMs >= effectiveExpiresAtUnixMs
          ? 'offline'
          : event.status,
      effectiveExpiresAtUnixMs,
    },
  };
}

function validPolicy(policy: AgentStatusLeasePolicy): boolean {
  return (
    Number.isSafeInteger(policy.maximumLeaseMs) &&
    policy.maximumLeaseMs > 0 &&
    Number.isSafeInteger(policy.allowedClockSkewMs) &&
    policy.allowedClockSkewMs >= 0 &&
    policy.allowedClockSkewMs < policy.maximumLeaseMs
  );
}

function isUnixMillis(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}
