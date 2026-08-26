use std::sync::Arc;

use agent_room_application::authentication::{AuthenticationRequirement, AuthenticationUseCases};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use url::Url;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store, origin_matches},
    telemetry_metrics::TelemetryMetrics,
};

const MAX_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct FrontendTelemetryHttpState {
    authentication: Arc<dyn AuthenticationUseCases>,
    frontend_origin: String,
    metrics: TelemetryMetrics,
}

impl FrontendTelemetryHttpState {
    pub(crate) fn new(
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &Url,
        metrics: TelemetryMetrics,
    ) -> Self {
        Self {
            authentication,
            frontend_origin: frontend_origin.origin().ascii_serialization(),
            metrics,
        }
    }
}

pub(crate) fn router(state: FrontendTelemetryHttpState) -> Router {
    Router::new()
        .route("/telemetry/frontend", post(record_frontend_metric))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrontendMetric {
    BridgeAvailability,
    BridgeReconnect,
    LargestContentfulPaint,
    InteractionToNextPaint,
    CumulativeLayoutShift,
    TimeToInteractive,
    SceneInitialization,
    MessageOpen,
}

impl FrontendMetric {
    const fn contract(self) -> FrontendMetricContract {
        match self {
            Self::BridgeAvailability => {
                FrontendMetricContract::bounded_score("bridge_availability", 1.0)
            }
            Self::BridgeReconnect => FrontendMetricContract::duration("bridge_reconnect"),
            Self::LargestContentfulPaint => {
                FrontendMetricContract::duration("largest_contentful_paint")
            }
            Self::InteractionToNextPaint => {
                FrontendMetricContract::duration("interaction_to_next_paint")
            }
            Self::CumulativeLayoutShift => FrontendMetricContract::score("cumulative_layout_shift"),
            Self::TimeToInteractive => FrontendMetricContract::duration("time_to_interactive"),
            Self::SceneInitialization => FrontendMetricContract::duration("scene_initialization"),
            Self::MessageOpen => FrontendMetricContract::duration("message_open"),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrontendSurface {
    Web,
    Desktop,
}

impl FrontendSurface {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Desktop => "desktop",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrontendTelemetryRequest {
    metric: FrontendMetric,
    surface: FrontendSurface,
    value: f64,
}

#[derive(Debug, Clone, Copy)]
struct FrontendMetricContract {
    name: &'static str,
    maximum: f64,
    score: bool,
}

impl FrontendMetricContract {
    const fn duration(name: &'static str) -> Self {
        Self {
            name,
            maximum: 60_000.0,
            score: false,
        }
    }

    const fn score(name: &'static str) -> Self {
        Self::bounded_score(name, 10.0)
    }

    const fn bounded_score(name: &'static str, maximum: f64) -> Self {
        Self {
            name,
            maximum,
            score: true,
        }
    }

    fn accepts(self, value: f64) -> bool {
        value.is_finite() && (0.0..=self.maximum).contains(&value)
    }
}

async fn record_frontend_metric(
    State(state): State<FrontendTelemetryHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    payload: Result<Json<FrontendTelemetryRequest>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.frontend_origin) {
        return telemetry_error(
            StatusCode::FORBIDDEN,
            "telemetry.invalid_origin",
            "遥测请求来源无效。",
            correlation_id,
        );
    }
    let Ok(Json(payload)) = payload else {
        return telemetry_error(
            StatusCode::BAD_REQUEST,
            "telemetry.invalid_payload",
            "遥测载荷无效。",
            correlation_id,
        );
    };
    let contract = payload.metric.contract();
    if !contract.accepts(payload.value) {
        return telemetry_error(
            StatusCode::BAD_REQUEST,
            "telemetry.invalid_value",
            "遥测数值超出允许范围。",
            correlation_id,
        );
    }
    if let Err(response) = authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        return response;
    }
    state.metrics.record_frontend(
        contract.name,
        payload.surface.as_str(),
        payload.value,
        contract.score,
    );
    no_store(StatusCode::NO_CONTENT.into_response())
}

fn telemetry_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    correlation_id: CorrelationId,
) -> Response {
    no_store(
        ApiError::new(
            status,
            code,
            if status == StatusCode::FORBIDDEN {
                ErrorCategory::Authorization
            } else {
                ErrorCategory::Validation
            },
            message,
            correlation_id,
        )
        .into_response(),
    )
}

#[cfg(test)]
mod tests {
    use super::{FrontendMetric, FrontendMetricContract};

    #[test]
    fn 前端指标只接受明确边界() {
        assert!(
            FrontendMetric::LargestContentfulPaint
                .contract()
                .accepts(60_000.0)
        );
        assert!(
            FrontendMetric::CumulativeLayoutShift
                .contract()
                .accepts(10.0)
        );
        for value in [-1.0, f64::INFINITY, f64::NAN] {
            assert!(!FrontendMetricContract::duration("test").accepts(value));
        }
        assert!(!FrontendMetricContract::score("test").accepts(10.1));
    }
}
