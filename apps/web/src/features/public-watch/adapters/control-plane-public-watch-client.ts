import { z } from 'zod';

import {
  isWatchSlug,
  publicWatchSchema,
  type PublicWatch,
  type PublicWatchFailure,
  type PublicWatchGateway,
} from '@/features/public-watch/domain/public-watch';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).max(128),
  retryable: z.boolean().optional(),
});

export type ControlPlanePublicWatchClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

/**
 * 不登录看公共大厅。这个请求不带登录：用组合根里没有登录范围的 fetch，退出登录时也不会被中止。
 */
export class ControlPlanePublicWatchClient implements PublicWatchGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 10_000,
  }: ControlPlanePublicWatchClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async read(slug: string): Promise<Result<PublicWatch, PublicWatchFailure>> {
    // 不像 slug 的不去问服务器，直接说没有这个大厅。
    if (!isWatchSlug(slug)) return err({ code: 'public_watch.lobby_not_found', retryable: false });
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(
        controlPlaneEndpoint(this.#baseUrl, `/public-lobbies/${slug}/watch`),
        {
          credentials: 'omit',
          headers: { Accept: 'application/json' },
          method: 'GET',
          signal: controller.signal,
        },
      );
      if (!response.ok) return err(await responseFailure(response));
      const payload = await responseJson(response);
      if (!payload.ok) return payload;
      const parsed = publicWatchSchema.safeParse(payload.value);
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'public_watch.response_invalid', retryable: true });
    } catch {
      return err({ code: 'public_watch.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function responseJson(response: Response): Promise<Result<unknown, PublicWatchFailure>> {
  try {
    return ok(await response.json());
  } catch {
    return err({ code: 'public_watch.response_invalid', retryable: true });
  }
}

async function responseFailure(response: Response): Promise<PublicWatchFailure> {
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
      };
    }
  } catch {
    // 响应正文不可信；只暴露稳定错误码。
  }
  return {
    code: `public_watch.http_${String(response.status)}`,
    retryable: response.status >= 500,
  };
}
