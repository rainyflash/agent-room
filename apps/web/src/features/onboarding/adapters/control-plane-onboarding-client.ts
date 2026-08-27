import { z } from 'zod';

import {
  onboardingAgentListSchema,
  onboardingAgentSchema,
  publicLobbyDirectorySchema,
  type OnboardingAgent,
  type OnboardingFailure,
  type OnboardingGateway,
  type PublicLobby,
} from '@/features/onboarding/domain/onboarding';
import { err, ok, type Result } from '@/shared/result';

const errorEnvelopeSchema = z.looseObject({
  code: z.string().min(1).max(128),
  retryable: z.boolean().optional(),
});

export type ControlPlaneOnboardingClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

export class ControlPlaneOnboardingClient implements OnboardingGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneOnboardingClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async listAgents(): Promise<Result<readonly OnboardingAgent[], OnboardingFailure>> {
    const response = await this.#request('/agents', 'GET');
    if (!response.ok) return response;
    const parsed = onboardingAgentListSchema.safeParse(response.value);
    return parsed.success
      ? ok(parsed.data.agents)
      : err({ code: 'onboarding.agents_response_invalid', retryable: true });
  }

  async ensureDefaultAgent(): Promise<Result<OnboardingAgent, OnboardingFailure>> {
    const response = await this.#request('/onboarding/default-agent', 'PUT');
    if (!response.ok) return response;
    const parsed = onboardingAgentSchema.safeParse(response.value);
    return parsed.success
      ? ok(parsed.data)
      : err({ code: 'onboarding.agent_response_invalid', retryable: true });
  }

  async listPublicLobbies(): Promise<Result<readonly PublicLobby[], OnboardingFailure>> {
    const response = await this.#request('/lobbies/public', 'GET');
    if (!response.ok) return response;
    const parsed = publicLobbyDirectorySchema.safeParse(response.value);
    return parsed.success
      ? ok(parsed.data.lobbies)
      : err({ code: 'onboarding.lobbies_response_invalid', retryable: true });
  }

  async #request(path: string, method: 'GET' | 'PUT'): Promise<Result<unknown, OnboardingFailure>> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => controller.abort(), this.#timeoutMs);
    try {
      const response = await this.#fetch(new URL(path, this.#baseUrl), {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method,
        signal: controller.signal,
      });
      if (!response.ok) return err(await responseFailure(response));
      try {
        return ok(await response.json());
      } catch {
        return err({ code: 'onboarding.invalid_json', retryable: true });
      }
    } catch {
      return err({ code: 'onboarding.unreachable', retryable: true });
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

async function responseFailure(response: Response): Promise<OnboardingFailure> {
  try {
    const parsed = errorEnvelopeSchema.safeParse(await response.json());
    if (parsed.success) {
      return {
        code: parsed.data.code,
        retryable: parsed.data.retryable ?? response.status >= 500,
      };
    }
  } catch {
    // 响应正文不可信；保留确定性的 HTTP 边界，不泄漏服务端正文。
  }
  return {
    code: `onboarding.http_${String(response.status)}`,
    retryable: response.status >= 500,
  };
}
