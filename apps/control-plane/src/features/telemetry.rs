use std::sync::Arc;

use agent_room_application::authentication::{AuthenticationRequirement, AuthenticationUseCases};
use agent_room_domain::regions::Region;
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
    features::authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
    request_regions::RequestRegion,
    telemetry_metrics::TelemetryMetrics,
};

const MAX_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct FrontendTelemetryHttpState {
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
    metrics: TelemetryMetrics,
}

impl FrontendTelemetryHttpState {
    pub(crate) fn new(
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &Url,
        desktop_origins: &crate::config::DesktopOrigins,
        metrics: TelemetryMetrics,
    ) -> Self {
        Self {
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
            metrics,
        }
        .with_onboarding_series()
    }

    /// 每种组合先报一个 0：计数第一次出现就是 1 的话，Prometheus 的 `increase` 看不出它涨过，
    /// 周报的数都不大，每次重启少算一条序列的一次就差得多。
    fn with_onboarding_series(self) -> Self {
        for (step, method) in ONBOARDING_SERIES {
            for surface in [FrontendSurface::Web, FrontendSurface::Desktop] {
                self.metrics
                    .record_onboarding_step(step.as_str(), method, surface.as_str(), 0);
            }
        }
        self
    }
}

pub(crate) fn router(state: FrontendTelemetryHttpState) -> Router {
    Router::new()
        .route("/telemetry/frontend", post(record_frontend_metric))
        .route("/telemetry/onboarding", post(record_onboarding_step))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrontendMetric {
    /// 旧版桌面端在本机连接的状态变了时报一次：连上 1、没连上 0。新版改报 `BridgeUptime`。
    BridgeAvailability,
    /// 每 15 分钟一次：这段时间里本机连接该连着的时候，有多少是连着的（0 到 1）。
    BridgeUptime,
    BridgeReconnect,
    LargestContentfulPaint,
    InteractionToNextPaint,
    CumulativeLayoutShift,
    TimeToInteractive,
    SceneInitialization,
    MessageOpen,
    /// 打开时调一次很轻的接口花了多久：看的是网络本身慢不慢，和页面加载分开。
    ApiRoundTrip,
}

impl FrontendMetric {
    const fn contract(self) -> FrontendMetricContract {
        match self {
            Self::BridgeAvailability => {
                FrontendMetricContract::bounded_score("bridge_availability", 1.0)
            }
            Self::BridgeUptime => FrontendMetricContract::bounded_score("bridge_uptime", 1.0),
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
            Self::ApiRoundTrip => FrontendMetricContract::duration("api_round_trip"),
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

/// 接入 Agent 对话框里只有前端知道的几步（`specs/onboarding-funnel/design.md` 第 2 节）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OnboardingStep {
    InviteOpened,
    InviteCopied,
    InviteArrived,
    DesktopDownload,
}

impl OnboardingStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InviteOpened => "invite_opened",
            Self::InviteCopied => "invite_copied",
            Self::InviteArrived => "invite_arrived",
            Self::DesktopDownload => "desktop_download",
        }
    }

    /// 复制和等到 Agent 分接入方式，别的两步没有方式可说。
    const fn takes_method(self) -> bool {
        matches!(self, Self::InviteCopied | Self::InviteArrived)
    }
}

const NO_METHOD: &str = "none";
const ONBOARDING_SERIES: [(OnboardingStep, &str); 8] = [
    (OnboardingStep::InviteOpened, NO_METHOD),
    (OnboardingStep::InviteCopied, "cli"),
    (OnboardingStep::InviteCopied, "mcp"),
    (OnboardingStep::InviteCopied, "network"),
    (OnboardingStep::InviteArrived, "cli"),
    (OnboardingStep::InviteArrived, "mcp"),
    (OnboardingStep::InviteArrived, "network"),
    (OnboardingStep::DesktopDownload, NO_METHOD),
];

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OnboardingMethod {
    Cli,
    Mcp,
    Network,
}

impl OnboardingMethod {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Mcp => "mcp",
            Self::Network => "network",
        }
    }
}

/// 只有步骤、接入方式和是网页还是桌面端：不带账户、房间和地址，控制面也不记地区。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OnboardingStepRequest {
    step: OnboardingStep,
    #[serde(default)]
    method: Option<OnboardingMethod>,
    surface: FrontendSurface,
}

impl OnboardingStepRequest {
    const fn is_consistent(&self) -> bool {
        self.step.takes_method() == self.method.is_some()
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
    region: Option<Extension<RequestRegion>>,
    headers: HeaderMap,
    jar: CookieJar,
    payload: Result<Json<FrontendTelemetryRequest>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
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
    let region = region.map_or(Region::Unknown, |Extension(RequestRegion(region))| region);
    state.metrics.record_frontend(
        contract.name,
        payload.surface.as_str(),
        region.slug(),
        payload.value,
        contract.score,
    );
    no_store(StatusCode::NO_CONTENT.into_response())
}

async fn record_onboarding_step(
    State(state): State<FrontendTelemetryHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    payload: Result<Json<OnboardingStepRequest>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return telemetry_error(
            StatusCode::FORBIDDEN,
            "telemetry.invalid_origin",
            "遥测请求来源无效。",
            correlation_id,
        );
    }
    let payload = match payload {
        Ok(Json(payload)) if payload.is_consistent() => payload,
        _ => {
            return telemetry_error(
                StatusCode::BAD_REQUEST,
                "telemetry.invalid_payload",
                "遥测载荷无效。",
                correlation_id,
            );
        }
    };
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
    state.metrics.record_onboarding_step(
        payload.step.as_str(),
        payload.method.map_or(NO_METHOD, OnboardingMethod::as_str),
        payload.surface.as_str(),
        1,
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
    use super::{
        FrontendMetric, FrontendMetricContract, NO_METHOD, ONBOARDING_SERIES, OnboardingStepRequest,
    };

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

    fn onboarding(json: &str) -> Option<OnboardingStepRequest> {
        serde_json::from_str(json).ok()
    }

    #[test]
    fn 接入步骤只有步骤方式和表面() {
        for json in [
            r#"{"step":"invite_opened","surface":"web"}"#,
            r#"{"step":"desktop_download","surface":"web"}"#,
            r#"{"step":"invite_copied","method":"network","surface":"web"}"#,
            r#"{"step":"invite_arrived","method":"cli","surface":"desktop"}"#,
        ] {
            assert!(
                onboarding(json).is_some_and(|request| request.is_consistent()),
                "{json}"
            );
        }
        // 复制和等到要说是哪种方式，别的两步不能带；多带别的字段、不认识的步骤一律不收。
        for json in [
            r#"{"step":"invite_copied","surface":"web"}"#,
            r#"{"step":"invite_opened","method":"mcp","surface":"web"}"#,
        ] {
            assert!(
                onboarding(json).is_some_and(|request| !request.is_consistent()),
                "{json}"
            );
        }
        for json in [
            r#"{"step":"invite_opened","surface":"web","accountId":"a"}"#,
            r#"{"step":"signed_up","surface":"web"}"#,
            r#"{"step":"invite_copied","method":"ssh","surface":"web"}"#,
        ] {
            assert!(onboarding(json).is_none(), "{json}");
        }
    }

    #[test]
    fn 每种接入步骤的序列都先报好() {
        for (step, method) in ONBOARDING_SERIES {
            assert_eq!(
                step.takes_method(),
                method != NO_METHOD,
                "{}",
                step.as_str()
            );
        }
    }
}
