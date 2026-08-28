use std::{sync::Arc, time::Duration};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationFailureKind, AuthenticationIntent,
        AuthenticationRequirement, AuthenticationUseCases, BeginLogin, CompleteLogin,
    },
    ports::{ProfileImportConsent, SafeReturnPath, SecretValue},
};
use axum::{
    Json, Router,
    extract::{Extension, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Duration as CookieDuration;
use url::Url;

use crate::{correlation::CorrelationId, error::ApiError};

const LOGIN_COOKIE: &str = "__Host-agent-room-login";
const SESSION_COOKIE: &str = "__Host-agent-room-session";

#[derive(Clone)]
pub(crate) struct AuthenticationHttpState {
    authentication: Arc<dyn AuthenticationUseCases>,
    frontend_origin: Url,
    issuer: Url,
    serialized_frontend_origin: String,
    login_failure_redirect: String,
    login_cookie_ttl: CookieDuration,
    session_cookie_ttl: CookieDuration,
}

impl AuthenticationHttpState {
    /// 创建 HTTP 认证适配器状态。
    ///
    /// # Errors
    ///
    /// 前端地址不是纯 Origin，或 Cookie 生命周期无法安全转换时返回配置错误。
    pub(crate) fn new(
        authentication: Arc<dyn AuthenticationUseCases>,
        issuer: Url,
        frontend_origin: Url,
        login_cookie_ttl: Duration,
        session_cookie_ttl: Duration,
    ) -> Result<Self, AuthenticationHttpConfigurationError> {
        if frontend_origin.path() != "/"
            || frontend_origin.query().is_some()
            || frontend_origin.fragment().is_some()
        {
            return Err(AuthenticationHttpConfigurationError::InvalidFrontendOrigin);
        }
        let serialized_frontend_origin = frontend_origin.origin().ascii_serialization();
        let login_failure_redirect = frontend_origin
            .join("/connect")
            .map_err(|_| AuthenticationHttpConfigurationError::InvalidFrontendOrigin)?
            .to_string();
        let login_cookie_ttl = cookie_duration(login_cookie_ttl)?;
        let session_cookie_ttl = cookie_duration(session_cookie_ttl)?;
        Ok(Self {
            authentication,
            frontend_origin,
            issuer,
            serialized_frontend_origin,
            login_failure_redirect,
            login_cookie_ttl,
            session_cookie_ttl,
        })
    }
}

pub(crate) fn router(state: AuthenticationHttpState) -> Router {
    Router::new()
        .route("/auth/oidc/start", get(begin_login))
        .route("/auth/oidc/callback", get(complete_login))
        .route("/auth/session", get(current_session))
        .route("/auth/logout", post(logout))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeginLoginQuery {
    #[serde(default)]
    return_to: Option<String>,
    #[serde(default)]
    import_display_name: bool,
    #[serde(default)]
    import_locale: bool,
    #[serde(default)]
    intent: BeginLoginIntent,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BeginLoginIntent {
    #[default]
    SignIn,
    Register,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteLoginQuery {
    code: String,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default, rename = "session_state")]
    _session_state: Option<String>,
    state: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    principal_id: String,
    matrix_user_id: String,
    display_name: String,
    locale: String,
    authenticated_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    recently_authenticated: bool,
}

async fn begin_login(
    State(state): State<AuthenticationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    query: Result<Query<BeginLoginQuery>, QueryRejection>,
    jar: CookieJar,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(
            ApiError::invalid_request("authentication.invalid_login_query", correlation_id)
                .into_response(),
        );
    };
    let Ok(return_path) = SafeReturnPath::new(query.return_to.unwrap_or_else(|| "/".to_owned()))
    else {
        return no_store(
            ApiError::invalid_request("authentication.unsafe_return_path", correlation_id)
                .into_response(),
        );
    };
    let redirect = match state
        .authentication
        .begin_login(BeginLogin {
            return_path,
            profile_import: ProfileImportConsent {
                display_name: query.import_display_name,
                locale: query.import_locale,
            },
            intent: match query.intent {
                BeginLoginIntent::SignIn => AuthenticationIntent::SignIn,
                BeginLoginIntent::Register => AuthenticationIntent::Register,
            },
        })
        .await
    {
        Ok(redirect) => redirect,
        Err(failure) => {
            return no_store(ApiError::authentication(failure, correlation_id).into_response());
        }
    };
    let jar = jar.add(secure_cookie(
        LOGIN_COOKIE,
        redirect.browser_secret.expose(),
        state.login_cookie_ttl,
    ));
    no_store((jar, Redirect::to(&redirect.authorization_url)).into_response())
}

async fn complete_login(
    State(state): State<AuthenticationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    query: Result<Query<CompleteLoginQuery>, QueryRejection>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Ok(Query(query)) = query else {
        return login_failure(
            &state,
            &headers,
            jar,
            ApiError::invalid_request("authentication.invalid_callback_query", correlation_id),
        );
    };
    if query
        .iss
        .as_deref()
        .is_some_and(|issuer| issuer != state.issuer.as_str())
    {
        return login_failure(
            &state,
            &headers,
            jar,
            ApiError::invalid_request("authentication.issuer_mismatch", correlation_id),
        );
    }
    let Some(browser_secret) = jar
        .get(LOGIN_COOKIE)
        .and_then(|cookie| SecretValue::new(cookie.value()).ok())
    else {
        return login_failure(
            &state,
            &headers,
            jar,
            ApiError::invalid_request("authentication.missing_login_cookie", correlation_id),
        );
    };
    let completion = match state
        .authentication
        .complete_login(CompleteLogin {
            code: &query.code,
            returned_state: &query.state,
            browser_secret: &browser_secret,
        })
        .await
    {
        Ok(completion) => completion,
        Err(failure) => {
            return login_failure(
                &state,
                &headers,
                jar,
                ApiError::authentication(failure, correlation_id),
            );
        }
    };
    let Ok(destination) = state.frontend_origin.join(completion.return_path.as_str()) else {
        return login_failure(
            &state,
            &headers,
            jar,
            ApiError::invalid_request("authentication.unsafe_return_path", correlation_id),
        );
    };
    let jar = jar.add(expired_cookie(LOGIN_COOKIE)).add(secure_cookie(
        SESSION_COOKIE,
        completion.session_secret.expose(),
        state.session_cookie_ttl,
    ));
    no_store((jar, Redirect::to(destination.as_str())).into_response())
}

async fn current_session(
    State(state): State<AuthenticationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let session_secret = match session_secret(&jar) {
        Ok(secret) => secret,
        Err(MissingSession) => {
            return no_store(missing_session_error(correlation_id).into_response());
        }
    };
    match state
        .authentication
        .authenticate(&session_secret, AuthenticationRequirement::ActiveSession)
        .await
    {
        Ok(principal) => no_store(Json(SessionResponse::from(principal)).into_response()),
        Err(failure) => {
            let clear_cookie = matches!(
                failure.kind(),
                AuthenticationFailureKind::InvalidSession
                    | AuthenticationFailureKind::PrincipalSuspended
            );
            let error = ApiError::authentication(failure, correlation_id);
            if clear_cookie {
                no_store((expired_session_jar(jar), error).into_response())
            } else {
                no_store(error.into_response())
            }
        }
    }
}

async fn logout(
    State(state): State<AuthenticationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.serialized_frontend_origin) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "authentication.invalid_logout_origin",
                agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
                "注销请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    if let Ok(secret) = session_secret(&jar)
        && let Err(failure) = state.authentication.logout(&secret).await
    {
        return no_store(
            (
                expired_session_jar(jar),
                ApiError::authentication(failure, correlation_id),
            )
                .into_response(),
        );
    }
    no_store((expired_session_jar(jar), StatusCode::NO_CONTENT).into_response())
}

fn secure_cookie(name: &'static str, value: &str, max_age: CookieDuration) -> Cookie<'static> {
    Cookie::build((name, value.to_owned()))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .build()
}

fn expired_cookie(name: &'static str) -> Cookie<'static> {
    let mut cookie = Cookie::build((name, ""))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    cookie.make_removal();
    cookie
}

pub(crate) fn expired_session_jar(jar: CookieJar) -> CookieJar {
    jar.add(expired_cookie(SESSION_COOKIE))
}

fn login_failure(
    state: &AuthenticationHttpState,
    headers: &HeaderMap,
    jar: CookieJar,
    error: ApiError,
) -> Response {
    let jar = jar.add(expired_cookie(LOGIN_COOKIE));
    if accepts_html(headers) {
        tracing::warn!(
            correlation.id = error.correlation_id(),
            error.code = error.code(),
            "浏览器登录回调失败，返回连接页重新开始"
        );
        return no_store((jar, Redirect::to(&state.login_failure_redirect)).into_response());
    }
    no_store((jar, error).into_response())
}

fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            accept.split(',').any(|media_range| {
                media_range
                    .split(';')
                    .next()
                    .is_some_and(|media_type| media_type.trim() == "text/html")
            })
        })
}

