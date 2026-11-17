use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    room_excerpts::{
        DeleteExcerpt, ExcerptFailure, ExcerptFailureKind, ExcerptList, ExcerptRangeProblem,
        ExcerptResult, ExcerptSummary, ListExcerpts, RoomExcerptUseCases, ShareExcerpt,
        SharedExcerpt,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
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

use super::{RoomExcerptHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const SERVER_NAME: &str = "agent-room.test";
const SHARE_CODE: &str = "Ab12Cd";
const EXCERPT_CODE: &str = "Xy34Zw56";

/// 记下收到的请求；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeExcerpts {
    shared: Mutex<Vec<ShareExcerpt>>,
    listed: Mutex<Vec<ListExcerpts>>,
    deleted: Mutex<Vec<DeleteExcerpt>>,
    failure: Mutex<Option<ExcerptFailure>>,
}

impl FakeExcerpts {
    fn failing(kind: ExcerptFailureKind) -> Arc<Self> {
        let excerpts = Arc::new(Self::default());
        *excerpts.failure.lock().unwrap() = Some(ExcerptFailure::new("room_excerpt.test", kind));
        excerpts
    }

    fn answer<T>(&self, value: T) -> ExcerptResult<T> {
        self.failure.lock().unwrap().take().map_or(Ok(value), Err)
    }
}

impl RoomExcerptUseCases for FakeExcerpts {
    fn share(&self, request: ShareExcerpt) -> PortFuture<'_, ExcerptResult<SharedExcerpt>> {
        let answer = self.answer(SharedExcerpt {
            code: excerpt_code(),
            share_code: share_code(),
            message_count: 3,
        });
        self.shared.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn list(&self, request: ListExcerpts) -> PortFuture<'_, ExcerptResult<ExcerptList>> {
        let answer = self.answer(ExcerptList {
            share_code: Some(share_code()),
            excerpts: vec![
                ExcerptSummary {
                    code: excerpt_code(),
                    message_count: 3,
                    first_message_at: time(1_700_000_100_000),
                    shared_at: time(1_700_000_200_000),
                    mine: true,
                },
                ExcerptSummary {
                    code: RoomExcerptCode::parse("Other123").unwrap(),
                    message_count: 30,
                    first_message_at: time(1_700_000_000_000),
                    shared_at: time(1_700_000_150_000),
                    mine: false,
                },
            ],
        });
        self.listed.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn delete(&self, request: DeleteExcerpt) -> PortFuture<'_, ExcerptResult<()>> {
        let answer = self.answer(());
        self.deleted.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn 分享一段_回201和挂在分享链接下面的地址() {
    let excerpts = Arc::new(FakeExcerpts::default());
    let response = test_router(excerpts.clone())
        .oneshot(share_request(
            &json!({"firstEventId": "$first", "lastEventId": "$last"}),
            true,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "code": EXCERPT_CODE,
            "url": format!("https://{SERVER_NAME}/r/{SHARE_CODE}/s/{EXCERPT_CODE}"),
        })
    );
    let shared = excerpts.shared.lock().unwrap();
    assert_eq!(shared[0].catalog_id, catalog());
    assert_eq!(shared[0].actor.principal_id, actor().principal_id);
    assert_eq!(shared[0].first_event_id.as_str(), "$first");
    assert_eq!(shared[0].last_event_id.as_str(), "$last");
}

#[tokio::test]
async fn 列出来的写着几句_什么时候_是不是自己的_不写是谁() {
    let excerpts = Arc::new(FakeExcerpts::default());
    let response = test_router(excerpts.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/private-rooms/{CATALOG_UUID}/excerpts"))
                .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "excerpts": [
                {
                    "code": EXCERPT_CODE,
                    "url": format!("https://{SERVER_NAME}/r/{SHARE_CODE}/s/{EXCERPT_CODE}"),
                    "messageCount": 3,
                    "firstMessageAtUnixMs": 1_700_000_100_000_i64,
                    "sharedAtUnixMs": 1_700_000_200_000_i64,
                    "mine": true,
                },
                {
                    "code": "Other123",
                    "url": format!("https://{SERVER_NAME}/r/{SHARE_CODE}/s/Other123"),
                    "messageCount": 30,
                    "firstMessageAtUnixMs": 1_700_000_000_000_i64,
                    "sharedAtUnixMs": 1_700_000_150_000_i64,
                    "mine": false,
                },
            ],
        })
    );
    assert_eq!(excerpts.listed.lock().unwrap()[0].catalog_id, catalog());
}

