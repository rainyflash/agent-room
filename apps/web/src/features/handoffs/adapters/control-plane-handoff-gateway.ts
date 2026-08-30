import { z } from 'zod';

import {
  handoffPermissions,
  handoffPurposes,
  handoffStatuses,
  type HandoffApprovalRequest,
  type HandoffFailure,
  type HandoffFailureCode,
  type HandoffGateway,
  type HandoffSnapshot,
  type HandoffTarget,
} from '@/features/handoffs/domain/handoff';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';
import { matrixRoomIdSchema, uuidV7Schema } from '@/shared/validation/identifiers';

const timestampSchema = z.number().int().nonnegative();
const matrixEventIdSchema = z.string().min(4).max(1_024).startsWith('$');
const handoffTargetSchema = z
  .object({
    adapterType: z.string().min(1).max(128),
    agentAvatarContentId: uuidV7Schema.nullable(),
    agentDisplayName: z.string().trim().min(1).max(80),
    agentId: uuidV7Schema,
    agentInstanceId: uuidV7Schema,
    capabilityVersion: z.string().min(1).max(128),
    device: z
      .object({
        deviceId: uuidV7Schema,
        label: z.string().trim().min(1).max(128),
        platform: z.enum(['windows', 'macos', 'linux', 'web']),
      })
      .strict(),
    instanceStatus: z.enum(['connecting', 'online', 'degraded', 'offline', 'revoked']),
    lastSeenAtUnixMs: timestampSchema.nullable(),
    leaseExpiresAtUnixMs: timestampSchema.nullable(),
    online: z.boolean(),
  })
  .strict();
const handoffTargetListSchema = z
  .object({ targets: z.array(handoffTargetSchema).max(2_000) })
  .strict();
const handoffResponseShape = {
  consumedAtUnixMs: timestampSchema.nullable(),
  content: z
    .object({
      byteLength: z
        .number()
        .int()
        .positive()
        .max(25 * 1_024 * 1_024),
      contentId: uuidV7Schema,
      mediaType: z.string().min(3).max(128),
      sha256: z.string().regex(/^[0-9a-f]{64}$/u),
    })
    .strict(),
  createdAtUnixMs: timestampSchema,
  deliveredAtUnixMs: timestampSchema.nullable(),
  expiresAtUnixMs: timestampSchema,
  failureCode: z.string().min(1).max(128).nullable(),
  handoffId: uuidV7Schema,
  permissions: z.array(z.enum(handoffPermissions)).min(1).max(handoffPermissions.length),
  principalId: uuidV7Schema,
  purpose: z.enum(handoffPurposes),
  queuedAtUnixMs: timestampSchema,
  resolvedAtUnixMs: timestampSchema.nullable(),
  source: z
    .object({
      matrixEventId: matrixEventIdSchema,
      matrixRoomId: matrixRoomIdSchema,
      messageId: uuidV7Schema,
    })
    .strict(),
  status: z.enum(handoffStatuses),
  target: z.object({ agentId: uuidV7Schema, agentInstanceId: uuidV7Schema }).strict(),
  version: z.number().int().nonnegative(),
} as const;
const handoffResponseSchema = z.object(handoffResponseShape).strict();
const createHandoffResponseSchema = z
  .object({ ...handoffResponseShape, created: z.boolean() })
  .strict();
const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).optional(),
  correlationId: z.string().min(1).optional(),
  retryable: z.boolean().optional(),
});

type HandoffResponse = z.output<typeof handoffResponseSchema>;

export type ControlPlaneHandoffGatewayOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneHandoffGateway implements HandoffGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 10_000,
  }: ControlPlaneHandoffGatewayOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async listTargets(roomId: string) {
    const endpoint = controlPlaneEndpoint(this.#baseUrl, '/handoff-targets');
    endpoint.searchParams.set('roomId', roomId);
    const response = await this.#request(endpoint, { method: 'GET' }, handoffTargetListSchema);
    if (!response.ok) {
      return response;
    }
    return ok(Object.freeze(response.value.targets.map((target) => toTarget(target))));
  }

  async approve(request: HandoffApprovalRequest) {
    const response = await this.#request(
      controlPlaneEndpoint(this.#baseUrl, '/handoffs'),
      {
        body: JSON.stringify({
          contentId: request.source.content.contentId,
          expiresAtUnixMs: request.expiresAtUnixMs,
          permissions: request.permissions,
          purpose: request.purpose,
          sourceEventId: request.source.matrixEventId,
          sourceMessageId: request.source.messageId,
          sourceRoomId: request.source.roomId,
          targetInstanceId: request.target.instanceId,
        }),
        headers: {
          'Content-Type': 'application/json',
          'Idempotency-Key': request.handoffId,
        },
        method: 'POST',
      },
      createHandoffResponseSchema,
    );
    if (!response.ok) {
      return response;
    }
    if (!matchesRequest(response.value, request)) {
      return err(invalidResponse());
    }
    return ok({
      kind: 'accepted' as const,
      reused: !response.value.created,
      snapshot: toSnapshot(response.value),
    });
  }

  async reconcile(handoffId: string) {
    return await this.#readSnapshot(`/handoffs/${encodeURIComponent(handoffId)}`, 'GET');
  }

  async revoke(handoffId: string) {
    return await this.#readSnapshot(`/handoffs/${encodeURIComponent(handoffId)}`, 'DELETE');
  }

  async #readSnapshot(path: string, method: 'DELETE' | 'GET') {
    const response = await this.#request(
      controlPlaneEndpoint(this.#baseUrl, path),
      { method },
      handoffResponseSchema,
    );
    return response.ok ? ok(toSnapshot(response.value)) : response;
  }

  async #request<TSchema extends z.ZodType>(
    endpoint: URL,
    init: RequestInit,
    schema: TSchema,
  ): Promise<Result<z.output<TSchema>, HandoffFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const headers = new Headers(init.headers);
      headers.set('Accept', 'application/json');
      const response = await this.#fetch(endpoint, {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers,
        signal: controller.signal,
      });
      if (!response.ok) {
        return err(await readFailure(response));
      }
      const parsed = schema.safeParse(await response.json());
      return parsed.success ? ok(parsed.data) : err(invalidResponse(correlationId(response)));
    } catch (error) {
      return err(
        error instanceof SyntaxError
          ? invalidResponse()
          : failure('handoff.cloud_unavailable', true),
      );
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

function matchesRequest(
  response: z.output<typeof createHandoffResponseSchema>,
  request: HandoffApprovalRequest,
): boolean {
  return (
    response.handoffId === request.handoffId &&
    response.source.matrixRoomId === request.source.roomId &&
    response.source.matrixEventId === request.source.matrixEventId &&
    response.source.messageId === request.source.messageId &&
    response.target.agentId === request.target.agentId &&
    response.target.agentInstanceId === request.target.instanceId &&
    response.content.contentId === request.source.content.contentId &&
    response.content.sha256 === request.source.content.digestSha256 &&
    response.content.byteLength === request.source.content.sizeBytes &&
    response.content.mediaType === request.source.content.mediaType &&
    response.purpose === request.purpose &&
    response.expiresAtUnixMs === request.expiresAtUnixMs &&
    samePermissions(response.permissions, request.permissions)
  );
}

function toTarget(target: z.output<typeof handoffTargetSchema>): HandoffTarget {
  return Object.freeze({
    adapterType: target.adapterType,
    agentAvatarContentId: target.agentAvatarContentId,
    agentDisplayName: target.agentDisplayName,
    agentId: target.agentId,
    capabilityVersion: target.capabilityVersion,
    device: Object.freeze(target.device),
    instanceId: target.agentInstanceId,
    instanceStatus: target.instanceStatus,
    lastSeenAtUnixMs: target.lastSeenAtUnixMs,
    leaseExpiresAtUnixMs: target.leaseExpiresAtUnixMs,
    online: target.online,
  });
}

function samePermissions(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((permission) => right.includes(permission));
}

function toSnapshot(response: HandoffResponse): HandoffSnapshot {
  return Object.freeze({
    consumedAtUnixMs: response.consumedAtUnixMs,
    createdAtUnixMs: response.createdAtUnixMs,
    deliveredAtUnixMs: response.deliveredAtUnixMs,
    expiresAtUnixMs: response.expiresAtUnixMs,
    failureCode: response.failureCode,
    handoffId: response.handoffId,
    queuedAtUnixMs: response.queuedAtUnixMs,
    resolvedAtUnixMs: response.resolvedAtUnixMs,
    status: response.status,
    targetAgentId: response.target.agentId,
    targetInstanceId: response.target.agentInstanceId,
    version: response.version,
  });
}

async function readFailure(response: Response): Promise<HandoffFailure> {
  const headerCorrelationId = correlationId(response);
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return failure(
        mapServerFailure(response.status, parsed.data.code),
        parsed.data.retryable ?? response.status >= 500,
        parsed.data.correlationId ?? headerCorrelationId,
      );
    }
  } catch {
    // 只保留稳定错误分类，不向界面传播未知服务端正文。
  }
  return failure(mapServerFailure(response.status), response.status >= 500, headerCorrelationId);
}

function mapServerFailure(status: number, code?: string): HandoffFailureCode {
  if (status === 401 || status === 403) {
    return 'handoff.authorization_denied';
  }
  if (status === 404 || code === 'targeted_handoff.not_found') {
    return 'handoff.not_found';
  }
  if (status === 409 || code === 'targeted_handoff.conflict') {
    return 'handoff.already_resolved';
  }
  if (code === 'targeted_handoff.target_unavailable') {
    return 'handoff.targets_unavailable';
  }
  if (status >= 500 || code === 'targeted_handoff.dependency_unavailable') {
    return 'handoff.cloud_unavailable';
  }
  return 'handoff.invalid_intent';
}

function invalidResponse(correlation?: string): HandoffFailure {
  return failure('handoff.invalid_response', false, correlation);
}

function failure(
  code: HandoffFailureCode,
  retryable: boolean,
  correlation?: string,
): HandoffFailure {
  return Object.freeze({
    code,
    retryable,
    ...(correlation === undefined ? {} : { correlationId: correlation }),
  });
}

function correlationId(response: Response): string | undefined {
  return response.headers.get('x-correlation-id') ?? undefined;
}
