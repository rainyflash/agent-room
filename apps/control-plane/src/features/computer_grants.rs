//! 这台电脑的授权跟着账户登录走（`specs/desktop-sign-in/design.md`）。
//!
//! - `POST /auth/desktop/computer-grant`：只认桌面登录，回一份一次性的电脑授权。原生层自己要、
//!   自己交给 Bridge，代发通道不转发这个地址，网页里的代码拿不到它。
//! - `POST /auth/devices/register-with-grant`：Bridge 拿电脑授权注册这台电脑，请求体和
//!   `/auth/devices/register` 一样，回答也一样。

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases,
    devices::{ComputerGrantUseCases, RegisterDeviceWithGrant},
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_desktop_session, no_store, origin_matches},
        devices::{
            DeviceCredentialsResponse, MAX_DEVICE_BODY_BYTES, RegisterDeviceBody, bearer_secret,
            invalid_bearer, registration_fields,
        },
    },
};

#[derive(Clone)]
pub(crate) struct ComputerGrantHttpState {
    grants: Arc<dyn ComputerGrantUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl ComputerGrantHttpState {
    pub(crate) fn new(
        grants: Arc<dyn ComputerGrantUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            grants,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: ComputerGrantHttpState) -> Router {
    Router::new()
        .route("/auth/desktop/computer-grant", post(issue))
        .route("/auth/devices/register-with-grant", post(register))
        .layer(DefaultBodyLimit::max(MAX_DEVICE_BODY_BYTES))
        .with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ComputerGrantResponse {
    grant: String,
    expires_at_unix_ms: i64,
}

async fn issue(
    State(state): State<ComputerGrantHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "device.computer_grant_invalid_origin",
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
    match state.grants.issue_computer_grant(&actor).await {
        Ok(issued) => no_store(
            Json(ComputerGrantResponse {
                grant: issued.grant.expose().to_owned(),
                expires_at_unix_ms: issued.expires_at.value(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

async fn register(
    State(state): State<ComputerGrantHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<RegisterDeviceBody>, JsonRejection>,
) -> Response {
    let Ok(grant) = bearer_secret(&headers) else {
        return no_store(invalid_bearer(correlation_id).into_response());
    };
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::invalid_request("device.invalid_registration_body", correlation_id)
                .into_response(),
        );
    };
    let fields = match registration_fields(body, correlation_id) {
        Ok(fields) => fields,
        Err(error) => return no_store(error.into_response()),
    };
    match state
        .grants
        .register_device_with_grant(RegisterDeviceWithGrant {
            grant,
            label: fields.label,
            platform: fields.platform,
            public_signing_key: fields.public_signing_key,
            possession_signature: fields.possession_signature,
        })
        .await
    {
        Ok(credentials) => {
            no_store(Json(DeviceCredentialsResponse::from(credentials)).into_response())
        }
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

#[cfg(test)]
#[path = "computer_grants/tests.rs"]
mod tests;
