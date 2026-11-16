//! 提醒（`specs/redesign/reminders.md`）：邮件提醒的开关和退订页。这一步只存开关，还不发信。
//!
//! - `GET /account/reminders`：`{ email: { enabled, available } }`。`available` 是这台服务器开没开
//!   邮件提醒；没开时网页不显示这个开关。
//! - `PUT /account/reminders`：带 `{ email: { enabled } }`，回答同上。有效登录就行，不要最近登录过：
//!   和“退出别处”一样，是管自己的事。
//! - `GET /reminders/unsubscribe?token=…`：邮件里的退订链接打开的一页，上面一个按钮。只看不改：
//!   邮箱的链接扫描会先打开一遍。
//! - `POST /reminders/unsubscribe`：那一页的表单，也收邮箱客户端的一键退订（RFC 8058，令牌在地址里，
//!   正文是 `List-Unsubscribe=One-Click`）。
//!
//! 退订页不要登录，按来源限流（[`AnonymousLimiter`]），样式用分享页那一份（`/r/_/share.css`）。
//! 地址里带着令牌，所以回答不缓存、不带 Referer。

use std::{collections::HashMap, sync::Arc};

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    reminders::{ReminderService, ReminderSettings, UnsubscribeFailure, UnsubscribeStanding},
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Form, Json, Router,
    extract::{
        DefaultBodyLimit, Extension, Query, State,
        rejection::{FormRejection, JsonRejection},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

mod page;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        anonymous_limits::AnonymousLimiter,
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        share_page::{Language, accepted_language},
    },
};

/// 开关的正文和退订页的表单都很小。
const MAX_BODY_BYTES: usize = 2 * 1_024;
/// 退订页自己的内容安全策略：只许同源的样式和同源的表单，不许脚本、不许被别的页面嵌进去。
const PAGE_POLICY: &str = "default-src 'none'; style-src 'self'; form-action 'self'; \
                           base-uri 'none'; frame-ancestors 'none'";

#[derive(Clone)]
pub(crate) struct RemindersHttpState {
    service: Arc<ReminderService>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
    limits: Arc<AnonymousLimiter>,
    /// 网页应用，页面上的按钮去它的设置页。
    app_origin: Arc<str>,
}

impl RemindersHttpState {
    pub(crate) fn new(
        service: Arc<ReminderService>,
        authentication: Arc<dyn AuthenticationUseCases>,
        limits: Arc<AnonymousLimiter>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            service,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
            limits,
            app_origin: frontend_origin.origin().ascii_serialization().into(),
        }
    }
}

pub(crate) fn router(state: RemindersHttpState) -> Router {
    Router::new()
        .route("/account/reminders", get(find_settings).put(store_settings))
        .route(
            "/reminders/unsubscribe",
            get(unsubscribe_page).post(unsubscribe),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResponse {
    email: EmailSettingsResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmailSettingsResponse {
    enabled: bool,
    available: bool,
}

impl From<ReminderSettings> for SettingsResponse {
    fn from(settings: ReminderSettings) -> Self {
        Self {
            email: EmailSettingsResponse {
                enabled: settings.email.enabled,
                available: settings.email.available,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsBody {
    email: EmailSettingsBody,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailSettingsBody {
    enabled: bool,
}

async fn find_settings(
    State(state): State<RemindersHttpState>,
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
    match state.service.settings(&actor).await {
        Ok(settings) => no_store(Json(SettingsResponse::from(settings)).into_response()),
        Err(_) => unavailable(correlation_id),
    }
}

async fn store_settings(
    State(state): State<RemindersHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<SettingsBody>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "account.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "reminders.invalid_request",
                ErrorCategory::Validation,
                "正文须为 { \"email\": { \"enabled\": true 或 false } }。",
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
    match state.service.set_email(&actor, body.email.enabled).await {
        Ok(settings) => no_store(Json(SettingsResponse::from(settings)).into_response()),
        Err(_) => unavailable(correlation_id),
    }
}

fn unavailable(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "reminders.unavailable",
            ErrorCategory::DependencyUnavailable,
            "提醒设置暂时读写不了，请稍后再试。",
            correlation_id,
        )
        .into_response(),
    )
}

async fn unsubscribe_page(
    State(state): State<RemindersHttpState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let language = accepted_language(&headers).unwrap_or(Language::English);
    if let Some(busy) = admit(&state, &headers, language) {
        return busy;
    }
    let Some(token) = query.get("token") else {
        return html(StatusCode::NOT_FOUND, page::invalid(language));
    };
    match state.service.unsubscribe_standing(token).await {
        Ok(UnsubscribeStanding::Subscribed) => html(
            StatusCode::OK,
            page::confirm(language, token, &state.app_origin),
        ),
        Ok(UnsubscribeStanding::Unsubscribed) => {
            html(StatusCode::OK, page::done(language, &state.app_origin))
        }
        Err(failure) => failed(failure, language),
    }
}

async fn unsubscribe(
    State(state): State<RemindersHttpState>,
    Query(mut query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    form: Result<Form<HashMap<String, String>>, FormRejection>,
) -> Response {
    let language = accepted_language(&headers).unwrap_or(Language::English);
    if let Some(busy) = admit(&state, &headers, language) {
        return busy;
    }
    // 页面上的表单把令牌放在正文里；一键退订照 RFC 8058 只发 POST，令牌在原来的地址里。
    let token = form
        .ok()
        .and_then(|Form(mut fields)| fields.remove("token"))
        .or_else(|| query.remove("token"));
    let Some(token) = token else {
        return html(StatusCode::NOT_FOUND, page::invalid(language));
    };
    match state.service.unsubscribe(&token).await {
        Ok(()) => html(StatusCode::OK, page::done(language, &state.app_origin)),
        Err(failure) => failed(failure, language),
    }
}

fn admit(state: &RemindersHttpState, headers: &HeaderMap, language: Language) -> Option<Response> {
    let limited = state.limits.admit(headers).err()?;
    let mut response = html(StatusCode::TOO_MANY_REQUESTS, page::busy(language));
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from(limited.retry_after_seconds),
    );
    Some(response)
}

fn failed(failure: UnsubscribeFailure, language: Language) -> Response {
    match failure {
        UnsubscribeFailure::InvalidToken => html(StatusCode::NOT_FOUND, page::invalid(language)),
        UnsubscribeFailure::Unavailable => {
            tracing::info!("退订页没读写成提醒设置");
            html(StatusCode::SERVICE_UNAVAILABLE, page::unavailable(language))
        }
    }
}

fn html(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::VARY, "Accept-Language"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CONTENT_SECURITY_POLICY, PAGE_POLICY),
        (HeaderName::from_static("x-robots-tag"), "noindex"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
#[path = "reminders/tests.rs"]
mod tests;
