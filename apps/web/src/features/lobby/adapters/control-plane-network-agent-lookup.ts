import { z } from 'zod';

import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok } from '@/shared/result';
import {
  NETWORK_AGENT_LOOKUP_BATCH,
  type NetworkAgentLookupGateway,
} from '../domain/network-agent-labels';

const responseSchema = z
  .object({
    schemaVersion: z.literal(1),
    networkAgentIds: z.array(z.uuid()).max(NETWORK_AGENT_LOOKUP_BATCH),
  })
  .strict();

export class ControlPlaneNetworkAgentLookup implements NetworkAgentLookupGateway {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}

  readonly lookup: NetworkAgentLookupGateway['lookup'] = async (agentIds) => {
    const query = new URLSearchParams({ agentIds: agentIds.join(',') });
    try {
      const response = await this.options.fetch(
        controlPlaneEndpoint(this.options.baseUrl, `/network-agents/lookup?${query.toString()}`),
        { credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(8_000) },
      );
      if (!response.ok) return err({ code: 'unavailable' });
      const parsed = responseSchema.safeParse(await response.json());
      return parsed.success
        ? ok(new Set(parsed.data.networkAgentIds))
        : err({ code: 'invalid_response' });
    } catch {
      return err({ code: 'unavailable' });
    }
  };
}
