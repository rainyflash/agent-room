import type {
  FrontendMetric,
  FrontendTelemetryGateway,
} from '@/features/telemetry/domain/frontend-metric';

export type ControlPlaneFrontendTelemetryClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
};

/** 只发送白名单数值；失败不会阻塞或改变用户流程。 */
export class ControlPlaneFrontendTelemetryClient implements FrontendTelemetryGateway {
  readonly #endpoint: URL;
  readonly #fetch: typeof globalThis.fetch;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
  }: ControlPlaneFrontendTelemetryClientOptions) {
    this.#endpoint = new URL('/telemetry/frontend', baseUrl);
    this.#fetch = fetchImplementation;
  }

  async record(sample: FrontendMetric): Promise<void> {
    try {
      await this.#fetch(this.#endpoint, {
        body: JSON.stringify(sample),
        cache: 'no-store',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        keepalive: true,
        method: 'POST',
      });
    } catch {
      // 遥测是旁路能力；网络失败由服务端缺口告警反映，不污染用户界面。
    }
  }
}
