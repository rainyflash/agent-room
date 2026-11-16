import { z } from 'zod';

import {
  reminderSettingsSchema,
  type ReminderSettings,
  type ReminderSettingsFailure,
  type ReminderSettingsGateway,
} from '@/features/reminders/domain/reminder-settings';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneReminderSettingsClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

/** 控制面的 `/account/reminders`。桌面端由组合根注入原生层代发的 fetch。 */
export class ControlPlaneReminderSettingsClient implements ReminderSettingsGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 15_000,
  }: ControlPlaneReminderSettingsClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async read(): Promise<Result<ReminderSettings, ReminderSettingsFailure>> {
    return await this.#request({ method: 'GET' });
  }

  async setEmail(enabled: boolean): Promise<Result<ReminderSettings, ReminderSettingsFailure>> {
    return await this.#request({
      body: JSON.stringify({ email: { enabled } }),
      method: 'PUT',
    });
  }

  async #request(init: RequestInit): Promise<Result<ReminderSettings, ReminderSettingsFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const headers = new Headers({ Accept: 'application/json' });
      if (init.body !== undefined) headers.set('Content-Type', 'application/json');
      const response = await this.#fetch(
        controlPlaneEndpoint(this.#baseUrl, '/account/reminders'),
        { ...init, cache: 'no-store', credentials: 'include', headers, signal: controller.signal },
      );
      if (!response.ok) return err(await readFailure(response));
      const parsed = reminderSettingsSchema.safeParse(await response.json());
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'reminders.invalid_response', retryable: false });
    } catch {
      return err({ code: 'reminders.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<ReminderSettingsFailure> {
  const headerCorrelationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const correlationId = parsed.data.correlationId ?? headerCorrelationId;
      return {
        code: parsed.data.code,
        ...(correlationId === undefined ? {} : { correlationId }),
        retryable: parsed.data.retryable ?? response.status >= 500,
      };
    }
  } catch {
    // 不是 JSON：按状态码给一个通用的。
  }
  return {
    code: `reminders.http_${String(response.status)}`,
    ...(headerCorrelationId === undefined ? {} : { correlationId: headerCorrelationId }),
    retryable: response.status >= 500,
  };
}
