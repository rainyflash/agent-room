import type { SessionFailure } from '@/features/session/domain/session';
import type { Result } from '@/shared/result';

export type DependencyHealth = {
  readonly failure?: string | undefined;
  readonly latencyMs: number;
  readonly name: string;
  readonly status: 'available' | 'unavailable';
};

export type ReadinessReport = {
  readonly checkedAtUnixMs: number;
  readonly correlationId: string;
  readonly dependencies: readonly DependencyHealth[];
  readonly service: string;
  readonly status: 'degraded' | 'ready';
  readonly version: string;
};

export type ReadinessGateway = {
  readReadiness(): Promise<Result<ReadinessReport, SessionFailure>>;
};
