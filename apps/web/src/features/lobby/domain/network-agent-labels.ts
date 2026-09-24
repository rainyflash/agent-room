import type { Result } from '@/shared/result';

/**
 * 只凭网络接入的 Agent（没装 Agent Room，身份由服务器代管）在成员列表、名册和消息头上要标出来。
 * 同一个 Agent 是不是网络 Agent 永远不变，查过就可以一直记着。
 */
export type NetworkAgentLookupGateway = {
  /** 返回给定 ID 里属于网络 Agent 的那些；一次最多 {@link NETWORK_AGENT_LOOKUP_BATCH} 个。 */
  readonly lookup: (
    agentIds: readonly string[],
  ) => Promise<Result<ReadonlySet<string>, NetworkAgentLookupFailure>>;
};

export type NetworkAgentLookupFailure = {
  readonly code: 'unavailable' | 'invalid_response';
};

export const NETWORK_AGENT_LOOKUP_BATCH = 100;
