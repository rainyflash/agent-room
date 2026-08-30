import { z } from 'zod';

import type {
  MessageContentBindingRequest,
  MessageContentUploadRequest,
  MessagePublicationContentGateway,
  MessagePublicationFailure,
} from '@/features/messages/domain/publication';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const contentObjectShape = {
  byteLength: z
    .number()
    .int()
    .positive()
    .max(25 * 1_024 * 1_024),
  contentId: z.uuid(),
  createdAtUnixMs: z.number().int().nonnegative(),
  encryptionMode: z.enum(['server_side', 'client_e2ee']),
  expiresAtUnixMs: z.number().int().positive().nullable(),
  lifecycleState: z.enum(['uploading', 'active', 'orphaned', 'redacted', 'expired', 'deleted']),
  mediaType: z.string().min(3).max(128),
  scanState: z.enum(['pending', 'clean', 'suspicious', 'rejected', 'not_applicable']),
  sha256: z.string().regex(/^[0-9a-f]{64}$/u),
} as const;
const beginUploadSchema = z
  .object({
    ...contentObjectShape,
    accessMode: z.literal('room_member'),
    created: z.boolean(),
    matrixRoomId: z.string().min(4).max(255),
  })
  .strict();
const completeUploadSchema = z
  .object({ ...contentObjectShape, alreadyActive: z.boolean() })
  .strict();
const bindEventSchema = z
  .object({
    accessMode: z.literal('room_member'),
    alreadyBound: z.boolean(),
    contentId: z.uuid(),
    matrixEventId: z.string().min(2).max(1_024).startsWith('$'),
    matrixRoomId: z.string().min(4).max(255),
  })
  .strict();
const errorEnvelopeSchema = z.looseObject({
  code: z.string().optional(),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneMessagePublicationContentGatewayOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneMessagePublicationContentGateway implements MessagePublicationContentGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 30_000,
  }: ControlPlaneMessagePublicationContentGatewayOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async upload(request: MessageContentUploadRequest) {
    const begin = await this.#request('/content/uploads', {
      body: JSON.stringify({
        accessMode: 'room_member',
        byteLength: request.body.bytes.byteLength,
        encryptionMode: 'server_side',
        matrixRoomId: request.roomId,
        mediaType: request.mediaType,
        sha256: request.body.digestSha256,
      }),
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        'Idempotency-Key': request.submissionId,
      },
      method: 'POST',
    });
    if (!begin.ok) {
      return begin;
    }
    const begun = await parseResponse(begin.value, beginUploadSchema);
    if (
      !begun.ok ||
      begun.value.matrixRoomId !== request.roomId ||
      begun.value.sha256 !== request.body.digestSha256 ||
      begun.value.byteLength !== request.body.bytes.byteLength ||
      begun.value.mediaType !== request.mediaType
    ) {
      return begun.ok ? err(invalidResponse(begin.value.correlationId)) : begun;
    }

    const complete = await this.#request(
      `/content/${encodeURIComponent(begun.value.contentId)}/bytes`,
      {
        body: request.body.bytes,
        headers: { Accept: 'application/json', 'Content-Type': request.mediaType },
        method: 'PUT',
      },
    );
    if (!complete.ok) {
      return complete;
    }
    const completed = await parseResponse(complete.value, completeUploadSchema);
    if (
      !completed.ok ||
      completed.value.contentId !== begun.value.contentId ||
      completed.value.sha256 !== request.body.digestSha256 ||
      completed.value.byteLength !== request.body.bytes.byteLength ||
      completed.value.mediaType !== request.mediaType ||
      completed.value.lifecycleState !== 'active'
    ) {
      return completed.ok ? err(invalidResponse(complete.value.correlationId)) : completed;
    }
    return ok(
      Object.freeze({
        contentId: completed.value.contentId,
        digestSha256: completed.value.sha256,
        mediaType: completed.value.mediaType,
        sizeBytes: completed.value.byteLength,
      }),
    );
  }

  async bind(
    request: MessageContentBindingRequest,
  ): Promise<Result<void, MessagePublicationFailure>> {
    const response = await this.#request(
      `/content/${encodeURIComponent(request.contentId)}/event-binding`,
      {
        body: JSON.stringify({
          matrixEventId: request.matrixEventId,
          matrixRoomId: request.roomId,
        }),
        headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
        method: 'PUT',
      },
    );
    if (!response.ok) {
      return response;
    }
    const bound = await parseResponse(response.value, bindEventSchema);
    if (!bound.ok) {
      return bound;
    }
    return bound.value.contentId === request.contentId &&
      bound.value.matrixEventId === request.matrixEventId &&
      bound.value.matrixRoomId === request.roomId
      ? ok(undefined)
      : err(invalidResponse(response.value.correlationId));
  }

  async #request(
    path: string,
    init: RequestInit,
  ): Promise<Result<HttpResponse, MessagePublicationFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const response = await this.#fetch(controlPlaneEndpoint(this.#baseUrl, path), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        signal: controller.signal,
      });
      const correlationId = response.headers.get('x-correlation-id') ?? undefined;
      if (!response.ok) {
        return err(await responseFailure(response, correlationId));
      }
      return ok({ ...(correlationId === undefined ? {} : { correlationId }), response });
    } catch {
      return err(Object.freeze({ code: 'publication.content_rejected', retryable: true }));
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

type HttpResponse = { readonly correlationId?: string; readonly response: Response };

async function parseResponse<TSchema extends z.ZodType>(
  value: HttpResponse,
  schema: TSchema,
): Promise<Result<z.output<TSchema>, MessagePublicationFailure>> {
  try {
    const parsed = schema.safeParse(await value.response.json());
    return parsed.success ? ok(parsed.data) : err(invalidResponse(value.correlationId));
  } catch {
    return err(invalidResponse(value.correlationId));
  }
}

async function responseFailure(
  response: Response,
  correlationId?: string,
): Promise<MessagePublicationFailure> {
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const resolvedCorrelationId = parsed.data.correlationId ?? correlationId;
      return Object.freeze({
        code: 'publication.content_rejected',
        ...(resolvedCorrelationId === undefined ? {} : { correlationId: resolvedCorrelationId }),
        retryable: parsed.data.retryable ?? response.status >= 500,
      });
    }
  } catch {
    // 返回稳定的边界错误，不传播服务端正文。
  }
  return Object.freeze({
    code: 'publication.content_rejected',
    ...(correlationId === undefined ? {} : { correlationId }),
    retryable: response.status >= 500,
  });
}

function invalidResponse(correlationId?: string): MessagePublicationFailure {
  return Object.freeze({
    code: 'publication.content_rejected',
    ...(correlationId === undefined ? {} : { correlationId }),
    retryable: false,
  });
}
