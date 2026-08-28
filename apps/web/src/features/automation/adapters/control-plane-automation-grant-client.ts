import { z } from 'zod';

import {
  automationGrantListSchema,
  automationGrantSchema,
  createAutomationGrantInputSchema,
  type AutomationGrant,
  type AutomationGrantFailure,
  type AutomationGrantGateway,
  type CreateAutomationGrantInput,
} from '@/features/automation/domain/automation-grant';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneAutomationGrantClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneAutomationGrantClient implements AutomationGrantGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneAutomationGrantClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async list(): Promise<Result<readonly AutomationGrant[], AutomationGrantFailure>> {
    const response = await this.#request('/automation-grants', { method: 'GET' });
    if (!response.ok) {
      return response;
    }
    return parseProjection(response.value, automationGrantListSchema, (value) => value.grants);
  }

  async create(
    grantId: string,
    input: CreateAutomationGrantInput,
  ): Promise<Result<AutomationGrant, AutomationGrantFailure>> {
    const parsedInput = createAutomationGrantInputSchema.safeParse(input);
    if (!parsedInput.success) {
      return err({ code: 'automation.invalid_creation_input', retryable: false });
    }
    const response = await this.#request('/automation-grants', {
      body: JSON.stringify(parsedInput.data),
      headers: { 'Idempotency-Key': grantId },
      method: 'POST',
    });
    return response.ok
      ? parseProjection(response.value, automationGrantSchema, (value) => value)
      : response;
  }

  async revoke(grantId: string): Promise<Result<AutomationGrant, AutomationGrantFailure>> {
    const response = await this.#request(`/automation-grants/${encodeURIComponent(grantId)}`, {
      method: 'DELETE',
    });
    return response.ok
      ? parseProjection(response.value, automationGrantSchema, (value) => value)
      : response;
  }

  async #request(
    path: string,
    init: RequestInit,
  ): Promise<Result<Response, AutomationGrantFailure>> {
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
      return response.ok ? ok(response) : err(await readFailure(response));
    } catch {
      return err({ code: 'automation.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function parseProjection<TWire, TValue>(
  response: Response,
  schema: z.ZodType<TWire>,
  project: (value: TWire) => TValue,
): Promise<Result<TValue, AutomationGrantFailure>> {
  try {
    const body: unknown = await response.json();
    const parsed = schema.safeParse(body);
    return parsed.success
      ? ok(project(parsed.data))
      : err({ code: 'automation.invalid_response', retryable: false });
  } catch {
    return err({ code: 'automation.invalid_json', retryable: false });
  }
}

async function readFailure(response: Response): Promise<AutomationGrantFailure> {
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
    // 错误正文不可读时回退到稳定的 HTTP 错误边界。
  }
  return {
    code: `automation.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
