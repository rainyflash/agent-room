use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    private_rooms::{
        PinsFailure, PinsFailureKind, PinsResult, PrivateRoomPinsUseCases, RoomPinView,
        UpdateRoomPins,
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

use super::{PrivateRoomPinsHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const PINNED_AT: i64 = 1_700_000_100_000;

/// 置顶的那一句放在最前面交回；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakePins {
    updated: Mutex<Vec<UpdateRoomPins>>,
    failure: Mutex<Option<PinsFailure>>,
}

impl PrivateRoomPinsUseCases for FakePins {
    fn update_pins(&self, request: UpdateRoomPins) -> PortFuture<'_, PinsResult<Vec<RoomPinView>>> {
        let answer = self.failure.lock().unwrap().take().map_or_else(
            || {
                let mut pinned = vec![RoomPinView {
                    event_id: "$older".to_owned(),
                    pinned_by: "@grace:matrix.agent-room.test".to_owned(),
                    pinned_at: time(PINNED_AT - 60_000),
                }];
                if request.pinned {
                    pinned.insert(
                        0,
                        RoomPinView {
                            event_id: request.event_id.clone(),
                            pinned_by: request.actor.matrix_user_id.clone(),
                            pinned_at: time(PINNED_AT),
                        },
                    );
                }
                Ok(pinned)
            },
            Err,
        );
        self.updated.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 置顶一句_回整份列表_新的在最前面() {
    let pins = Arc::new(FakePins::default());
    let app = test_router(pins.clone());

    let response = app
        .oneshot(request(
            Some(json!({"eventId": "$plan:matrix.agent-room.test", "pinned": true})),
            true,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({"pinned": [
            {
                "eventId": "$plan:matrix.agent-room.test",
                "pinnedBy": "@user:matrix.agent-room.test",
                "pinnedAtUnixMs": PINNED_AT,
            },
            {
                "eventId": "$older",
                "pinnedBy": "@grace:matrix.agent-room.test",
                "pinnedAtUnixMs": PINNED_AT - 60_000,
            },
        ]})
    );
    let updated = pins.updated.lock().unwrap();
    assert_eq!(updated[0].catalog_id, catalog());
    assert_eq!(updated[0].actor.principal_id, actor().principal_id);
    assert_eq!(updated[0].event_id, "$plan:matrix.agent-room.test");
    assert!(updated[0].pinned);
}

#[tokio::test]
async fn 正文不对_来源不对的不改() {
    let pins = Arc::new(FakePins::default());
    let app = test_router(pins.clone());
    for (body, origin, status, code) in [
        (
            Some(json!({"eventId": "$a", "pinned": true, "text": "多一个字段"})),
            true,
            StatusCode::BAD_REQUEST,
            "room_pins.invalid",
        ),
        (
            Some(json!({"eventId": "$a"})),
            true,
            StatusCode::BAD_REQUEST,
            "room_pins.invalid",
        ),
        (
            Some(json!({"eventId": "$a", "pinned": "yes"})),
            true,
            StatusCode::BAD_REQUEST,
            "room_pins.invalid",
        ),
        (None, true, StatusCode::BAD_REQUEST, "room_pins.invalid"),
        (
            Some(json!({"eventId": "$a", "pinned": false})),
            false,
            StatusCode::FORBIDDEN,
            "room_pins.invalid_origin",
        ),
    ] {
        let response = app.clone().oneshot(request(body, origin)).await.unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(pins.updated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 失败照种类回答() {
    let pins = Arc::new(FakePins::default());
    let app = test_router(pins.clone());
    for (kind, status, code) in [
        (
            PinsFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "room_pins.invalid",
        ),
        (
            PinsFailureKind::InvalidEvent,
            StatusCode::BAD_REQUEST,
            "room_pins.not_a_message",
        ),
        (
            PinsFailureKind::Full,
            StatusCode::CONFLICT,
            "room_pins.full",
        ),
        (
            PinsFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_pins.forbidden",
        ),
        (
            PinsFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_pins.not_found",
        ),
        (
            PinsFailureKind::Conflict,
            StatusCode::CONFLICT,
            "room_pins.conflict",
        ),
        (
            PinsFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_pins.dependency_unavailable",
        ),
        (
            PinsFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_pins.internal",
        ),
    ] {
        *pins.failure.lock().unwrap() = Some(PinsFailure::new("test", kind));
        let response = app
            .clone()
            .oneshot(request(
                Some(json!({"eventId": "$a", "pinned": true})),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn 改得太勤回_429_带_retry_after() {
    let pins = Arc::new(FakePins::default());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let retry_at = i64::try_from(now).unwrap() + 120_000;
    *pins.failure.lock().unwrap() = Some(PinsFailure::new(
        "test",
        PinsFailureKind::RateLimited {
            retry_at: time(retry_at),
        },
    ));
    let response = test_router(pins)
        .oneshot(request(
            Some(json!({"eventId": "$a", "pinned": true})),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((100..=120).contains(&retry_after), "{retry_after}");
    assert_eq!(
        response_json(response).await["code"],
        "room_pins.rate_limited"
    );
}

fn test_router(pins: Arc<FakePins>) -> Router {
    router(PrivateRoomPinsHttpState::new(
        pins,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri(format!("/private-rooms/{CATALOG_UUID}/pins"))
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
        assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
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
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: false,
    }
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
