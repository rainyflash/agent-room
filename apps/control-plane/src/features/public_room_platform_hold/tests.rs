use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, PublicRoomPlatformHold, SecretValue},
    private_rooms::{
        HoldPublicRoom, PlatformHoldFailure, PlatformHoldFailureKind, PlatformHoldResult,
        PublicRoomPlatformHoldUseCases, ReleasePublicRoom,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{PublicRoomPlatformHoldHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const HELD_AT: i64 = 1_700_000_100_000;

/// 记下拿掉和放回的请求；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeHolds {
    held: Mutex<Vec<HoldPublicRoom>>,
    released: Mutex<Vec<ReleasePublicRoom>>,
    failure: Mutex<Option<PlatformHoldFailure>>,
}

impl FakeHolds {
    fn answer<T>(&self, value: T) -> PlatformHoldResult<T> {
        self.failure.lock().unwrap().take().map_or(Ok(value), Err)
    }
}

impl PublicRoomPlatformHoldUseCases for FakeHolds {
    fn hold(
        &self,
        request: HoldPublicRoom,
    ) -> PortFuture<'_, PlatformHoldResult<PublicRoomPlatformHold>> {
        let answer = self.answer(PublicRoomPlatformHold {
            catalog_id: request.catalog_id,
            held_by: request.actor.principal_id,
            reason: request.reason.trim().to_owned(),
            held_at: time(HELD_AT),
        });
        self.held.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn release(&self, request: ReleasePublicRoom) -> PortFuture<'_, PlatformHoldResult<()>> {
        let answer = self.answer(());
        self.released.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 拿掉回原因和时间_不回是谁拿掉的() {
    let holds = Arc::new(FakeHolds::default());
    let app = test_router(holds.clone());

    let response = app
        .oneshot(request(
            Method::PUT,
            Some(json!({"reason": "  冒充官方客服  "})),
            true,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({"reason": "冒充官方客服", "heldAtUnixMs": HELD_AT})
    );
    let held = holds.held.lock().unwrap();
    assert_eq!(held[0].catalog_id, catalog());
    assert_eq!(held[0].actor.principal_id, actor().principal_id);
    assert_eq!(held[0].reason, "  冒充官方客服  ");
}

#[tokio::test]
async fn 放回去回_204() {
    let holds = Arc::new(FakeHolds::default());
    let app = test_router(holds.clone());

    let response = app
        .oneshot(request(Method::DELETE, None, true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(holds.released.lock().unwrap()[0].catalog_id, catalog());
}

#[tokio::test]
async fn 正文不对_来源不对的不办() {
    let holds = Arc::new(FakeHolds::default());
    let app = test_router(holds.clone());
    for (method, body, origin, status, code) in [
        (
            Method::PUT,
            Some(json!({"reason": "原因", "listed": false})),
            true,
            StatusCode::BAD_REQUEST,
            "public_room_hold.invalid",
        ),
        (
            Method::PUT,
            None,
            true,
            StatusCode::BAD_REQUEST,
            "public_room_hold.invalid",
        ),
        (
            Method::PUT,
            Some(json!({"reason": "原因"})),
            false,
            StatusCode::FORBIDDEN,
            "public_room_hold.invalid_origin",
        ),
        (
            Method::DELETE,
            None,
            false,
            StatusCode::FORBIDDEN,
            "public_room_hold.invalid_origin",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, body, origin))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(holds.held.lock().unwrap().is_empty());
    assert!(holds.released.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 失败照种类回答() {
    let holds = Arc::new(FakeHolds::default());
    let app = test_router(holds.clone());
    for (kind, status, code) in [
        (
            PlatformHoldFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "public_room_hold.invalid",
        ),
        (
            PlatformHoldFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "public_room_hold.forbidden",
        ),
        (
            PlatformHoldFailureKind::ReauthenticationRequired,
            StatusCode::UNAUTHORIZED,
            "authentication.reauthentication_required",
        ),
        (
            PlatformHoldFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "public_room_hold.not_found",
        ),
        (
            PlatformHoldFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "public_room_hold.dependency_unavailable",
        ),
    ] {
        for method in [Method::PUT, Method::DELETE] {
            *holds.failure.lock().unwrap() = Some(PlatformHoldFailure::new("test", kind));
            let body = (method == Method::PUT).then(|| json!({"reason": ""}));
            let response = app
                .clone()
                .oneshot(request(method.clone(), body, true))
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{method} {code}");
            assert_eq!(response_json(response).await["code"], code);
        }
    }
}

fn test_router(holds: Arc<FakeHolds>) -> Router {
    router(PublicRoomPlatformHoldHttpState::new(
        holds,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(method: Method, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!(
            "/private-rooms/{CATALOG_UUID}/listing/platform-hold"
        ))
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// 拿掉和放回都要最近登录过。
struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("不开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("不完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(requirement, AuthenticationRequirement::RecentAuthentication);
        Box::pin(async { Ok(actor()) })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不停用账户") })
    }
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).unwrap()),
        matrix_user_id: "@moderator:matrix.agent-room.test".to_owned(),
        display_name: "Platform Moderator".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: true,
    }
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
