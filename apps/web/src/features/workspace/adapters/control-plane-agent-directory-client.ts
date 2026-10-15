import { z } from 'zod';

import {
  agentRoomListSchema,
  broughtAgentSchema,
  leftAgentRoomSchema,
  ownedAgentListSchema,
  type AgentDirectoryFailure,
  type AgentDirectoryGateway,
  type AgentRoom,
  type OwnedAgent,
} from '@/features/workspace/domain/agent-directory';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
  retryAfterSeconds: z.number().int().nonnegative().optional(),
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

  async deleteAgent(agentId: string): Promise<Result<void, AgentDirectoryFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(
        controlPlaneEndpoint(this.#baseUrl, `/agents/${encodeURIComponent(agentId)}`),
        {
          cache: 'no-store',
          credentials: 'include',
          headers: { Accept: 'application/json' },
          method: 'DELETE',
          signal: controller.signal,
        },
      );
      return response.ok ? ok(undefined) : err(await readFailure(response));
    } catch {
      return err({ code: 'workspace.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }

  async listAgentRooms(
    agentId: string,
  ): Promise<Result<readonly AgentRoom[], AgentDirectoryFailure>> {
    return await this.#request(roomsPath(agentId), { method: 'GET' }, (body) => {
      const parsed = agentRoomListSchema.safeParse(body);
      return parsed.success ? parsed.data.rooms : null;
    });
  }

  async bringAgentIntoRoom(
    agentId: string,
    catalogId: string,
  ): Promise<Result<number, AgentDirectoryFailure>> {
    return await this.#request(
      roomsPath(agentId),
      {
        body: JSON.stringify({ catalogId }),
        headers: { 'Content-Type': 'application/json' },
        method: 'POST',
      },
      (body) => {
        const parsed = broughtAgentSchema.safeParse(body);
        return parsed.success ? parsed.data.entered : null;
      },
    );
  }

  async leaveAgentRoom(
    agentId: string,
    catalogId: string,
  ): Promise<Result<boolean, AgentDirectoryFailure>> {
    return await this.#request(
      `${roomsPath(agentId)}/${encodeURIComponent(catalogId)}`,
      { method: 'DELETE' },
      (body) => {
        const parsed = leftAgentRoomSchema.safeParse(body);
        return parsed.success ? parsed.data.left : null;
      },
    );
  }

  /** 发一个带登录的请求，按 `read` 读出回答；读不出来（返回 `null`）算回答损坏。 */
  async #request<T>(
    path: string,
    init: {
      readonly body?: string;
      readonly headers?: Record<string, string>;
      readonly method: string;
    },
    read: (body: unknown) => T | null,
  ): Promise<Result<T, AgentDirectoryFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, path), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json', ...init.headers },
        signal: controller.signal,
      });
      if (!response.ok) {
        return err(await readFailure(response));
      }
      const value = read(await response.json());
      return value === null
        ? err({ code: 'workspace.invalid_agent_rooms', retryable: false })
        : ok(value);
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

function roomsPath(agentId: string): string {
  return `/agents/${encodeURIComponent(agentId)}/rooms`;
}

async function readFailure(response: Response): Promise<AgentDirectoryFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  const headerRetryAfter = parseRetryAfter(response.headers.get('retry-after'));
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const responseCorrelationId = parsed.data.correlationId ?? correlationId;
      const retryAfterSeconds = parsed.data.retryAfterSeconds ?? headerRetryAfter;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(responseCorrelationId === undefined ? {} : { correlationId: responseCorrelationId }),
        ...(retryAfterSeconds === undefined ? {} : { retryAfterSeconds }),
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

function parseRetryAfter(value: string | null): number | undefined {
  if (value === null || !/^\d{1,9}$/u.test(value)) {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}