pub(crate) fn session_secret(jar: &CookieJar) -> Result<SecretValue, MissingSession> {
    jar.get(SESSION_COOKIE)
        .and_then(|cookie| SecretValue::new(cookie.value()).ok())
        .ok_or(MissingSession)
}

pub(crate) async fn authenticate_session(
    authentication: &dyn AuthenticationUseCases,
    jar: &CookieJar,
    requirement: AuthenticationRequirement,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    let secret = session_secret(jar).map_err(|MissingSession| {
        no_store(missing_session_error(correlation_id).into_response())
    })?;
    authentication
        .authenticate(&secret, requirement)
        .await
        .map_err(|failure| {
            no_store(ApiError::authentication(failure, correlation_id).into_response())
        })
}

pub(crate) fn origin_matches(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
}

pub(crate) fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}

fn cookie_duration(
    duration: Duration,
) -> Result<CookieDuration, AuthenticationHttpConfigurationError> {
    let seconds = i64::try_from(duration.as_secs())
        .map_err(|_| AuthenticationHttpConfigurationError::InvalidCookieLifetime)?;
    if seconds == 0 {
        return Err(AuthenticationHttpConfigurationError::InvalidCookieLifetime);
    }
    Ok(CookieDuration::seconds(seconds))
}

impl From<AuthenticatedPrincipal> for SessionResponse {
    fn from(value: AuthenticatedPrincipal) -> Self {
        Self {
            principal_id: value.principal_id.to_string(),
            matrix_user_id: value.matrix_user_id,
            display_name: value.display_name,
            locale: value.locale,
            authenticated_at_unix_ms: value.authenticated_at.value(),
            expires_at_unix_ms: value.expires_at.value(),
            recently_authenticated: value.recently_authenticated,
        }
    }
}

