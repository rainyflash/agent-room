import { z } from 'zod';

import {
  publicLobbyDirectoryResponseSchema,
  type PublicRoomDirectoryFailure,
  type PublicRoomDirectoryGateway,
  type PublicRoomSummary,
} from '@/features/room-directory/domain/public-room-directory';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).max(128),
  retryable: z.boolean().optional(),
});

export type ControlPlanePublicRoomDirectoryClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlanePublicRoomDirectoryClient implements PublicRoomDirectoryGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlanePublicRoomDirectoryClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async list(): Promise<Result<readonly PublicRoomSummary[], PublicRoomDirectoryFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, '/lobbies/public'), {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'GET',
        signal: controller.signal,
      });
      if (!response.ok) return err(await responseFailure(response));
      const payload = await responseJson(response);
      if (!payload.ok) return payload;
      const parsed = publicLobbyDirectoryResponseSchema.safeParse(payload.value);
      return parsed.success
        ? ok(parsed.data.lobbies)
        : err({ code: 'room_directory.response_invalid', retryable: true });
    } catch {
      return err({ code: 'room_directory.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function responseJson(
  response: Response,
): Promise<Result<unknown, PublicRoomDirectoryFailure>> {
  try {
    return ok(await response.json());
  } catch {
    return err({ code: 'room_directory.response_invalid', retryable: true });
  }
}

async function responseFailure(response: Response): Promise<PublicRoomDirectoryFailure> {
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
    code: `room_directory.http_${String(response.status)}`,
    retryable: response.status >= 500,
  };
}
