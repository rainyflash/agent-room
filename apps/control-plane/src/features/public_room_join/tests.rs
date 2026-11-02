use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    private_rooms::{
        JoinPublicRoom, PublicJoinFailure, PublicJoinFailureKind, PublicJoinMembership,
        PublicJoinResult, PublicRoomJoinUseCases,
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

use super::{PublicRoomJoinHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 回答 `answer`；`failure` 设上时回答它。
struct FakeJoins {
    joined: Mutex<Vec<JoinPublicRoom>>,
    answer: PublicJoinMembership,
    failure: Mutex<Option<PublicJoinFailure>>,
}

impl FakeJoins {
    fn new(answer: PublicJoinMembership) -> Self {
        Self {
            joined: Mutex::new(Vec::new()),
            answer,
            failure: Mutex::new(None),
        }
    }
}

impl PublicRoomJoinUseCases for FakeJoins {
    fn join(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<PublicJoinMembership>> {
        self.joined.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        let answer = self.answer;
        Box::pin(async move { failure.map_or(Ok(answer), Err) })
    }
}

#[tokio::test]
async fn 直接进的房间回受邀_已经在里面的回已加入_都不缓存() {
    for (answer, membership) in [
        (PublicJoinMembership::Invited, "invited"),
        (PublicJoinMembership::Joined, "joined"),
    ] {
        let joins = Arc::new(FakeJoins::new(answer));
        let response = test_router(joins.clone())
            .oneshot(request(
                &format!("/private-rooms/{CATALOG_UUID}/join"),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response_json(response).await,
            json!({ "catalogId": CATALOG_UUID, "membership": membership })
        );
        let joined = joins.joined.lock().unwrap();
        assert_eq!(joined[0].actor, actor());
        assert_eq!(joined[0].catalog_id, catalog());
    }
}

#[tokio::test]
async fn 要来自自己的网页_房间号要对() {
    let joins = Arc::new(FakeJoins::new(PublicJoinMembership::Invited));
    let app = test_router(joins.clone());
    let response = app
        .clone()
        .oneshot(request(
            &format!("/private-rooms/{CATALOG_UUID}/join"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await["code"],
        "room_join.invalid_origin"
    );
    let response = app
        .oneshot(request("/private-rooms/not-a-room/join", true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["code"],
        "room_join.invalid_resource"
    );
    assert!(joins.joined.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 失败按种类回答() {
    for (kind, status, code) in [
        (
            PublicJoinFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_join.not_found",
        ),
        (
            PublicJoinFailureKind::Banned,
            StatusCode::FORBIDDEN,
            "room_join.banned",
        ),
        (
            PublicJoinFailureKind::ApprovalRequired,
            StatusCode::CONFLICT,
            "room_join.approval_required",
        ),
        (
            PublicJoinFailureKind::InviteOnly,
            StatusCode::FORBIDDEN,
            "room_join.invite_only",
        ),
        (
            PublicJoinFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_join.dependency_unavailable",
        ),
    ] {
        let joins = Arc::new(FakeJoins::new(PublicJoinMembership::Invited));
        *joins.failure.lock().unwrap() = Some(PublicJoinFailure::new("test", kind));
        let response = test_router(joins)
            .oneshot(request(
                &format!("/private-rooms/{CATALOG_UUID}/join"),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn test_router(joins: Arc<FakeJoins>) -> Router {
    router(PublicRoomJoinHttpState::new(
        joins,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(uri: &str, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(uri)
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

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