pub(crate) struct MissingSession;

pub(crate) fn missing_session_error(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "authentication.invalid_session",
        agent_room_protocol_conformance::generated::ErrorCategory::Authentication,
        "会话无效或已过期。",
        correlation_id,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum AuthenticationHttpConfigurationError {
    #[error("前端 Origin 配置无效")]
    InvalidFrontendOrigin,
    #[error("Cookie 生命周期配置无效")]
    InvalidCookieLifetime,
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use agent_room_application::{
        authentication::{
            AuthenticatedPrincipal, AuthenticationIntent, AuthenticationRequirement,
            AuthenticationResult, AuthenticationUseCases, BeginLogin, CompleteLogin,
            LoginCompletion, LoginRedirect,
        },
        ports::{PortFuture, SafeReturnPath, SecretValue},
    };
    use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
    };
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::{AuthenticationHttpState, router};

    #[derive(Default)]
    struct FakeAuthentication {
        begin: AtomicUsize,
        register_begin: AtomicUsize,
        complete: AtomicUsize,
        logout: AtomicUsize,
    }

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            self.begin.fetch_add(1, Ordering::SeqCst);
            if request.intent == AuthenticationIntent::Register {
                self.register_begin.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async move {
                match request.intent {
                    AuthenticationIntent::SignIn => {
                        assert_eq!(request.return_path.as_str(), "/rooms/42?tab=chat");
                        assert!(request.profile_import.display_name);
                        assert!(!request.profile_import.locale);
                    }
                    AuthenticationIntent::Register => {
                        assert_eq!(request.return_path.as_str(), "/onboarding");
                    }
                }
                Ok(LoginRedirect {
                    authorization_url: "https://identity.example/authorize?state=opaque".to_owned(),
                    browser_secret: SecretValue::new("browser-secret").expect("测试密钥有效"),
                    expires_at: time(1_700_000_600_000),
                })
            })
        }

        fn complete_login<'a>(
            &'a self,
            request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            self.complete.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert_eq!(request.code, "authorization-code");
                assert_eq!(request.returned_state, "returned-state");
                assert_eq!(request.browser_secret.expose(), "browser-secret");
                Ok(LoginCompletion {
                    session_secret: SecretValue::new("session-secret").expect("测试密钥有效"),
                    return_path: SafeReturnPath::new("/rooms/42?tab=chat")
                        .expect("测试返回路径有效"),
                    principal: principal(),
                })
            })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            Box::pin(async move {
                assert_eq!(session_secret.expose(), "session-secret");
                assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
                Ok(principal())
            })
        }

        fn logout<'a>(
            &'a self,
            session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            self.logout.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert_eq!(session_secret.expose(), "session-secret");
                Ok(())
            })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn test_router(fake: Arc<FakeAuthentication>) -> axum::Router {
        let state = AuthenticationHttpState::new(
            fake,
            Url::parse("https://identity.example").expect("OIDC issuer 有效"),
            Url::parse("https://app.agent-room.test").expect("前端 Origin 有效"),
            Duration::from_mins(10),
            Duration::from_hours(8),
        )
        .expect("HTTP 认证配置有效");
        router(state).layer(middleware::from_fn(crate::correlation::attach))
    }

    fn principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: PrincipalId::from_uuid(
                Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e42").expect("UUID 有效"),
            ),
            matrix_user_id: "@user-0198b601:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            locale: "en".to_owned(),
            authenticated_at: time(1_700_000_000_000),
            expires_at: time(1_700_028_800_000),
            recently_authenticated: true,
        }
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }

    fn set_cookies(response: &axum::response::Response) -> Vec<String> {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("Cookie 头有效").to_owned())
            .collect()
    }

    #[tokio::test]
    async fn 登录起点签发完整安全属性的主机_cookie() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/start?returnTo=%2Frooms%2F42%3Ftab%3Dchat&importDisplayName=true")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION),
            Some(&header::HeaderValue::from_static(
                "https://identity.example/authorize?state=opaque"
            ))
        );
        let cookies = set_cookies(&response);
        assert_eq!(cookies.len(), 1);
        let cookie = &cookies[0];
        assert!(cookie.starts_with("__Host-agent-room-login=browser-secret"));
        for attribute in [
            "HttpOnly",
            "SameSite=Lax",
            "Secure",
            "Path=/",
            "Max-Age=600",
        ] {
            assert!(cookie.contains(attribute), "缺少 Cookie 属性：{attribute}");
        }
        assert_eq!(fake.begin.load(Ordering::SeqCst), 1);
        assert_eq!(fake.register_begin.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn 注册起点显式传递创建账户意图() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/start?returnTo=%2Fonboarding&intent=register")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(fake.begin.load(Ordering::SeqCst), 1);
        assert_eq!(fake.register_begin.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn 登录起点在进入用例前拒绝开放重定向() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/start?returnTo=%2F%2Fevil.example%2Fsteal")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(fake.begin.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn 回调轮换登录_cookie_为安全会话_cookie_并固定前端源() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/callback?code=authorization-code&state=returned-state&iss=https%3A%2F%2Fidentity.example%2F&session_state=opaque")
                    .header(header::COOKIE, "__Host-agent-room-login=browser-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("必须返回跳转地址")
                .to_str()
                .expect("跳转地址有效"),
            "https://app.agent-room.test/rooms/42?tab=chat"
        );
        let cookies = set_cookies(&response);
        assert_eq!(cookies.len(), 2);
        assert!(cookies.iter().any(|cookie| {
            cookie.starts_with("__Host-agent-room-login=")
                && cookie.contains("Max-Age=0")
                && cookie.contains("Path=/")
        }));
        assert!(cookies.iter().any(|cookie| {
            cookie.starts_with("__Host-agent-room-session=session-secret")
                && cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Lax")
                && cookie.contains("Secure")
                && cookie.contains("Max-Age=28800")
        }));
        assert_eq!(fake.complete.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn 回调显式拒绝不匹配的授权服务器_issuer() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/callback?code=authorization-code&state=returned-state&iss=https%3A%2F%2Fevil.example")
                    .header(header::COOKIE, "__Host-agent-room-login=browser-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(fake.complete.load(Ordering::SeqCst), 0);
        assert!(
            set_cookies(&response)
                .iter()
                .any(|cookie| cookie.contains("Max-Age=0"))
        );
    }

    #[tokio::test]
    async fn 回调缺少浏览器绑定_cookie_时拒绝登录并清理残留() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/callback?code=authorization-code&state=returned-state")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(fake.complete.load(Ordering::SeqCst), 0);
        assert!(
            set_cookies(&response)
                .iter()
                .any(|cookie| cookie.contains("Max-Age=0"))
        );
    }

    #[tokio::test]
    async fn 浏览器回调缺少登录_cookie_时返回连接页重新开始() {
        let fake = Arc::new(FakeAuthentication::default());
        let response = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .uri("/auth/oidc/callback?code=authorization-code&state=returned-state")
                    .header(header::ACCEPT, "text/html,application/xhtml+xml")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION),
            Some(&header::HeaderValue::from_static(
                "https://app.agent-room.test/connect"
            ))
        );
        assert_eq!(fake.complete.load(Ordering::SeqCst), 0);
        assert!(
            set_cookies(&response)
                .iter()
                .any(|cookie| cookie.contains("Max-Age=0"))
        );
    }

    #[tokio::test]
    async fn 会话端点只从_http_only_cookie_认证() {
        let response = test_router(Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .uri("/auth/session")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let body = to_bytes(response.into_body(), 32 * 1_024)
            .await
            .expect("响应正文可读");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("响应是 JSON");
        assert_eq!(json["principalId"], "0198b601-77a1-7bb8-83eb-a8fe68c97e42");
        assert_eq!(json["recentlyAuthenticated"], true);
    }

    #[tokio::test]
    async fn 注销要求精确前端_origin_并清除会话_cookie() {
        let fake = Arc::new(FakeAuthentication::default());
        let wrong_origin = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
        assert_eq!(fake.logout.load(Ordering::SeqCst), 0);

        let valid_origin = test_router(fake.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::ORIGIN, "https://app.agent-room.test")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(valid_origin.status(), StatusCode::NO_CONTENT);
        assert_eq!(fake.logout.load(Ordering::SeqCst), 1);
        assert!(
            set_cookies(&valid_origin)
                .iter()
                .any(|cookie| cookie.starts_with("__Host-agent-room-session=")
                    && cookie.contains("Max-Age=0"))
        );
    }
}
