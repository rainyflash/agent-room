import { z } from 'zod';

import {
  directContactSchema,
  directSessionListSchema,
  directSessionSchema,
  type DirectContact,
  type DirectSession,
  type DirectSessionFailure,
  type DirectSessionGateway,
} from '@/features/direct-sessions/domain/direct-session';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneDirectSessionClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneDirectSessionClient implements DirectSessionGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneDirectSessionClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async list(): Promise<Result<readonly DirectSession[], DirectSessionFailure>> {
    const response = await this.#request(
      '/direct-sessions',
      { method: 'GET' },
      directSessionListSchema,
    );
    return response.ok ? ok(response.value.sessions) : response;
  }

  inspect(catalogId: string): Promise<Result<DirectSession, DirectSessionFailure>> {
    return this.#request(
      `/direct-sessions/${encodeURIComponent(catalogId)}`,
      { method: 'GET' },
      directSessionSchema,
    );
  }

  open(targetAgentId: string): Promise<Result<DirectSession, DirectSessionFailure>> {
    return this.#request(
      '/direct-sessions',
      { body: JSON.stringify({ targetAgentId }), method: 'POST' },
      directSessionSchema,
    );
  }

  setBlocked(
    targetAgentId: string,
    blocked: boolean,
  ): Promise<Result<DirectContact, DirectSessionFailure>> {
    return this.#request(
      `/direct-contacts/${encodeURIComponent(targetAgentId)}/block`,
      { body: JSON.stringify({ blocked }), method: 'PUT' },
      directContactSchema,
    );
  }

  async #request<T>(
    path: string,
    init: RequestInit,
    schema: z.ZodType<T>,
  ): Promise<Result<T, DirectSessionFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const headers = new Headers(init.headers);
      headers.set('Accept', 'application/json');
      if (init.body !== undefined) {
        headers.set('Content-Type', 'application/json');
      }
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, path), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers,
        signal: controller.signal,
      });
      if (!response.ok) {
        return err(await readFailure(response));
      }
      const body: unknown = await response.json();
      const parsed = schema.safeParse(body);
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'direct_session.invalid_response', retryable: false });
    } catch {
      return err({ code: 'direct_session.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<DirectSessionFailure> {
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
    // 错误正文不可用时保留确定性的 HTTP 失败边界。
  }
  return {
    code: `direct_session.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
