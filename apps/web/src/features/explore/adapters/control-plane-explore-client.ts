import { z } from 'zod';

import {
  explorePageSchema,
  exploreRoomDetailSchema,
  exploreSearchParams,
  exploreShareCodeSchema,
  type ExploreFailure,
  type ExploreFilter,
  type ExploreGateway,
  type ExplorePage,
  type ExploreRoomDetail,
} from '@/features/explore/domain/explore';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).max(128),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneExploreClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

/**
 * 发现页的目录和详情。不登录也能读，所以用组合根里没有登录范围的 fetch：没登录的人打开时清理登录
 * 状态也不会中止它。照样带上 Cookie，登录了服务器就写上你在不在里面。
 */
export class ControlPlaneExploreClient implements ExploreGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 10_000,
  }: ControlPlaneExploreClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async browse(
    filter: ExploreFilter,
    cursor: string | null,
  ): Promise<Result<ExplorePage, ExploreFailure>> {
    const endpoint = controlPlaneEndpoint(this.#baseUrl, '/public-rooms');
    endpoint.search = exploreSearchParams(filter, cursor).toString();
    const payload = await this.#get(endpoint);
    if (!payload.ok) return payload;
    const parsed = explorePageSchema.safeParse(payload.value);
    return parsed.success
      ? ok({ nextCursor: parsed.data.nextCursor ?? null, rooms: parsed.data.rooms })
      : err({ code: 'public_room.invalid_response', retryable: false });
  }

  async detail(shareCode: string): Promise<Result<ExploreRoomDetail, ExploreFailure>> {
    // 不像分享码的不去问服务器，和不公开的房间说同一句。
    if (!exploreShareCodeSchema.safeParse(shareCode).success) {
      return err({ code: 'public_room.not_found', retryable: false });
    }
    const payload = await this.#get(
      controlPlaneEndpoint(this.#baseUrl, `/public-rooms/${shareCode}`),
    );
    if (!payload.ok) return payload;
    const parsed = exploreRoomDetailSchema.safeParse(payload.value);
    return parsed.success
      ? ok(parsed.data)
      : err({ code: 'public_room.invalid_response', retryable: false });
  }

  async #get(endpoint: URL): Promise<Result<unknown, ExploreFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(endpoint, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'GET',
        signal: controller.signal,
      });
      if (!response.ok) return err(await readFailure(response));
      return ok(await response.json());
    } catch {
      return err({ code: 'public_room.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<ExploreFailure> {
  const retryAfter = Number(response.headers.get('retry-after'));
  const timing =
    Number.isInteger(retryAfter) && retryAfter > 0 ? { retryAfterSeconds: retryAfter } : {};
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? (response.status >= 500 || response.status === 429),
        ...(parsed.data.correlationId === undefined
          ? {}
          : { correlationId: parsed.data.correlationId }),
        ...timing,
      };
    }
  } catch {
    // 响应正文不可信；只留稳定的错误码。
  }
  return {
    code: `public_room.http_${String(response.status)}`,
    retryable: response.status >= 500 || response.status === 429,
    ...timing,
  };
}
