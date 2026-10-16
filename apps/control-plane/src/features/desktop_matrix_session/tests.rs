use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult, BeginLogin,
        CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{
        Clock, MatrixDeviceId, MatrixFailure, MatrixFailureKind, MatrixHumanSessionIssuer,
        MatrixOperation, MatrixResult, MatrixSession, MatrixSessionMetadata, MatrixUserId,
        PortFuture, SecretValue,
    },
};
use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

const DESKTOP_COOKIE: &str = "__Secure-agent-room-desktop-session=session-secret";
const WEB_COOKIE: &str = "__Host-agent-room-session=session-secret";
const DESKTOP_ORIGIN: &str = "tauri://localhost";
const RAINY: &str = "@rainy:matrix.agent-room.test";

#[derive(Default)]
struct Synapse {
    calls: Mutex<Vec<(String, String)>>,
    unavailable: Mutex<bool>,
}

impl MatrixHumanSessionIssuer for Synapse {
    fn issue_human_session<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        display_name: &'a str,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
        self.calls
            .lock()
            .unwrap()
            .push((user_id.as_str().to_owned(), display_name.to_owned()));
        let result = if *self.unavailable.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::IssueHumanSession,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(MatrixSession::new(
                MatrixSessionMetadata::new(
                    user_id.clone(),
                    MatrixDeviceId::new("DESKTOPDEV").unwrap(),
                ),
                SecretValue::new("syt_access".to_owned()).unwrap(),
                Some(SecretValue::new("syr_refresh".to_owned()).unwrap()),
            ))
        };
        Box::pin(async move { result })
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000).unwrap()
    }
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("聊天登录不会开始账户登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("聊天登录不会完成账户登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(
            requirement,
            AuthenticationRequirement::ActiveSession,
            "聊天登录丢了时也用它重新拿，不要求刚登录过"
        );
        Box::pin(async {
            Ok(AuthenticatedPrincipal {
                principal_id: PrincipalId::from_uuid(
                    Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e42").unwrap(),
                ),
                matrix_user_id: RAINY.to_owned(),
                display_name: "Rainy".to_owned(),
                locale: "zh-CN".to_owned(),
                authenticated_at: UtcMillis::new(1_790_000_000_000).unwrap(),
                expires_at: UtcMillis::new(1_790_000_060_000).unwrap(),
                recently_authenticated: false,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("聊天登录不会退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("聊天登录不会暂停主体") })
    }
}

fn app(synapse: Arc<Synapse>) -> Router {
    router(DesktopMatrixSessionHttpState::new(
        Arc::new(DesktopMatrixSessionService::new(
            synapse,
            Arc::new(FixedClock),
        )),
        Arc::new(FakeAuthentication),
        &url::Url::parse("https://app.agent-room.test").unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(cookie: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/auth/desktop/matrix-session")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, origin)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 桌面登录换来本人的聊天登录_形状和_matrix_登录一样_不缓存() {
    let synapse = Arc::new(Synapse::default());
    let response = app(synapse.clone())
        .oneshot(request(DESKTOP_COOKIE, DESKTOP_ORIGIN))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        body_json(response).await,
        json!({
            "user_id": RAINY,
            "device_id": "DESKTOPDEV",
            "access_token": "syt_access",
            "refresh_token": "syr_refresh",
        })
    );
    assert_eq!(
        synapse.calls.lock().unwrap().as_slice(),
        [(RAINY.to_owned(), "Rainy".to_owned())]
    );
}

#[tokio::test]
async fn 网页的登录换不来_回_403_免得桌面端当成登录失效() {
    let synapse = Arc::new(Synapse::default());
    let response = app(synapse.clone())
        .oneshot(request(WEB_COOKIE, "https://app.agent-room.test"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "authentication.desktop_session_required"
    );
    assert!(synapse.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 别的来源不给() {
    let synapse = Arc::new(Synapse::default());
    let response = app(synapse.clone())
        .oneshot(request(DESKTOP_COOKIE, "https://evil.example"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "matrix_session.invalid_origin"
    );
    assert!(synapse.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn synapse_不可用时回_503() {
    let synapse = Arc::new(Synapse::default());
    *synapse.unavailable.lock().unwrap() = true;
    let response = app(synapse)
        .oneshot(request(DESKTOP_COOKIE, DESKTOP_ORIGIN))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_json(response).await["code"],
        "matrix_session.unavailable"
    );
}
