import type { Result } from '@/shared/result';

/**
 * 房间里的 Agent 是谁的（`specs/room-people/design.md`“Agent 是谁的”）：服务器按自己的记录回答，
 * Agent 自己写的名片说了不算。私人房间只有已加入的成员问得到，公共大厅登录了的人都问得到。
 */
export type AgentOwnerPerson = {
  readonly principalId: string;
  readonly displayName: string;
};

export type AgentOwner =
  | {
      /** 本机 Agent：主人是它最早的那位还有效的所有者；查不到时没有。 */
      readonly kind: 'local';
      readonly owner?: AgentOwnerPerson;
    }
  | {
      /** 网络 Agent 没有人类主人；私人房间里经敲门放行的，写放行的人。 */
      readonly kind: 'network';
      readonly admittedBy?: AgentOwnerPerson;
    };

export type AgentOwnerRecord = AgentOwner & { readonly agentId: string };

export type AgentOwnerLookupFailure = {
  readonly code: 'unavailable' | 'invalid_response';
};

export type AgentOwnerLookupGateway = {
  /**
   * 问一个房间里这些 Agent 是谁的；一次最多 {@link AGENT_OWNER_LOOKUP_BATCH} 个。
   * 此刻不在这个房间里的不回答。
   */
  readonly lookup: (
    catalogId: string,
    agentIds: readonly string[],
  ) => Promise<Result<readonly AgentOwnerRecord[], AgentOwnerLookupFailure>>;
};

export const AGENT_OWNER_LOOKUP_BATCH = 100;
