use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationFailure, AuthenticationFailureKind,
        AuthenticationRequirement, AuthenticationResult, BeginLogin, CompleteLogin,
        LoginCompletion, LoginRedirect,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixDeviceId, MatrixFailure, MatrixFailureKind, MatrixHumanDeviceSignOut,
        MatrixOperation, MatrixResult, MatrixUserId, OidcResult, OtherSessionsRevoked, PortFuture,
        SecretDigest, SecretFactory, SecretGenerationFailure, SecretValue, SignInAccount,
        SignInSessionTermination, StoredWebSession, WebSessionStore,
    },
    sign_out_elsewhere::{SignOutElsewhereDependencies, SignOutElsewhereService},
};
use agent_room_domain::{
    ids::{PrincipalId, WebSessionId},
    time::UtcMillis,
};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

const WEB_COOKIE: &str = "__Host-agent-room-session=session-secret";
const DESKTOP_COOKIE: &str = "__Secure-agent-room-desktop-session=session-secret";
const WEB_ORIGIN: &str = "https://app.agent-room.test";
const DESKTOP_ORIGIN: &str = "tauri://localhost";
const RAINY: &str = "@rainy:matrix.agent-room.test";

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("退出别的地方不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("退出别的地方不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(
            requirement,
            AuthenticationRequirement::ActiveSession,
            "改密码的页面上停久了也退得了"
        );
        let valid = session_secret.expose() == "session-secret";
        Box::pin(async move {
            if !valid {
                return Err(AuthenticationFailure::new(
                    "authentication.authenticate",
                    AuthenticationFailureKind::InvalidSession,
                ));
            }
            Ok(AuthenticatedPrincipal {
                principal_id: principal_id(),
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
        Box::pin(async { unreachable!("退出别的地方不退这一份") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("退出别的地方不会暂停主体") })
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e42").unwrap())
}

#[derive(Default)]
struct Sessions {
    revoked_for: Mutex<Vec<(PrincipalId, Option<SecretDigest>)>>,
}

impl WebSessionStore for Sessions {
    fn find_active<'a>(
        &'a self,
        _secret_digest: &'a SecretDigest,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        unreachable!("认证由认证用例做")
    }

    fn revoke<'a>(
        &'a self,
        _secret_digest: &'a SecretDigest,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<SignInAccount>>> {
        unreachable!("这一份不撤")
    }

    fn revoke_others<'a>(
        &'a self,
        principal_id: PrincipalId,
        keep: Option<&'a SecretDigest>,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<OtherSessionsRevoked>>> {
        self.revoked_for
            .lock()
            .unwrap()
            .push((principal_id, keep.copied()));
        Box::pin(async {
            Ok(Some(OtherSessionsRevoked {
                revoked: 3,
                account: SignInAccount {
                    issuer: "https://identity.agent-room.test/realms/agent-room".to_owned(),
                    subject: "subject".to_owned(),
                },
            }))
        })
    }

    fn extend(
        &self,
        _session_id: WebSessionId,
        _expires_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        unreachable!("认证由认证用例做")
    }
}

#[derive(Default)]
struct ChatDevices {
    kept: Mutex<Vec<Option<String>>>,
    unavailable: Mutex<bool>,
}

impl MatrixHumanDeviceSignOut for ChatDevices {
    fn sign_out_other_devices<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keep: Option<&'a MatrixDeviceId>,
    ) -> PortFuture<'a, MatrixResult<usize>> {
        assert_eq!(user_id.as_str(), RAINY);
        self.kept
            .lock()
            .unwrap()
            .push(keep.map(|device| device.as_str().to_owned()));
        let result = if *self.unavailable.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::SignOutHumanDevices,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(2)
        };
        Box::pin(async move { result })
    }
}

struct SignInSessions;

impl SignInSessionTermination for SignInSessions {
    fn end_sessions<'a>(&'a self, _account: &'a SignInAccount) -> PortFuture<'a, OidcResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

struct Secrets;

impl SecretFactory for Secrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        unreachable!("不发新秘密")
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (slot, byte) in digest.iter_mut().zip(value.bytes()) {
            *slot = byte;
        }
        SecretDigest::from_array(digest)
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000).unwrap()
    }
}

