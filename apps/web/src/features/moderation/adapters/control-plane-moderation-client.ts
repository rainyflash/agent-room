import { z } from 'zod';

import {
  applyModerationActionInputSchema,
  moderationActionListSchema,
  moderationActionSchema,
  moderationAuditListSchema,
  moderationCaseListSchema,
  moderationCaseSchema,
  submitModerationReportInputSchema,
  type ApplyModerationActionInput,
  type ModerationAction,
  type ModerationAuditEvent,
  type ModerationCase,
  type ModerationFailure,
  type ModerationGateway,
  type SubmitModerationReportInput,
} from '@/features/moderation/domain/moderation';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryAfterSeconds: z.number().int().nonnegative().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneModerationClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneModerationClient implements ModerationGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneModerationClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async report(
    caseId: string,
    input: SubmitModerationReportInput,
  ): Promise<Result<ModerationCase, ModerationFailure>> {
    const parsed = submitModerationReportInputSchema.safeParse(input);
    if (!parsed.success) {
      return err({ code: 'moderation.invalid_report_input', retryable: false });
    }
    return await this.#jsonRequest(
      '/moderation/cases',
      {
        body: JSON.stringify(parsed.data),
        headers: { 'Idempotency-Key': caseId },
        method: 'POST',
      },
      moderationCaseSchema,
    );
  }

  async listCases(): Promise<Result<readonly ModerationCase[], ModerationFailure>> {
    const response = await this.#jsonRequest(
      '/moderation/cases',
      { method: 'GET' },
      moderationCaseListSchema,
    );
    return response.ok ? ok(response.value.cases) : response;
  }

  async listRoomCases(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationCase[], ModerationFailure>> {
    const response = await this.#jsonRequest(
      `/rooms/${encodeURIComponent(roomCatalogId)}/moderation/cases`,
      { method: 'GET' },
      moderationCaseListSchema,
    );
    return response.ok ? ok(response.value.cases) : response;
  }

  async listActions(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationAction[], ModerationFailure>> {
    const response = await this.#jsonRequest(
      `/rooms/${encodeURIComponent(roomCatalogId)}/moderation/actions`,
      { method: 'GET' },
      moderationActionListSchema,
    );
    return response.ok ? ok(response.value.actions) : response;
  }

  async applyAction(
    actionId: string,
    roomCatalogId: string,
    input: ApplyModerationActionInput,
  ): Promise<Result<ModerationAction, ModerationFailure>> {
    const parsed = applyModerationActionInputSchema.safeParse(input);
    if (!parsed.success) {
      return err({ code: 'moderation.invalid_action_input', retryable: false });
    }
    return await this.#jsonRequest(
      `/rooms/${encodeURIComponent(roomCatalogId)}/moderation/actions`,
      {
        body: JSON.stringify(parsed.data),
        headers: { 'Idempotency-Key': actionId },
        method: 'POST',
      },
      moderationActionSchema,
    );
  }

  reverseAction(actionId: string): Promise<Result<ModerationAction, ModerationFailure>> {
    return this.#jsonRequest(
      `/moderation/actions/${encodeURIComponent(actionId)}`,
      { body: JSON.stringify({ impactAcknowledged: true }), method: 'DELETE' },
      moderationActionSchema,
    );
  }

  async listAudit(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationAuditEvent[], ModerationFailure>> {
    const query = new URLSearchParams({ limit: '100', roomCatalogId });
    const response = await this.#jsonRequest(
      `/moderation/audit?${query.toString()}`,
      { method: 'GET' },
      moderationAuditListSchema,
    );
    return response.ok ? ok(response.value.events) : response;
  }

  async #jsonRequest<T>(
    path: string,
    init: RequestInit,
    schema: z.ZodType<T>,
  ): Promise<Result<T, ModerationFailure>> {
    const response = await this.#request(path, init);
    if (!response.ok) {
      return response;
    }
    try {
      const body: unknown = await response.value.json();
      const parsed = schema.safeParse(body);
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'moderation.invalid_response', retryable: false });
    } catch {
      return err({ code: 'moderation.invalid_json', retryable: false });
    }
  }

  async #request(path: string, init: RequestInit): Promise<Result<Response, ModerationFailure>> {
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
      const response = await this.#fetch(new URL(path, this.#baseUrl), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers,
        signal: controller.signal,
      });
      return response.ok ? ok(response) : err(await readFailure(response));
    } catch {
      return err({ code: 'moderation.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<ModerationFailure> {
  const headerCorrelationId = response.headers.get('x-correlation-id') ?? undefined;
  const headerRetryAfter = parseRetryAfter(response.headers.get('retry-after'));
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const correlationId = parsed.data.correlationId ?? headerCorrelationId;
      const retryAfterSeconds = parsed.data.retryAfterSeconds ?? headerRetryAfter;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(correlationId === undefined ? {} : { correlationId }),
        ...(retryAfterSeconds === undefined ? {} : { retryAfterSeconds }),
      };
    }
  } catch {
    // 不可信错误正文不可读时回退到 HTTP 状态和白名单响应头。
  }
  return {
    code: `moderation.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(headerCorrelationId === undefined ? {} : { correlationId: headerCorrelationId }),
    ...(headerRetryAfter === undefined ? {} : { retryAfterSeconds: headerRetryAfter }),
  };
}

function parseRetryAfter(value: string | null): number | undefined {
  if (value === null || !/^\d{1,9}$/u.test(value)) {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}
