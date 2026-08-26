export type FrontendMetricName =
  | 'bridge_availability'
  | 'bridge_reconnect'
  | 'cumulative_layout_shift'
  | 'interaction_to_next_paint'
  | 'largest_contentful_paint'
  | 'message_open'
  | 'scene_initialization'
  | 'time_to_interactive';

export type FrontendSurface = 'desktop' | 'web';

export type FrontendMetric = {
  readonly metric: FrontendMetricName;
  readonly surface: FrontendSurface;
  readonly value: number;
};

export type FrontendTelemetryGateway = {
  record(sample: FrontendMetric): Promise<void>;
};
