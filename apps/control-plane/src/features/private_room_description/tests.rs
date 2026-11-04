use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    private_rooms::{
        DescribePrivateRoom, DescriptionFailure, DescriptionFailureKind, DescriptionResult,
        PrivateRoomDescriptionUseCases,
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

use super::{PrivateRoomDescriptionHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 把简介去掉首尾空白再交回；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeDescriptions {
    described: Mutex<Vec<DescribePrivateRoom>>,
    failure: Mutex<Option<DescriptionFailure>>,
}

impl PrivateRoomDescriptionUseCases for FakeDescriptions {
    fn describe(&self, request: DescribePrivateRoom) -> PortFuture<'_, DescriptionResult<String>> {
        let answer = self
            .failure
            .lock()
            .unwrap()
            .take()
            .map_or_else(|| Ok(request.description.trim().to_owned()), Err);
        self.described.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 房主改简介_回改好以后的简介() {
    let descriptions = Arc::new(FakeDescriptions::default());
    let app = test_router(descriptions.clone());

    let response = app
        .oneshot(request(
            Some(json!({"description": "  每周读一本书\n周五晚上聊  "})),
            true,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({"description": "每周读一本书\n周五晚上聊"})
    );
    let described = descriptions.described.lock().unwrap();
    assert_eq!(described[0].catalog_id, catalog());
    assert_eq!(described[0].actor.principal_id, actor().principal_id);
    assert_eq!(described[0].description, "  每周读一本书\n周五晚上聊  ");
}

#[tokio::test]
async fn 正文不对_来源不对的不改() {
    let descriptions = Arc::new(FakeDescriptions::default());
    let app = test_router(descriptions.clone());
    for (body, origin, status, code) in [
        (
            Some(json!({"description": "简介", "name": "改名"})),
            true,
            StatusCode::BAD_REQUEST,
            "room_description.invalid",
        ),
        (
            Some(json!({"description": 3})),
            true,
            StatusCode::BAD_REQUEST,
            "room_description.invalid",
        ),
        (
            Some(json!({"description": "简介"})),
            false,
            StatusCode::FORBIDDEN,
            "room_description.invalid_origin",
        ),
    ] {
        let response = app.clone().oneshot(request(body, origin)).await.unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(descriptions.described.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 失败照种类回答() {
    let descriptions = Arc::new(FakeDescriptions::default());
    let app = test_router(descriptions.clone());
    for (kind, status, code) in [
        (
            DescriptionFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "room_description.invalid",
        ),
        (
            DescriptionFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_description.forbidden",
        ),
        (
            DescriptionFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_description.not_found",
        ),
        (
            DescriptionFailureKind::Conflict,
            StatusCode::CONFLICT,
            "room_description.conflict",
        ),
        (
            DescriptionFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_description.dependency_unavailable",
        ),
    ] {
        *descriptions.failure.lock().unwrap() = Some(DescriptionFailure::new("test", kind));
        let response = app
            .clone()
            .oneshot(request(Some(json!({"description": ""})), true))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn test_router(descriptions: Arc<FakeDescriptions>) -> Router {
    router(PrivateRoomDescriptionHttpState::new(
        descriptions,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri(format!("/private-rooms/{CATALOG_UUID}/description"))
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
