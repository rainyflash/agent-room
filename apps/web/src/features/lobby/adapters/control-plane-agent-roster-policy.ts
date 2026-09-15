import {
  agentRosterPolicySchema,
  type AgentRosterPolicy,
  type AgentRosterPolicyFailure,
  type AgentRosterPolicyGateway,
} from '../domain/agent-roster-policy';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

export class ControlPlaneAgentRosterPolicy implements AgentRosterPolicyGateway {
  constructor(
    private readonly baseUrl: string,
    private readonly request: typeof fetch = globalThis.fetch.bind(globalThis),
  ) {}
  async update(
    catalogId: string,
    policy: AgentRosterPolicy,
  ): Promise<Result<AgentRosterPolicy, AgentRosterPolicyFailure>> {
    if (!agentRosterPolicySchema.safeParse(policy).success)
      return err({ code: 'invalid_response' });
    try {
      const response = await this.request(
        controlPlaneEndpoint(
          this.baseUrl,
          `/rooms/${encodeURIComponent(catalogId)}/agent-roster-policy`,
        ),
        {
          method: 'PUT',
          credentials: 'include',
          cache: 'no-store',
          signal: AbortSignal.timeout(8_000),
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ archiveAfterDays: policy.archiveAfterDays }),
        },
      );
      if (!response.ok) return err({ code: response.status === 403 ? 'forbidden' : 'unavailable' });
      const parsed = agentRosterPolicySchema.safeParse(await response.json());
      return parsed.success ? ok(parsed.data) : err({ code: 'invalid_response' });
    } catch {
      return err({ code: 'unavailable' });
    }
  }
}
