import { z } from 'zod';

import type {
  ContentFailure,
  ContentGateway,
  ContentReadTicket,
  DownloadedContent,
} from '@/features/messages/domain/content';
import { err, ok, type Result } from '@/shared/result';

const readTicketSchema = z
  .object({
    expiresAtUnixMs: z.number().int().positive(),
    ticket: z.string().min(16).max(4_096),
  })
  .strict();
const errorEnvelopeSchema = z.looseObject({
  code: z.string(),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneContentClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly online?: () => boolean;
  readonly timeoutMs?: number;
};

export class ControlPlaneContentClient implements ContentGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #online: () => boolean;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    online = browserIsOnline,
    timeoutMs = 20_000,
  }: ControlPlaneContentClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#online = online;
    this.#timeoutMs = timeoutMs;
  }

  async issueReadTicket(contentId: string): Promise<Result<ContentReadTicket, ContentFailure>> {
    const response = await this.#request(`/content/${encodeURIComponent(contentId)}/read-tickets`, {
      headers: { Accept: 'application/json' },
      method: 'POST',
    });
    if (!response.ok) {
      return response;
    }
    if (!response.value.response.ok) {
      return err(await responseFailure(response.value.response, 'content.ticket_rejected'));
    }
    try {
      const parsed = readTicketSchema.safeParse(await response.value.response.json());
      return parsed.success
        ? ok(parsed.data)
        : err(failure('content.invalid_response', false, response.value.correlationId));
    } catch {
      return err(failure('content.invalid_response', false, response.value.correlationId));
    }
  }

  async download(
    contentId: string,
    ticket: string,
  ): Promise<Result<DownloadedContent, ContentFailure>> {
    const response = await this.#request(`/content/${encodeURIComponent(contentId)}/open`, {
      body: JSON.stringify({ ticket }),
      headers: {
        Accept: 'application/octet-stream, text/plain, text/markdown, application/json',
        'Content-Type': 'application/json',
      },
      method: 'POST',
    });
    if (!response.ok) {
      return response;
    }
    if (!response.value.response.ok) {
      return err(await responseFailure(response.value.response, 'content.download_rejected'));
    }
    try {
      const body = new Uint8Array(await response.value.response.arrayBuffer());
      return ok(
        Object.freeze({
          bytes: body,
          contentDigest: response.value.response.headers.get('content-digest'),
          contentLength: response.value.response.headers.get('content-length'),
          mediaType: response.value.response.headers.get('content-type'),
        }),
      );
    } catch {
      return err(failure('content.invalid_response', true, response.value.correlationId));
    }
  }

  async #request(
    path: string,
    init: RequestInit,
  ): Promise<
    Result<{ readonly correlationId?: string; readonly response: Response }, ContentFailure>
  > {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(new URL(path, this.#baseUrl), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        signal: controller.signal,
      });
      const correlationId = response.headers.get('x-correlation-id') ?? undefined;
      return ok({
        ...(correlationId === undefined ? {} : { correlationId }),
        response,
      });
    } catch (error) {
      const timedOut = isAbortError(error);
      return err(
        failure(timedOut ? 'content.timeout' : 'content.offline', timedOut || this.#online()),
      );
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function responseFailure(
  response: Response,
  fallbackCode: 'content.download_rejected' | 'content.ticket_rejected',
): Promise<ContentFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return failure(
        fallbackCode,
        parsed.data.retryable ?? response.status >= 500,
        parsed.data.correlationId ?? correlationId,
      );
    }
  } catch {
    // 稳定的回退错误不会暴露服务端响应正文。
  }
  return failure(fallbackCode, response.status >= 500, correlationId);
}

function failure(
  code: ContentFailure['code'],
  retryable: boolean,
  correlationId?: string,
): ContentFailure {
  return Object.freeze({
    code,
    ...(correlationId === undefined ? {} : { correlationId }),
    retryable,
  });
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

function browserIsOnline(): boolean {
  const navigatorValue: unknown = Reflect.get(globalThis, 'navigator');
  if (typeof navigatorValue !== 'object' || navigatorValue === null) {
    return true;
  }
  const onlineValue: unknown = Reflect.get(navigatorValue, 'onLine');
  return typeof onlineValue === 'boolean' ? onlineValue : true;
}
