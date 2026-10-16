//! 桌面端的聊天登录（`specs/desktop-sign-in/design.md`）：`POST /auth/desktop/matrix-session`。
//!
//! 只认桌面登录。控制面以应用服务的身份替这个人登录 Matrix，回答和 Matrix 登录接口同一个形状，
//! 桌面端拿到后和以前换完登录令牌一样存下、连上，不再开第二次浏览器、停在 Synapse 的确认页。

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases, desktop_matrix_sessions::DesktopMatrixSessionService,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{
        TrustedOrigins, authenticate_desktop_session, no_store, origin_matches,
    },
};

#[derive(Clone)]
pub(crate) struct DesktopMatrixSessionHttpState {
    service: Arc<DesktopMatrixSessionService>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl DesktopMatrixSessionHttpState {
    pub(crate) fn new(
        service: Arc<DesktopMatrixSessionService>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            service,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: DesktopMatrixSessionHttpState) -> Router {
    Router::new()
        .route("/auth/desktop/matrix-session", post(issue))
        .with_state(state)
}

/// Matrix 登录接口的回答形状（`snake_case`），桌面端拿它和以前换登录令牌得到的一样处理。
#[derive(Serialize)]
struct MatrixLoginResponse {
    user_id: String,
    device_id: String,
    access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

async fn issue(
    State(state): State<DesktopMatrixSessionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "matrix_session.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let actor =
        match authenticate_desktop_session(state.authentication.as_ref(), &jar, correlation_id)
            .await
        {
            Ok(actor) => actor,
            Err(response) => return response,
        };
    match state.service.issue(&actor).await {
        Ok(session) => no_store(
            Json(MatrixLoginResponse {
                user_id: session.metadata().user_id().as_str().to_owned(),
                device_id: session.metadata().device_id().as_str().to_owned(),
                access_token: session.access_token().expose().to_owned(),
                refresh_token: session
                    .refresh_token()
                    .map(|token| token.expose().to_owned()),
            })
            .into_response(),
        ),
        Err(failure) => {
            no_store(ApiError::desktop_matrix_session(failure, correlation_id).into_response())
        }
    }
}

#[cfg(test)]
#[path = "desktop_matrix_session/tests.rs"]
mod tests;
