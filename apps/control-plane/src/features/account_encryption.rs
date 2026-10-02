//! 人的设备自动签名（ADR 0011，`specs/device-signing/design.md`）：控制面替账户保管 Matrix 密钥存储
//! 的钥匙，任何设备登录后凭它签好自己。只经本人的登录会话存取；改动类的请求要校验来源。
//!
//! - `GET /account/encryption-key`：取钥匙；还没有就是 404 `account.encryption_key_missing`。
//! - `PUT /account/encryption-key`：存下或覆盖钥匙。
//! - `POST /account/encryption-reset`：重建签名身份时，替本人上传新的签名公钥（正文就是 Matrix
//!   `keys/device_signing/upload` 的正文，不带交互认证）。没接 MAS 的 Synapse 只认应用服务的
//!   免认证上传，所以由控制面以应用服务的身份代传。

use std::sync::Arc;

use agent_room_application::{
    account_encryption::{AccountEncryptionFailure, AccountEncryptionService, EncryptionKeyBytes},
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
};

/// 钥匙 ID 最长 255 字节，钥匙编码后 44 字节：1 KiB 足够。
const MAX_BODY_BYTES: usize = 1_024;
/// 三把签名公钥连同签名不到 2 KiB，留足余量。
const MAX_SIGNING_KEYS_BYTES: usize = 16 * 1_024;
const SCHEMA_VERSION: u8 = 1;

#[derive(Clone)]
pub(crate) struct AccountEncryptionHttpState {
    service: Arc<AccountEncryptionService>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl AccountEncryptionHttpState {
    pub(crate) fn new(
        service: Arc<AccountEncryptionService>,
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

pub(crate) fn router(state: AccountEncryptionHttpState) -> Router {
    Router::new()
        .route("/account/encryption-key", get(find_key).put(store_key))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .merge(
            Router::new()
                .route(
                    "/account/encryption-reset",
                    post(replace_cross_signing_keys),
                )
                .layer(DefaultBodyLimit::max(MAX_SIGNING_KEYS_BYTES)),
        )
        .with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyResponse {
    schema_version: u8,
    key_id: String,
    /// 标准 base64（带填充）。
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeyBody {
    key_id: String,
    key: String,
}

async fn find_key(
    State(state): State<AccountEncryptionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
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
    match state.service.find(&actor).await {
        Ok(Some(key)) => no_store(
            Json(KeyResponse {
                schema_version: SCHEMA_VERSION,
                key: STANDARD.encode(key.key.expose()),
                key_id: key.key_id,
            })
            .into_response(),
        ),
        Ok(None) => {
            no_store(ApiError::account_encryption_key_missing(correlation_id).into_response())
        }
        Err(failure) => {
            no_store(ApiError::account_encryption(failure, correlation_id).into_response())
        }
    }
}

async fn store_key(
    State(state): State<AccountEncryptionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<KeyBody>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return invalid_origin(correlation_id);
    }
    let Ok(Json(body)) = body else {
        return invalid_key(correlation_id);
    };
    let Ok(bytes) = STANDARD.decode(body.key.as_bytes()) else {
        return invalid_key(correlation_id);
    };
    let key = EncryptionKeyBytes::new(bytes);
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
    match state.service.store(&actor, body.key_id, &key).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => {
            no_store(ApiError::account_encryption(failure, correlation_id).into_response())
        }
    }
}

async fn replace_cross_signing_keys(
    State(state): State<AccountEncryptionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return invalid_origin(correlation_id);
    }
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::account_encryption(
                AccountEncryptionFailure::InvalidCrossSigningKeys,
                correlation_id,
            )
            .into_response(),
        );
    };
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
    match state.service.replace_cross_signing_keys(&actor, body).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => {
            no_store(ApiError::account_encryption(failure, correlation_id).into_response())
        }
    }
}

fn invalid_origin(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::FORBIDDEN,
            "account.invalid_origin",
            ErrorCategory::Authorization,
            "请求来源无效。",
            correlation_id,
        )
        .into_response(),
    )
}

fn invalid_key(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::account_encryption(AccountEncryptionFailure::InvalidKey, correlation_id)
            .into_response(),
    )
}

#[cfg(test)]
#[path = "account_encryption/tests.rs"]
mod tests;