#[tokio::test]
async fn 删掉一段回204() {
    let excerpts = Arc::new(FakeExcerpts::default());
    let response = test_router(excerpts.clone())
        .oneshot(delete_request(EXCERPT_CODE, true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let deleted = excerpts.deleted.lock().unwrap();
    assert_eq!(deleted[0].code, excerpt_code());
    assert_eq!(deleted[0].catalog_id, catalog());
}

#[tokio::test]
async fn 每种失败各有各的码() {
    for (kind, status, code) in [
        (
            ExcerptFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_excerpt.forbidden",
        ),
        (
            ExcerptFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_excerpt.not_found",
        ),
        (
            ExcerptFailureKind::NotLive,
            StatusCode::CONFLICT,
            "room_excerpt.not_live",
        ),
        (
            ExcerptFailureKind::TooLong,
            StatusCode::UNPROCESSABLE_ENTITY,
            "room_excerpt.too_long",
        ),
        (
            ExcerptFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_excerpt.dependency_unavailable",
        ),
        (
            ExcerptFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_excerpt.internal",
        ),
    ] {
        let response = test_router(FakeExcerpts::failing(kind))
            .oneshot(share_request(
                &json!({"firstEventId": "$first", "lastEventId": "$last"}),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response_json(response).await["code"], code);
    }
}

#[tokio::test]
async fn 选的那几句不对时说是哪一种_分享太多时说多久以后再来() {
    for (problem, reason) in [
        (ExcerptRangeProblem::NotFound, "not_found"),
        (ExcerptRangeProblem::BeforeLive, "before_live"),
    ] {
        let response = test_router(FakeExcerpts::failing(ExcerptFailureKind::InvalidRange(
            problem,
        )))
        .oneshot(share_request(
            &json!({"firstEventId": "$first", "lastEventId": "$last"}),
            true,
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = response_json(response).await;
        assert_eq!(body["code"], "room_excerpt.invalid_range");
        assert_eq!(body["details"]["reason"], reason);
    }

    let response = test_router(FakeExcerpts::failing(ExcerptFailureKind::RateLimited {
        retry_after_seconds: 120,
    }))
    .oneshot(share_request(
        &json!({"firstEventId": "$first", "lastEventId": "$last"}),
        true,
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "120");
    assert_eq!(
        response_json(response).await["code"],
        "room_excerpt.rate_limited"
    );
}

#[tokio::test]
async fn 来源不对_正文不对_房间号和段码不对都不往下做() {
    let excerpts = Arc::new(FakeExcerpts::default());

    let wrong_origin = test_router(excerpts.clone())
        .oneshot(share_request(
            &json!({"firstEventId": "$first", "lastEventId": "$last"}),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(wrong_origin).await["code"],
        "room_excerpt.invalid_origin"
    );
    let wrong_origin = test_router(excerpts.clone())
        .oneshot(delete_request(EXCERPT_CODE, false))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);

    for body in [
        json!({"firstEventId": "$first"}),
        json!({"firstEventId": "$first", "lastEventId": "$last", "text": "不收正文"}),
        json!({"firstEventId": "", "lastEventId": "$last"}),
        json!({"firstEventId": "$first", "lastEventId": 7}),
    ] {
        let response = test_router(excerpts.clone())
            .oneshot(share_request(&body, true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(response).await["code"],
            "room_excerpt.invalid"
        );
    }

    for code in ["short", "has-dash", "toolong123"] {
        let response = test_router(excerpts.clone())
            .oneshot(delete_request(code, true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{code}");
        assert_eq!(
            response_json(response).await["code"],
            "room_excerpt.invalid_resource"
        );
    }

    let wrong_room = Request::builder()
        .method(Method::GET)
        .uri("/private-rooms/not-a-room/excerpts")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .body(Body::empty())
        .unwrap();
    let response = test_router(excerpts.clone())
        .oneshot(wrong_room)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["code"],
        "room_excerpt.invalid_resource"
    );

    assert!(excerpts.shared.lock().unwrap().is_empty());
    assert!(excerpts.listed.lock().unwrap().is_empty());
    assert!(excerpts.deleted.lock().unwrap().is_empty());
}

fn test_router(excerpts: Arc<FakeExcerpts>) -> Router {
    router(RoomExcerptHttpState::new(
        excerpts,
        Arc::new(FakeAuthentication),
        SERVER_NAME,
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn share_request(body: &Value, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(format!("/private-rooms/{CATALOG_UUID}/excerpts"))
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .header(header::CONTENT_TYPE, "application/json");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

fn delete_request(code: &str, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/private-rooms/{CATALOG_UUID}/excerpts/{code}"))
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).unwrap()
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

fn share_code() -> RoomShareCode {
    RoomShareCode::parse(SHARE_CODE).unwrap()
}

fn excerpt_code() -> RoomExcerptCode {
    RoomExcerptCode::parse(EXCERPT_CODE).unwrap()
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
