use std::sync::Mutex;

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult, BeginLogin,
        CompleteLogin, LoginCompletion, LoginRedirect,
    },
    devices::{
        AuthenticatedDevice, DeviceAuthorizationResult, DeviceCredentials, IssuedComputerGrant,
    },
    ports::{PortFuture, PrincipalAccount, SecretValue},
};
use agent_room_domain::{
    devices::DevicePlatform,
    identity::Principal,
    ids::{DeviceId, PrincipalId},
    time::UtcMillis,
};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

const DESKTOP_COOKIE: &str = "__Secure-agent-room-desktop-session=session-secret";
const WEB_COOKIE: &str = "__Host-agent-room-session=session-secret";
const DESKTOP_ORIGIN: &str = "tauri://localhost";
const PRINCIPAL: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const DEVICE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";

#[derive(Default)]
struct FakeGrants {
    issued_for: Mutex<Vec<(PrincipalId, UtcMillis)>>,
    registrations: Mutex<Vec<RegisterDeviceWithGrant>>,
}

impl ComputerGrantUseCases for FakeGrants {
    fn issue_computer_grant<'a>(
        &'a self,
        principal: &'a AuthenticatedPrincipal,
    ) -> PortFuture<'a, DeviceAuthorizationResult<IssuedComputerGrant>> {
        self.issued_for
            .lock()
            .unwrap()
            .push((principal.principal_id, principal.authenticated_at));
        Box::pin(async {
            Ok(IssuedComputerGrant {
                grant: SecretValue::new("computer-grant").unwrap(),
                expires_at: time(1_790_000_600_000),
            })
        })
    }

    fn register_device_with_grant(
        &self,
        request: RegisterDeviceWithGrant,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        self.registrations.lock().unwrap().push(request);
        Box::pin(async move {
            Ok(DeviceCredentials {
                device: AuthenticatedDevice {
                    account: PrincipalAccount {
                        principal: Principal::new(principal_id()),
                        matrix_user_id: "@rainy:matrix.agent-room.test".to_owned(),
                        display_name: "Rainy".to_owned(),
                        avatar_content_id: None,
                        locale: "zh-CN".to_owned(),
                    },
                    device_id: DeviceId::from_uuid(Uuid::parse_str(DEVICE).unwrap()),
                    access_token_expires_at: time(1_790_000_900_000),
                },
                access_token: SecretValue::new("access-token").unwrap(),
                refresh_token: SecretValue::new("refresh-token").unwrap(),
                refresh_token_expires_at: time(1_792_592_000_000),
            })
        })
    }
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("电脑授权不会开始账户登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("电脑授权不会完成账户登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
        Box::pin(async {
            Ok(AuthenticatedPrincipal {
                principal_id: principal_id(),
                matrix_user_id: "@rainy:matrix.agent-room.test".to_owned(),
                display_name: "Rainy".to_owned(),
                locale: "zh-CN".to_owned(),
                authenticated_at: time(1_780_000_000_000),
                expires_at: time(1_792_592_000_000),
                recently_authenticated: false,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("电脑授权不会退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("电脑授权不会暂停主体") })
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn app(grants: Arc<FakeGrants>) -> Router {
    router(ComputerGrantHttpState::new(
        grants,
        Arc::new(FakeAuthentication),
        &url::Url::parse("https://app.agent-room.test").unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn issue_request(cookie: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/auth/desktop/computer-grant")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, origin)
        .body(Body::empty())
        .unwrap()
}

fn register_request(authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/auth/devices/register-with-grant")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }
    builder
        .body(Body::from(
            json!({
                "label": "Windows 工作站",
                "platform": "windows",
                "publicSigningKey": URL_SAFE_NO_PAD.encode([7_u8; 32]),
                "possessionSignature": URL_SAFE_NO_PAD.encode([9_u8; 64]),
                "importDisplayName": true,
                "importLocale": true
            })
            .to_string(),
        ))
        .unwrap()
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 桌面登录换来一次性的电脑授权_记着认证时刻_不缓存() {
    let grants = Arc::new(FakeGrants::default());
    let response = app(grants.clone())
        .oneshot(issue_request(DESKTOP_COOKIE, DESKTOP_ORIGIN))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        body_json(response).await,
        json!({ "grant": "computer-grant", "expiresAtUnixMs": 1_790_000_600_000_i64 })
    );
    assert_eq!(
        grants.issued_for.lock().unwrap().as_slice(),
        [(principal_id(), time(1_780_000_000_000))]
    );
}

#[tokio::test]
async fn 网页的登录换不来电脑授权_回_403() {
    let grants = Arc::new(FakeGrants::default());
    let response = app(grants.clone())
        .oneshot(issue_request(WEB_COOKIE, "https://app.agent-room.test"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "authentication.desktop_session_required"
    );
    assert!(grants.issued_for.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 别的来源要不到电脑授权() {
    let grants = Arc::new(FakeGrants::default());
    let response = app(grants.clone())
        .oneshot(issue_request(DESKTOP_COOKIE, "https://evil.example"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "device.computer_grant_invalid_origin"
    );
    assert!(grants.issued_for.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 拿电脑授权注册这台电脑_回答和设备码那条路一样() {
    let grants = Arc::new(FakeGrants::default());
    let response = app(grants.clone())
        .oneshot(register_request(Some("Bearer computer-grant")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let body = body_json(response).await;
    assert_eq!(body["deviceId"], DEVICE);
    assert_eq!(body["principalId"], PRINCIPAL);
    assert_eq!(body["accessToken"], "access-token");
    assert_eq!(body["refreshToken"], "refresh-token");
    assert_eq!(body["refreshTokenExpiresAtUnixMs"], 1_792_592_000_000_i64);
    let registrations = grants.registrations.lock().unwrap();
    let [registration] = registrations.as_slice() else {
        panic!("注册用例只调用一次");
    };
    assert_eq!(registration.grant.expose(), "computer-grant");
    assert_eq!(registration.label, "Windows 工作站");
    assert_eq!(registration.platform, DevicePlatform::Windows);
}

#[tokio::test]
async fn 没带电脑授权不注册() {
    let grants = Arc::new(FakeGrants::default());
    let response = app(grants.clone())
        .oneshot(register_request(None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(response).await["code"], "device.invalid_bearer");
    assert!(grants.registrations.lock().unwrap().is_empty());
}
