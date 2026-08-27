import { z } from 'zod';

import type { ReadinessReport } from '@/features/health/domain/readiness';
import type {
  ControlPlaneGateway,
  AuthenticationIntent,
  SessionFailure,
  WebSession,
} from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

const sessionSchema = z
  .object({
    authenticatedAtUnixMs: z.number().int().nonnegative(),
    displayName: z.string().trim().min(1).max(160),
    expiresAtUnixMs: z.number().int().positive(),
    locale: z.string().trim().min(2).max(35),
    matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
    principalId: z.uuid(),
    recentlyAuthenticated: z.boolean(),
  })
  .strict();

const errorEnvelopeSchema = z.looseObject({
  code: z.string(),
  correlationId: z.string().optional(),
  retryable: z.boolean().optional(),
});

const dependencyHealthSchema = z
  .object({
    failure: z.string().optional(),
    latencyMs: z.number().int().nonnegative(),
    name: z.string().min(1),
    status: z.enum(['available', 'unavailable']),
  })
  .strict();

const readinessSchema = z
  .object({
    checkedAtUnixMs: z.number().int().nonnegative(),
    correlationId: z.uuid(),
    dependencies: z.array(dependencyHealthSchema),
    service: z.string().min(1),
    status: z.enum(['degraded', 'ready']),
    version: z.string().min(1),
  })
  .strict();

export type ControlPlaneClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly navigate?: (url: string) => void;
  readonly timeoutMs?: number;
};

export class ControlPlaneClient implements ControlPlaneGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #navigate: (url: string) => void;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    navigate = (url) => {
      window.location.assign(url);
    },
    timeoutMs = 8_000,
  }: ControlPlaneClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#navigate = navigate;
    this.#timeoutMs = timeoutMs;
  }

  beginAuthentication(returnPath: string, intent: AuthenticationIntent = 'sign-in'): void {
    const target = new URL('/auth/oidc/start', this.#baseUrl);
    target.searchParams.set('returnTo', safeReturnPath(returnPath));
    target.searchParams.set('importDisplayName', 'true');
    target.searchParams.set('importLocale', 'true');
    target.searchParams.set('intent', intent);
    this.#navigate(target.toString());
  }

  async readSession(): Promise<Result<WebSession, SessionFailure>> {
    const response = await this.#request('/auth/session', { method: 'GET' });
    if (!response.ok) {
      return response;
    }
    if (response.value.status === 401) {
      return err(failure('control-plane', 'authentication.session_required', false, false));
    }
    if (!response.value.response.ok) {
      return err(await responseFailure(response.value.response, 'control-plane'));
    }

    const body = await readJson(response.value.response);
    if (!body.ok) {
      return body;
    }
    const parsed = sessionSchema.safeParse(body.value);
    return parsed.success
      ? ok(parsed.data)
      : err(
          failure(
            'control-plane',
            'control_plane.invalid_session_response',
            false,
            true,
            response.value.correlationId,
          ),
        );
  }

  async readReadiness(): Promise<Result<ReadinessReport, SessionFailure>> {
    const response = await this.#request('/health/ready', { method: 'GET' });
    if (!response.ok) {
      return response;
    }
    const body = await readJson(response.value.response);
    if (!body.ok) {
      return body;
    }
    const parsed = readinessSchema.safeParse(body.value);
    return parsed.success
      ? ok(parsed.data)
      : err(
          failure(
            'control-plane',
            'control_plane.invalid_readiness_response',
            false,
            true,
            response.value.correlationId,
          ),
        );
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    const response = await this.#request('/auth/logout', { method: 'POST' });
    if (!response.ok) {
      return response;
    }
    return response.value.response.ok || response.value.status === 401
      ? ok(undefined)
      : err(await responseFailure(response.value.response, 'control-plane'));
  }

  async #request(
    path: string,
    init: RequestInit,
  ): Promise<
    Result<
      { readonly correlationId?: string; readonly response: Response; readonly status: number },
      SessionFailure
    >
  > {
    const controller = new AbortController();
    const timeout = window.setTimeout(() => {
      controller.abort();
    }, this.#timeoutMs);
    try {
      const headers = new Headers(init.headers);
      headers.set('Accept', 'application/json');
      const response = await this.#fetch(new URL(path, this.#baseUrl), {
        ...init,
        cache: 'no-store',
        credentials: 'include',
        headers,
        signal: controller.signal,
      });
      const correlationId = response.headers.get('x-correlation-id') ?? undefined;
      return ok({
        ...(correlationId === undefined ? {} : { correlationId }),
        response,
        status: response.status,
      });
    } catch {
      return err(failure('control-plane', 'control_plane.unreachable', !navigator.onLine, true));
    } finally {
      window.clearTimeout(timeout);
    }
  }
}

async function readJson(response: Response): Promise<Result<unknown, SessionFailure>> {
  try {
    return ok(await response.json());
  } catch {
    return err(failure('control-plane', 'control_plane.invalid_json', false, true));
  }
}

async function responseFailure(
  response: Response,
  boundary: SessionFailure['boundary'],
): Promise<SessionFailure> {
  const correlationId = response.headers.get('x-correlation-id') ?? undefined;
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return failure(
        boundary,
        parsed.data.code,
        !navigator.onLine,
        parsed.data.retryable ?? response.status >= 500,
        parsed.data.correlationId ?? correlationId,
      );
    }
  } catch {
    // The deterministic fallback below preserves the HTTP boundary without leaking a body.
  }
  return failure(
    boundary,
    `control_plane.http_${String(response.status)}`,
    !navigator.onLine,
    response.status >= 500,
    correlationId,
  );
}

function safeReturnPath(path: string): string {
  if (!path.startsWith('/') || path.startsWith('//') || path.includes('\\')) {
    return '/connect';
  }
  return path;
}

export function failure(
  boundary: SessionFailure['boundary'],
  code: string,
  offline: boolean,
  retryable: boolean,
  correlationId?: string,
): SessionFailure {
  return {
    boundary,
    code,
    offline,
    retryable,
    ...(correlationId === undefined ? {} : { correlationId }),
  };
}
