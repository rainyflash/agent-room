import { z } from 'zod';

import {
  ownedAgentListSchema,
  type AgentDirectoryFailure,
  type AgentDirectoryGateway,
  type OwnedAgent,
} from '@/features/workspace/domain/agent-directory';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneAgentDirectoryClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneAgentDirectoryClient implements AgentDirectoryGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneAgentDirectoryClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async listOwnedAgents(): Promise<Result<readonly OwnedAgent[], AgentDirectoryFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, '/agents'), {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'GET',
        signal: controller.signal,
      });
      if (!response.ok) {
        return err(await readFailure(response));
      }
      const body: unknown = await response.json();
      const parsed = ownedAgentListSchema.safeParse(body);
      return parsed.success
        ? ok(parsed.data.agents)
        : err({ code: 'workspace.invalid_agent_directory', retryable: false });
    } catch (error) {
      return err({
        code: error instanceof SyntaxError ? 'workspace.invalid_json' : 'workspace.unreachable',
        retryable: !(error instanceof SyntaxError),
      });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<AgentDirectoryFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const responseCorrelationId = parsed.data.correlationId ?? correlationId;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(responseCorrelationId === undefined ? {} : { correlationId: responseCorrelationId }),
      };
    }
  } catch {
    // 无法解析的错误正文只降级为稳定 HTTP 边界，不向界面泄露原始响应。
  }
  return {
    code: `workspace.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
