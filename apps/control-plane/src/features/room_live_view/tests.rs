use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, RoomLiveViewRecord, SecretValue},
    room_live::{
        InspectLiveView, LiveViewCondition, LiveViewFailure, LiveViewFailureKind, LiveViewResult,
        LiveViewState, RoomLiveViewUseCases, UpdateLiveView,
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

use super::{RoomLiveViewHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const ENABLED_AT: i64 = 1_700_000_100_000;

/// 记下收到的请求；`failure` 设上时下一次调用回答它，不然开着的回开着、关的回关着。
#[derive(Default)]
struct FakeLiveViews {
    inspected: Mutex<Vec<InspectLiveView>>,
    updated: Mutex<Vec<UpdateLiveView>>,
    failure: Mutex<Option<LiveViewFailure>>,
}

impl FakeLiveViews {
    fn answer(&self, enabled: bool) -> LiveViewResult<LiveViewState> {
        self.failure.lock().unwrap().take().map_or_else(
            || {
                Ok(LiveViewState {
                    enabled: enabled.then(|| RoomLiveViewRecord {
                        enabled_by: actor().principal_id,
                        enabled_at: time(ENABLED_AT),
                    }),
                    unmet: if enabled {
                        Vec::new()
                    } else {
                        vec![LiveViewCondition::NotListed, LiveViewCondition::JoinMode]
                    },
                    can_change: true,
                })
            },
            Err,
        )
    }
}

impl RoomLiveViewUseCases for FakeLiveViews {
    fn inspect(&self, request: InspectLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>> {
        let answer = self.answer(true);
        self.inspected.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn update(&self, request: UpdateLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>> {
        let answer = self.answer(request.enabled);
        self.updated.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 房间里的人读得到开没开_放没放_差哪几样() {
    let live_views = Arc::new(FakeLiveViews::default());
    let response = test_router(live_views.clone())
        .oneshot(request(Method::GET, None, false))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "enabled": true,
            "enabledAtUnixMs": ENABLED_AT,
            "live": true,
            "unmetConditions": [],
            "canChange": true,
        })
    );
    let inspected = live_views.inspected.lock().unwrap();
    assert_eq!(inspected[0].catalog_id, catalog());
    assert_eq!(inspected[0].actor.principal_id, actor().principal_id);
}

#[tokio::test]
async fn 开关照正文的开关来_关着的写出差哪几样() {
    let live_views = Arc::new(FakeLiveViews::default());

    let response = test_router(live_views.clone())
        .oneshot(request(Method::PUT, Some(json!({"enabled": false})), true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "enabled": false,
            "enabledAtUnixMs": null,
            "live": false,
            "unmetConditions": ["not_listed", "join_mode"],
            "canChange": true,
        })
    );
    let updated = live_views.updated.lock().unwrap();
    assert!(!updated[0].enabled);
    assert_eq!(updated[0].catalog_id, catalog());
}

#[tokio::test]
async fn 开不了时回409_说是哪一样() {
    let live_views = Arc::new(FakeLiveViews::default());
    *live_views.failure.lock().unwrap() = Some(LiveViewFailure::new(
        "room_live.update",
        LiveViewFailureKind::NotEligible(LiveViewCondition::JoinMode),
    ));

    let response = test_router(live_views)
        .oneshot(request(Method::PUT, Some(json!({"enabled": true})), true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response_json(response).await;
    assert_eq!(body["code"], "room_live.not_eligible");
    assert_eq!(body["details"]["condition"], "join_mode");
    assert!(body["message"].as_str().unwrap().contains("直接进"));
}

#[tokio::test]
async fn 别的错误各有各的码() {
    for (kind, status, code) in [
        (
            LiveViewFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_live.forbidden",
        ),
        (
            LiveViewFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_live.not_found",
        ),
        (
            LiveViewFailureKind::Conflict,
            StatusCode::CONFLICT,
            "room_live.conflict",
        ),
        (
            LiveViewFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_live.dependency_unavailable",
        ),
        (
            LiveViewFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_live.internal",
        ),
    ] {
        let live_views = Arc::new(FakeLiveViews::default());
        *live_views.failure.lock().unwrap() = Some(LiveViewFailure::new("room_live.update", kind));
        let response = test_router(live_views)
            .oneshot(request(Method::PUT, Some(json!({"enabled": true})), true))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn 来源不对_正文不对_房间号不对都不去开关() {
    let live_views = Arc::new(FakeLiveViews::default());

    let wrong_origin = test_router(live_views.clone())
        .oneshot(request(Method::PUT, Some(json!({"enabled": true})), false))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(wrong_origin).await["code"],
        "room_live.invalid_origin"
    );

    for body in [json!({"enabled": "yes"}), json!({"enabled": true, "at": 1})] {
        let response = test_router(live_views.clone())
            .oneshot(request(Method::PUT, Some(body), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(response).await["code"], "room_live.invalid");
    }

    let wrong_room = Request::builder()
        .method(Method::GET)
        .uri("/private-rooms/not-a-room/live-view")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .body(Body::empty())
        .unwrap();
    let response = test_router(live_views.clone())
        .oneshot(wrong_room)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["code"],
        "room_live.invalid_resource"
    );

    assert!(live_views.updated.lock().unwrap().is_empty());
    assert!(live_views.inspected.lock().unwrap().is_empty());
}

fn test_router(live_views: Arc<FakeLiveViews>) -> Router {
    router(RoomLiveViewHttpState::new(
        live_views,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(method: Method, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/private-rooms/{CATALOG_UUID}/live-view"))
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
