import type {
  FrontendMetric,
  FrontendTelemetryGateway,
} from '@/features/telemetry/domain/frontend-metric';
import type {
  OnboardingStepGateway,
  OnboardingStepSample,
} from '@/features/telemetry/domain/onboarding-step';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

export type ControlPlaneFrontendTelemetryClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
};

/** 只发送白名单数值；失败不会阻塞或改变用户流程。 */
export class ControlPlaneFrontendTelemetryClient
  implements FrontendTelemetryGateway, OnboardingStepGateway
{
  readonly #endpoint: URL;
  readonly #onboardingEndpoint: URL;
  readonly #fetch: typeof globalThis.fetch;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
  }: ControlPlaneFrontendTelemetryClientOptions) {
    this.#endpoint = controlPlaneEndpoint(baseUrl, '/telemetry/frontend');
    this.#onboardingEndpoint = controlPlaneEndpoint(baseUrl, '/telemetry/onboarding');
    this.#fetch = fetchImplementation;
  }

  async record(sample: FrontendMetric): Promise<void> {
    await this.#send(this.#endpoint, sample);
  }

  async recordStep(sample: OnboardingStepSample): Promise<void> {
    await this.#send(this.#onboardingEndpoint, sample);
  }

  async #send(endpoint: URL, body: FrontendMetric | OnboardingStepSample): Promise<void> {
    try {
      await this.#fetch(endpoint, {
        body: JSON.stringify(body),
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
