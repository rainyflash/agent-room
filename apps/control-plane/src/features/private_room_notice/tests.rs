use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    private_rooms::{
        NoticeFailure, NoticeFailureKind, NoticeResult, PrivateRoomNoticeUseCases, RoomNotice,
        UpdateRoomNotice,
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

use super::{PrivateRoomNoticeHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const UPDATED_AT: i64 = 1_700_000_100_000;

/// 把公告去掉首尾空白再交回；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeNotices {
    updated: Mutex<Vec<UpdateRoomNotice>>,
    failure: Mutex<Option<NoticeFailure>>,
}

impl PrivateRoomNoticeUseCases for FakeNotices {
    fn update_notice(&self, request: UpdateRoomNotice) -> PortFuture<'_, NoticeResult<RoomNotice>> {
        let answer = self.failure.lock().unwrap().take().map_or_else(
            || {
                Ok(RoomNotice {
                    text: request.text.trim().to_owned(),
                    updated_at: time(UPDATED_AT),
                })
            },
            Err,
        );
        self.updated.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 改公告_回写进去的那一版和时间() {
    let notices = Arc::new(FakeNotices::default());
    let app = test_router(notices.clone());

    let response = app
        .oneshot(request(
            Some(json!({"text": "  只在被点名时说话。\n回复要短。 "})),
            true,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({"text": "只在被点名时说话。\n回复要短。", "updatedAtUnixMs": UPDATED_AT})
    );
    let updated = notices.updated.lock().unwrap();
    assert_eq!(updated[0].catalog_id, catalog());
    assert_eq!(updated[0].actor.principal_id, actor().principal_id);
    assert_eq!(updated[0].text, "  只在被点名时说话。\n回复要短。 ");
}

#[tokio::test]
async fn 正文不对_来源不对的不改() {
    let notices = Arc::new(FakeNotices::default());
    let app = test_router(notices.clone());
    for (body, origin, status, code) in [
        (
            Some(json!({"text": "公告", "pinned": true})),
            true,
            StatusCode::BAD_REQUEST,
            "room_notice.invalid",
        ),
        (
            Some(json!({"text": 3})),
            true,
            StatusCode::BAD_REQUEST,
            "room_notice.invalid",
        ),
        (None, true, StatusCode::BAD_REQUEST, "room_notice.invalid"),
        (
            Some(json!({"text": "公告"})),
            false,
            StatusCode::FORBIDDEN,
            "room_notice.invalid_origin",
        ),
    ] {
        let response = app.clone().oneshot(request(body, origin)).await.unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(notices.updated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 失败照种类回答() {
    let notices = Arc::new(FakeNotices::default());
    let app = test_router(notices.clone());
    for (kind, status, code) in [
        (
            NoticeFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "room_notice.invalid",
        ),
        (
            NoticeFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_notice.forbidden",
        ),
        (
            NoticeFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_notice.not_found",
        ),
        (
            NoticeFailureKind::Conflict,
            StatusCode::CONFLICT,
            "room_notice.conflict",
        ),
        (
            NoticeFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_notice.dependency_unavailable",
        ),
        (
            NoticeFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_notice.internal",
        ),
    ] {
        *notices.failure.lock().unwrap() = Some(NoticeFailure::new("test", kind));
        let response = app
            .clone()
            .oneshot(request(Some(json!({"text": ""})), true))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn 改得太勤回_429_带_retry_after() {
    let notices = Arc::new(FakeNotices::default());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let retry_at = i64::try_from(now).unwrap() + 120_000;
    *notices.failure.lock().unwrap() = Some(NoticeFailure::new(
        "test",
        NoticeFailureKind::RateLimited {
            retry_at: time(retry_at),
        },
    ));
    let response = test_router(notices)
        .oneshot(request(Some(json!({"text": "再改一次"})), true))
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
        "room_notice.rate_limited"
    );
}

fn test_router(notices: Arc<FakeNotices>) -> Router {
    router(PrivateRoomNoticeHttpState::new(
        notices,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri(format!("/private-rooms/{CATALOG_UUID}/notice"))
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