struct Harness {
    sessions: Arc<Sessions>,
    chat_devices: Arc<ChatDevices>,
    app: Router,
}

fn harness() -> Harness {
    let sessions = Arc::new(Sessions::default());
    let chat_devices = Arc::new(ChatDevices::default());
    let service = Arc::new(SignOutElsewhereService::new(SignOutElsewhereDependencies {
        sessions: sessions.clone(),
        chat_devices: chat_devices.clone(),
        sign_in_sessions: Arc::new(SignInSessions),
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
    }));
    let app = router(SignOutElsewhereHttpState::new(
        service,
        Arc::new(FakeAuthentication),
        &url::Url::parse(WEB_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach));
    Harness {
        sessions,
        chat_devices,
        app,
    }
}

fn request(cookie: Option<&str>, origin: &str, body: &Value) -> Request<Body> {
    let builder = Request::builder()
        .method(Method::POST)
        .uri("/auth/sign-out-elsewhere")
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json");
    let builder = match cookie {
        Some(cookie) => builder.header(header::COOKIE, cookie),
        None => builder,
    };
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 网页登录退出别的地方_留着这一份和这一台的聊天设备_不缓存() {
    let harness = harness();
    let response = harness
        .app
        .oneshot(request(
            Some(WEB_COOKIE),
            WEB_ORIGIN,
            &json!({ "keepChatDeviceId": "THISBROWSER" }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        body_json(response).await,
        json!({ "accountSessions": 3, "chatDevices": 2 })
    );
    assert_eq!(
        *harness.sessions.revoked_for.lock().unwrap(),
        [(principal_id(), Some(Secrets.digest("session-secret")))]
    );
    assert_eq!(
        *harness.chat_devices.kept.lock().unwrap(),
        [Some("THISBROWSER".to_owned())]
    );
}

#[tokio::test]
async fn 桌面端的登录也认_还没连上聊天时不带设备号() {
    let harness = harness();
    let response = harness
        .app
        .oneshot(request(Some(DESKTOP_COOKIE), DESKTOP_ORIGIN, &json!({})))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(*harness.chat_devices.kept.lock().unwrap(), [None]);
}

#[tokio::test]
async fn 别的来源不给() {
    let harness = harness();
    let response = harness
        .app
        .oneshot(request(
            Some(WEB_COOKIE),
            "https://evil.example",
            &json!({}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "sign_out_elsewhere.invalid_origin"
    );
    assert!(harness.sessions.revoked_for.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 没登录或者登录失效回_401() {
    for cookie in [None, Some("__Host-agent-room-session=stale-secret")] {
        let harness = harness();
        let response = harness
            .app
            .oneshot(request(cookie, WEB_ORIGIN, &json!({})))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(response).await["code"],
            "authentication.invalid_session"
        );
        assert!(harness.sessions.revoked_for.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 设备号不对或者多了字段回_400_什么都不退() {
    for body in [
        json!({ "keepChatDeviceId": "" }),
        json!({ "keepChatDeviceId": "THISBROWSER", "everything": true }),
    ] {
        let harness = harness();
        let response = harness
            .app
            .oneshot(request(Some(WEB_COOKIE), WEB_ORIGIN, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body_json(response).await["code"],
            "sign_out_elsewhere.invalid_request"
        );
        assert!(harness.sessions.revoked_for.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 聊天服务器不通时回_503_账户登录已经撤了() {
    let harness = harness();
    *harness.chat_devices.unavailable.lock().unwrap() = true;
    let response = harness
        .app
        .oneshot(request(Some(WEB_COOKIE), WEB_ORIGIN, &json!({})))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_json(response).await["code"],
        "sign_out_elsewhere.chat_unavailable"
    );
    assert_eq!(harness.sessions.revoked_for.lock().unwrap().len(), 1);
}
