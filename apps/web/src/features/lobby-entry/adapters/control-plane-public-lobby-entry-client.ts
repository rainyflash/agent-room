import { z } from 'zod';

import {
  publicLobbyEntryTargetSchema,
  type PublicLobbyEntryFailure,
  type PublicLobbyEntryGateway,
  type PublicLobbyEntryTarget,
} from '@/features/lobby-entry/domain/public-lobby-entry';
import { err, ok, type Result } from '@/shared/result';
import { uuidV7Schema } from '@/shared/validation/identifiers';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).max(128),
  retryable: z.boolean().optional(),
});

export type ControlPlanePublicLobbyEntryClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlanePublicLobbyEntryClient implements PublicLobbyEntryGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlanePublicLobbyEntryClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async resolve(
    catalogId: string,
  ): Promise<Result<PublicLobbyEntryTarget, PublicLobbyEntryFailure>> {
    if (!uuidV7Schema.safeParse(catalogId).success) {
      return err({ code: 'lobby_entry.invalid_catalog_id', retryable: false });
    }
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(
        new URL(`/lobbies/${encodeURIComponent(catalogId)}/observation`, this.#baseUrl),
        {
          cache: 'no-store',
          credentials: 'include',
          headers: { Accept: 'application/json' },
          method: 'GET',
          signal: controller.signal,
        },
      );
      if (!response.ok) return err(await responseFailure(response));
      let payload: unknown;
      try {
        payload = await response.json();
      } catch {
        return err({ code: 'lobby_entry.response_invalid', retryable: true });
      }
      const parsed = publicLobbyEntryTargetSchema.safeParse(payload);
      return parsed.success && parsed.data.catalogId === catalogId
        ? ok(parsed.data)
        : err({ code: 'lobby_entry.response_invalid', retryable: true });
    } catch {
      return err({ code: 'lobby_entry.control_plane_unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function responseFailure(response: Response): Promise<PublicLobbyEntryFailure> {
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
      };
    }
  } catch {
    // 不可信错误正文不得穿透到产品状态。
  }
  return {
    code: `lobby_entry.http_${String(response.status)}`,
    retryable: response.status >= 500,
  };
}
