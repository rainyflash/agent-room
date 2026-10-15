import { z } from 'zod';

import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok } from '@/shared/result';
import {
  AGENT_OWNER_LOOKUP_BATCH,
  type AgentOwnerLookupGateway,
  type AgentOwnerRecord,
} from '../domain/agent-owners';

const personSchema = z.object({ displayName: z.string(), principalId: z.uuid() }).strict();

const responseSchema = z
  .object({
    agents: z
      .array(
        z.discriminatedUnion('kind', [
          z
            .object({ agentId: z.uuid(), kind: z.literal('local'), owner: personSchema.optional() })
            .strict(),
          z
            .object({
              admittedBy: personSchema.optional(),
              agentId: z.uuid(),
              kind: z.literal('network'),
            })
            .strict(),
        ]),
      )
      .max(AGENT_OWNER_LOOKUP_BATCH),
  })
  .strict();

/** 没有的字段不写出来（`owner`、`admittedBy` 在回答里是省略的）。 */
function toRecord(agent: z.infer<typeof responseSchema>['agents'][number]): AgentOwnerRecord {
  if (agent.kind === 'local') {
    return agent.owner === undefined
      ? { agentId: agent.agentId, kind: 'local' }
      : { agentId: agent.agentId, kind: 'local', owner: agent.owner };
  }
  return agent.admittedBy === undefined
    ? { agentId: agent.agentId, kind: 'network' }
    : { agentId: agent.agentId, kind: 'network', admittedBy: agent.admittedBy };
}

/** `GET /rooms/{c}/agent-owners?agentIds=…`，和网络 Agent 的查询一样逗号分隔、一次最多 100 个。 */
export class ControlPlaneAgentOwnerLookup implements AgentOwnerLookupGateway {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}

  readonly lookup: AgentOwnerLookupGateway['lookup'] = async (catalogId, agentIds) => {
    const query = new URLSearchParams({ agentIds: agentIds.join(',') });
    try {
      const response = await this.options.fetch(
        controlPlaneEndpoint(
          this.options.baseUrl,
          `/rooms/${encodeURIComponent(catalogId)}/agent-owners?${query.toString()}`,
        ),
        { credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(8_000) },
      );
      if (!response.ok) return err({ code: 'unavailable' });
      const parsed = responseSchema.safeParse(await response.json());
      return parsed.success
        ? ok(parsed.data.agents.map(toRecord))
        : err({ code: 'invalid_response' });
    } catch {
      return err({ code: 'unavailable' });
    }
  };
}
