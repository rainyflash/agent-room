//! 改好密码以后退出别的地方的登录（`specs/change-password/design.md`）：`POST /auth/sign-out-elsewhere`。
//!
//! 网页的、桌面端的登录都认，只要有效登录：退出别处是保护自己的动作，人在改密码的页面上停久了，
//! 回来这份登录就不算“最近登录过”，要求它的话又得登录一次。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::MatrixDeviceId,
    sign_out_elsewhere::SignOutElsewhereService,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{
        TrustedOrigins, authenticate_session, missing_session_error, no_store, origin_matches,
        session_secret,
    },
};

#[derive(Clone)]
pub(crate) struct SignOutElsewhereHttpState {
    service: Arc<SignOutElsewhereService>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl SignOutElsewhereHttpState {
    pub(crate) fn new(
        service: Arc<SignOutElsewhereService>,
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

pub(crate) fn router(state: SignOutElsewhereHttpState) -> Router {
    Router::new()
        .route("/auth/sign-out-elsewhere", post(sign_out_elsewhere))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignOutElsewhereRequest {
    /// 这台的聊天设备，留着；还没连上聊天就不带。
    #[serde(default)]
    keep_chat_device_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignOutElsewhereResponse {
    account_sessions: u64,
    chat_devices: usize,
}

async fn sign_out_elsewhere(
    State(state): State<SignOutElsewhereHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    request: Result<Json<SignOutElsewhereRequest>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "sign_out_elsewhere.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(secret) = session_secret(&jar) else {
        return no_store(missing_session_error(correlation_id).into_response());
    };
    let keep = match request {
        Ok(Json(request)) => match request.keep_chat_device_id.map(MatrixDeviceId::new) {
            None => None,
            Some(Ok(device)) => Some(device),
            Some(Err(_)) => return invalid_request(correlation_id),
        },
        Err(_) => return invalid_request(correlation_id),
    };
    match state.service.sign_out(&actor, &secret, keep.as_ref()).await {
        Ok(signed_out) => {
            if !signed_out.sign_in_sessions_ended {
                tracing::warn!(
                    correlation.id = %correlation_id.as_uuid(),
                    "退出别的地方时没能结束身份服务上的登录，别处还能不输密码再登录回来"
                );
            }
            tracing::info!(
                correlation.id = %correlation_id.as_uuid(),
                account_sessions = signed_out.account_sessions,
                chat_devices = signed_out.chat_devices,
                "退出了别的地方的登录"
            );
            no_store(
                Json(SignOutElsewhereResponse {
                    account_sessions: signed_out.account_sessions,
                    chat_devices: signed_out.chat_devices,
                })
                .into_response(),
            )
        }
        Err(failure) => {
            no_store(ApiError::sign_out_elsewhere(failure, correlation_id).into_response())
        }
    }
}

fn invalid_request(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::invalid_request("sign_out_elsewhere.invalid_request", correlation_id)
            .into_response(),
    )
}

#[cfg(test)]
#[path = "sign_out_elsewhere/tests.rs"]
mod tests;
