import { z } from 'zod';

import {
  agentInstanceListSchema,
  agentInstanceRevocationSchema,
  pendingProductDeviceRevocationSchema,
  productDeviceListSchema,
  type AccessManagementFailure,
  type AccessManagementGateway,
  type AgentInstance,
  type AgentInstanceRevocation,
  type ProductDevice,
  type ProductDeviceRevocation,
} from '@/features/security/domain/access-management';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

export type ControlPlaneAccessManagementClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneAccessManagementClient implements AccessManagementGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneAccessManagementClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async listProductDevices(): Promise<Result<readonly ProductDevice[], AccessManagementFailure>> {
    const response = await this.#request('/auth/devices', { method: 'GET' });
    if (!response.ok) {
      return response;
    }
    return parseProjection(
      response.value,
      productDeviceListSchema,
      'access.invalid_device_list',
      (value) => value.devices,
    );
  }

  async listAgentInstances(): Promise<Result<readonly AgentInstance[], AccessManagementFailure>> {
    const response = await this.#request('/agent-instances', { method: 'GET' });
    if (!response.ok) {
      return response;
    }
    return parseProjection(
      response.value,
      agentInstanceListSchema,
      'access.invalid_instance_list',
      (value) => value.instances,
    );
  }

  async revokeProductDevice(
    deviceId: string,
  ): Promise<Result<ProductDeviceRevocation, AccessManagementFailure>> {
    const response = await this.#request(`/auth/devices/${encodeURIComponent(deviceId)}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      return response;
    }
    if (response.value.status === 204) {
      return ok({ matrixCleanup: 'complete', pendingAgentInstanceCount: 0 });
    }
    const parsed = await parseJson(response.value, pendingProductDeviceRevocationSchema);
    return parsed.ok
      ? ok({
          matrixCleanup: parsed.value.matrixCleanup,
          pendingAgentInstanceCount: parsed.value.pendingAgentInstanceCount,
        })
      : err({ code: 'access.invalid_device_revocation', retryable: false });
  }

  async revokeAgentInstance(
    instanceId: string,
  ): Promise<Result<AgentInstanceRevocation, AccessManagementFailure>> {
    const response = await this.#request(`/agent-instances/${encodeURIComponent(instanceId)}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      return response;
    }
    const parsed = await parseJson(response.value, agentInstanceRevocationSchema);
    return parsed.ok
      ? parsed
      : err({ code: 'access.invalid_instance_revocation', retryable: false });
  }

  async #request(
    path: string,
    init: RequestInit,
  ): Promise<Result<Response, AccessManagementFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => controller.abort(), this.#timeoutMs);
    try {
      const response = await this.#fetch(new URL(path, this.#baseUrl), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json', ...init.headers },
        signal: controller.signal,
      });
      return response.ok ? ok(response) : err(await readFailure(response));
    } catch {
      return err({ code: 'access.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function parseProjection<TWire, TValue>(
  response: Response,
  schema: z.ZodType<TWire>,
  failureCode: string,
  project: (value: TWire) => TValue,
): Promise<Result<TValue, AccessManagementFailure>> {
  const parsed = await parseJson(response, schema);
  return parsed.ok ? ok(project(parsed.value)) : err({ code: failureCode, retryable: false });
}

async function parseJson<T>(
  response: Response,
  schema: z.ZodType<T>,
): Promise<Result<T, AccessManagementFailure>> {
  try {
    const body: unknown = await response.json();
    const parsed = schema.safeParse(body);
    return parsed.success
      ? ok(parsed.data)
      : err({ code: 'access.invalid_response', retryable: false });
  } catch {
    return err({ code: 'access.invalid_json', retryable: false });
  }
}

async function readFailure(response: Response): Promise<AccessManagementFailure> {
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
    // 错误正文不可解析时退回稳定的 HTTP 错误码。
  }
  return {
    code: `access.http_${String(response.status)}`,
    retryable: response.status >= 500,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
