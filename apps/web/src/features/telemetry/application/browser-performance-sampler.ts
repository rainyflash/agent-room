import type {
  FrontendMetricName,
  FrontendSurface,
  FrontendTelemetryGateway,
} from '@/features/telemetry/domain/frontend-metric';

type MetricAccumulator = Map<FrontendMetricName, number>;

/** 浏览器性能条目只在本地聚合，提交时不附带 URL、选择器或资源名称。 */
export class BrowserPerformanceSampler {
  readonly #gateway: FrontendTelemetryGateway;
  readonly #surface: FrontendSurface;
  readonly #accumulator: MetricAccumulator = new Map();
  readonly #observers: PerformanceObserver[] = [];
  #started = false;

  constructor(gateway: FrontendTelemetryGateway, surface: FrontendSurface) {
    this.#gateway = gateway;
    this.#surface = surface;
  }

  start(): () => void {
    if (this.#started) {
      return () => undefined;
    }
    this.#started = true;
    this.#recordNavigationTiming();
    this.#observeLargestContentfulPaint();
    this.#observeLayoutShift();
    this.#observeInteractionLatency();
    window.addEventListener('pagehide', this.#flush);
    document.addEventListener('visibilitychange', this.#flushWhenHidden);
    return () => {
      this.#flush();
      this.#observers.forEach((observer) => {
        observer.disconnect();
      });
      this.#observers.length = 0;
      window.removeEventListener('pagehide', this.#flush);
      document.removeEventListener('visibilitychange', this.#flushWhenHidden);
      this.#started = false;
    };
  }

  readonly #flushWhenHidden = (): void => {
    if (document.visibilityState === 'hidden') {
      this.#flush();
    }
  };

  readonly #flush = (): void => {
    for (const [metric, value] of this.#accumulator) {
      void this.#gateway.record({ metric, surface: this.#surface, value });
    }
    this.#accumulator.clear();
  };

  #recordNavigationTiming(): void {
    const navigation = performance.getEntriesByType('navigation')[0];
    if (navigation === undefined || !hasFiniteNumber(navigation, 'domInteractive')) {
      return;
    }
    this.#setMaximum('time_to_interactive', navigation.domInteractive);
  }

  #observeLargestContentfulPaint(): void {
    this.#observe('largest-contentful-paint', (entry) => {
      this.#setMaximum('largest_contentful_paint', entry.startTime);
    });
  }

  #observeLayoutShift(): void {
    this.#observe('layout-shift', (entry) => {
      if (
        hasFiniteNumber(entry, 'value') &&
        hasBoolean(entry, 'hadRecentInput') &&
        !entry.hadRecentInput
      ) {
        const current = this.#accumulator.get('cumulative_layout_shift') ?? 0;
        this.#accumulator.set('cumulative_layout_shift', current + entry.value);
      }
    });
  }

  #observeInteractionLatency(): void {
    this.#observe('event', (entry) => {
      this.#setMaximum('interaction_to_next_paint', entry.duration);
    });
  }

  #observe(type: string, consume: (entry: PerformanceEntry) => void): void {
    if (typeof PerformanceObserver === 'undefined') {
      return;
    }
    try {
      const observer = new PerformanceObserver((list) => {
        list.getEntries().forEach(consume);
      });
      observer.observe({ buffered: true, type });
      this.#observers.push(observer);
    } catch {
      // 不支持该条目类型的浏览器只跳过该指标。
    }
  }

  #setMaximum(metric: FrontendMetricName, value: number): void {
    if (!Number.isFinite(value) || value < 0 || value > 60_000) {
      return;
    }
    const current = this.#accumulator.get(metric) ?? 0;
    this.#accumulator.set(metric, Math.max(current, value));
  }
}

function hasFiniteNumber<Key extends string>(
  value: object,
  key: Key,
): value is object & Record<Key, number> {
  const candidate: unknown = Reflect.get(value, key);
  return typeof candidate === 'number' && Number.isFinite(candidate);
}

function hasBoolean<Key extends string>(
  value: object,
  key: Key,
): value is object & Record<Key, boolean> {
  return typeof Reflect.get(value, key) === 'boolean';
}
