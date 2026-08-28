import { z } from 'zod';

import {
  privateRoomListSchema,
  privateRoomSchema,
  type CreatePrivateRoomInput,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomInvitationInput,
  type PrivateRoomPermissions,
  type TransferPrivateRoomOwnershipInput,
} from '@/features/private-rooms/domain/private-room';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlanePrivateRoomClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlanePrivateRoomClient implements PrivateRoomGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlanePrivateRoomClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async list(): Promise<Result<readonly PrivateRoom[], PrivateRoomFailure>> {
    const response = await this.#request(
      '/private-rooms',
      { method: 'GET' },
      privateRoomListSchema,
    );
    return response.ok ? ok(response.value.rooms) : response;
  }

  inspect(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}`, { method: 'GET' });
  }

  create(
    catalogId: string,
    input: CreatePrivateRoomInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest('/private-rooms', {
      body: JSON.stringify(input),
      headers: { 'Idempotency-Key': catalogId },
      method: 'POST',
    });
  }

  invite(
    catalogId: string,
    invitation: PrivateRoomInvitationInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}/invitations`, {
      body: JSON.stringify({
        permissions: invitation.permissions,
        targetPrincipalId: invitation.principalId,
      }),
      method: 'POST',
    });
  }

  accept(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'accept');
  }

  decline(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'decline');
  }

  leave(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'leave');
  }

  remove(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(this.#memberPath(catalogId, principalId), { method: 'DELETE' });
  }

  ban(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`${this.#memberPath(catalogId, principalId)}/ban`, { method: 'POST' });
  }

  updatePermissions(
    catalogId: string,
    principalId: string,
    permissions: PrivateRoomPermissions,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`${this.#memberPath(catalogId, principalId)}/permissions`, {
      body: JSON.stringify(permissions),
      method: 'PUT',
    });
  }

  transferOwnership(
    catalogId: string,
    input: TransferPrivateRoomOwnershipInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}/owner`, {
      body: JSON.stringify(input),
      method: 'PUT',
    });
  }

  archive(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}`, {
      method: 'DELETE',
    });
  }

  #membership(
    catalogId: string,
    action: 'accept' | 'decline' | 'leave',
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(
      `/private-rooms/${encodeURIComponent(catalogId)}/membership/${action}`,
      { method: 'POST' },
    );
  }

  #memberPath(catalogId: string, principalId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/members/${encodeURIComponent(principalId)}`;
  }

  #roomRequest(path: string, init: RequestInit): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#request(path, init, privateRoomSchema);
  }

  async #request<T>(
    path: string,
    init: RequestInit,
    schema: z.ZodType<T>,
  ): Promise<Result<T, PrivateRoomFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
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
      const body: unknown = await response.json();
      const parsed = schema.safeParse(body);
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'private_room.invalid_response', retryable: false });
    } catch {
      return err({ code: 'private_room.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function readFailure(response: Response): Promise<PrivateRoomFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      const responseCorrelationId = parsed.data.correlationId ?? correlationId;
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
        ...(responseCorrelationId === undefined ? {} : { correlationId: responseCorrelationId }),
      };
    }
  } catch {
    // 响应体不可用时仍保留确定性的 HTTP 边界。
  }
  return {
    code: `private_room.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
