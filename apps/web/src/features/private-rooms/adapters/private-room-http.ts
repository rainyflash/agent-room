import { z } from 'zod';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
  retryAfterSeconds: z.number().int().nonnegative().optional(),
  // 只要 `condition`（实况开不了时差的那一样）和 `reason`（分享一段对话时选的那几句哪里不对）；
  // 别的样子当成没有，不让整个错误读不出来。
  details: z
    .looseObject({ condition: z.string().optional(), reason: z.string().optional() })
    .optional()
    .catch(undefined),
});

export type PrivateRoomHttpOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

/**
 * 私人房间和子房间的客户端共用的那一段：带着登录发请求，按 schema 读回答，出错时读出错误码。
 * 请求不加缓存；超时、连不上都说 `private_room.unreachable`，可以再试。
 */
export class PrivateRoomHttp {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: PrivateRoomHttpOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  request<T>(
    path: string,
    init: RequestInit,
    schema: z.ZodType<T>,
    timeoutMs = this.#timeoutMs,
  ): Promise<Result<T, PrivateRoomFailure>> {
    return this.#exchange(
      path,
      init,
      async (response) => {
        const parsed = schema.safeParse(await response.json());
        return parsed.success
          ? ok(parsed.data)
          : err({ code: 'private_room.invalid_response', retryable: false });
      },
      timeoutMs,
    );
  }

  /** Writes that answer 204 without a body. */
  send(path: string, init: RequestInit): Promise<Result<void, PrivateRoomFailure>> {
    return this.#exchange(path, init, () => Promise.resolve(ok(undefined)), this.#timeoutMs);
  }

  async #exchange<T>(
    path: string,
    init: RequestInit,
    read: (response: Response) => Promise<Result<T, PrivateRoomFailure>>,
    timeoutMs: number,
  ): Promise<Result<T, PrivateRoomFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, timeoutMs);
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
      return await read(response);
    } catch {
      return err({ code: 'private_room.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<PrivateRoomFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  const headerRetryAfter = parseRetryAfter(response.headers.get('retry-after'));
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const responseCorrelationId = parsed.data.correlationId ?? correlationId;
      const retryAfterSeconds = parsed.data.retryAfterSeconds ?? headerRetryAfter;
      const condition = parsed.data.details?.condition;
      const reason = parsed.data.details?.reason;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(responseCorrelationId === undefined ? {} : { correlationId: responseCorrelationId }),
        ...(retryAfterSeconds === undefined ? {} : { retryAfterSeconds }),
        ...(condition === undefined ? {} : { condition }),
        ...(reason === undefined ? {} : { reason }),
      };
    }
  } catch {
    // 响应体不可用时仍保留确定性的 HTTP 边界。
  }
  return {
    code: `private_room.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
    ...(headerRetryAfter === undefined ? {} : { retryAfterSeconds: headerRetryAfter }),
  };
}

/** `Retry-After` 只认秒数；写成日期的不认。 */
function parseRetryAfter(value: string | null): number | undefined {
  if (value === null || !/^\d{1,9}$/u.test(value.trim())) return undefined;
  const parsed = Number(value.trim());
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}
