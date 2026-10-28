import type { ApiRoundTripProbe } from '@/features/telemetry/domain/frontend-metric';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

export type ControlPlaneRoundTripProbeOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly now?: () => number;
};

/** 问一次 `/health/live`：不要登录、不碰数据库，花的时间几乎都在路上。 */
export class ControlPlaneRoundTripProbe implements ApiRoundTripProbe {
  readonly #endpoint: URL;
  readonly #fetch: typeof globalThis.fetch;
  readonly #now: () => number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    now = () => performance.now(),
  }: ControlPlaneRoundTripProbeOptions) {
    this.#endpoint = controlPlaneEndpoint(baseUrl, '/health/live');
    this.#fetch = fetchImplementation;
    this.#now = now;
  }

  async measure(): Promise<number | null> {
    const started = this.#now();
    try {
      const response = await this.#fetch(this.#endpoint, {
        cache: 'no-store',
        credentials: 'omit',
        method: 'GET',
      });
      return response.ok ? this.#now() - started : null;
    } catch {
      return null;
    }
  }
}
