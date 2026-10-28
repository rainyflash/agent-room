export type FrontendMetricName =
  | 'api_round_trip'
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

/** 调一次很轻的控制面接口，量网络本身来回一趟要多久（毫秒）；没量成给 `null`。 */
export type ApiRoundTripProbe = {
  measure(): Promise<number | null>;
};
