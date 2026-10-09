import { z } from 'zod';

import {
  ACCOUNT_DELETION_CONFIRMATION,
  accountDeletionStartedSchema,
  accountExportSchema,
  type AccountExport,
  type AccountFailure,
  type AccountGateway,
} from '@/features/account/domain/account';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneAccountClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly now?: () => number;
  readonly timeoutMs?: number;
};

/** 控制面的 `/account/export` 和 `DELETE /account`。桌面端由组合根注入原生层代发的 fetch。 */
export class ControlPlaneAccountClient implements AccountGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #now: () => number;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    now = Date.now,
    timeoutMs = 30_000,
  }: ControlPlaneAccountClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#now = now;
    this.#timeoutMs = timeoutMs;
  }

  async exportData(): Promise<Result<AccountExport, AccountFailure>> {
    const response = await this.#request('/account/export', { method: 'GET' });
    if (!response.ok) return response;
    try {
      const parsed = accountExportSchema.safeParse(await response.value.json());
      if (!parsed.success) return err({ code: 'account.invalid_export', retryable: false });
      const date = new Date(this.#now()).toISOString().slice(0, 10);
      return ok({
        fileName: `agent-room-account-${date}.json`,
        json: `${JSON.stringify(parsed.data, null, 2)}\n`,
      });
    } catch {
      return err({ code: 'account.invalid_export', retryable: false });
    }
  }

  async requestDeletion(idempotencyKey: string): Promise<Result<void, AccountFailure>> {
    const response = await this.#request('/account', {
      body: JSON.stringify({
        confirmation: ACCOUNT_DELETION_CONFIRMATION,
        federationResidualAcknowledged: true,
      }),
      headers: { 'Idempotency-Key': idempotencyKey },
      method: 'DELETE',
    });
    if (!response.ok) return response;
    try {
      const parsed = accountDeletionStartedSchema.safeParse(await response.value.json());
      return parsed.success
        ? ok(undefined)
        : err({ code: 'account.invalid_deletion_response', retryable: false });
    } catch {
      return err({ code: 'account.invalid_deletion_response', retryable: false });
    }
  }

  async #request(path: string, init: RequestInit): Promise<Result<Response, AccountFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const headers = new Headers(init.headers);
      headers.set('Accept', 'application/json');
      if (init.body !== undefined) headers.set('Content-Type', 'application/json');
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, path), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers,
        signal: controller.signal,
      });
      return response.ok ? ok(response) : err(await readFailure(response));
    } catch {
      return err({ code: 'account.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<AccountFailure> {
  const headerCorrelationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const correlationId = parsed.data.correlationId ?? headerCorrelationId;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(correlationId === undefined ? {} : { correlationId }),
      };
    }
  } catch {
    // 错误正文读不出来时只按状态码说。
  }
  return {
    code: `account.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(headerCorrelationId === undefined ? {} : { correlationId: headerCorrelationId }),
  };
}
