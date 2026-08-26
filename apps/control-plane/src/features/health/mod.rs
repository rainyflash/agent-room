mod probes;

use std::time::{SystemTime, UNIX_EPOCH};

use agent_room_application::health::{DependencyState, ProbeFailureKind, ReadinessReport};
use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub(crate) use probes::HealthRuntime;

use crate::{AppState, SERVICE_NAME, SERVICE_VERSION, correlation::CorrelationId};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    checked_at_unix_ms: u64,
    correlation_id: String,
    dependencies: Vec<DependencyResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyResponse {
    name: &'static str,
    status: &'static str,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<&'static str>,
}

pub(crate) async fn live(
    Extension(correlation_id): Extension<CorrelationId>,
) -> Json<LiveResponse> {
    Json(LiveResponse {
        status: "live",
        service: SERVICE_NAME,
        version: SERVICE_VERSION,
        correlation_id: correlation_id.as_uuid().to_string(),
    })
}

pub(crate) async fn ready(
    State(state): State<AppState>,
    Extension(correlation_id): Extension<CorrelationId>,
) -> Response {
    let report = state
        .readiness
        .check(&correlation_id.as_uuid().to_string())
        .await;
    for check in report.checks() {
        state.metrics.record_dependency(
            check.dependency().as_str(),
            check.state() != DependencyState::Unavailable,
            check.latency_millis(),
        );
    }
    record_degraded_dependencies(&report, correlation_id);

    let status = if report.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let dependencies = report
        .checks()
        .iter()
        .map(|check| DependencyResponse {
            name: check.dependency().as_str(),
            status: check.state().as_str(),
            latency_ms: check.latency_millis(),
            failure: check.failure().map(ProbeFailureKind::as_str),
        })
        .collect();

    (
        status,
        Json(ReadyResponse {
            status: report.state().as_str(),
            service: SERVICE_NAME,
            version: SERVICE_VERSION,
            checked_at_unix_ms: current_unix_millis(),
            correlation_id: correlation_id.as_uuid().to_string(),
            dependencies,
        }),
    )
        .into_response()
}

fn record_degraded_dependencies(report: &ReadinessReport, correlation_id: CorrelationId) {
    for check in report
        .checks()
        .iter()
        .filter(|check| check.state() == DependencyState::Unavailable)
    {
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            dependency = check.dependency().as_str(),
            failure = check.failure().map_or("unknown", |failure| failure.as_str()),
            latency_ms = check.latency_millis(),
            "依赖健康检查失败"
        );
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
