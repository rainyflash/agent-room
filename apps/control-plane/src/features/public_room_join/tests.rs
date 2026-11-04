use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{JoinRequestRecord, PendingJoinRequest, PortFuture, SecretValue},
    private_rooms::{
        AnswerJoinRequest, JoinPublicRoom, JoinRequestState, PublicJoinFailure,
        PublicJoinFailureKind, PublicJoinMembership, PublicJoinResult, PublicRoomJoinUseCases,
        RequestToJoin,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
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
const REQUESTER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 进房间回答 `answer`，申请回答在等；`failure` 设上时下一次调用回答它。
struct FakeJoins {
    joined: Mutex<Vec<JoinPublicRoom>>,
    requested: Mutex<Vec<RequestToJoin>>,
    withdrawn: Mutex<Vec<JoinPublicRoom>>,
    answered: Mutex<Vec<AnswerJoinRequest>>,
    answer: PublicJoinMembership,
    failure: Mutex<Option<PublicJoinFailure>>,
}

impl FakeJoins {
    fn new(answer: PublicJoinMembership) -> Self {
        Self {
            joined: Mutex::new(Vec::new()),
            requested: Mutex::new(Vec::new()),
            withdrawn: Mutex::new(Vec::new()),
            answered: Mutex::new(Vec::new()),
            answer,
            failure: Mutex::new(None),
        }
    }

    fn reply<T: Send + 'static>(&self, value: T) -> PortFuture<'_, PublicJoinResult<T>> {
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or(Ok(value), Err) })
    }
}

impl PublicRoomJoinUseCases for FakeJoins {
    fn join(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<PublicJoinMembership>> {
        self.joined.lock().unwrap().push(request);
        self.reply(self.answer)
    }

    fn request(
        &self,
        request: RequestToJoin,
    ) -> PortFuture<'_, PublicJoinResult<JoinRequestState>> {
        self.requested.lock().unwrap().push(request);
        self.reply(JoinRequestState::Pending {
            requested_at: time(1_000),
        })
    }

    fn withdraw(&self, request: JoinPublicRoom) -> PortFuture<'_, PublicJoinResult<()>> {
        self.withdrawn.lock().unwrap().push(request);
        self.reply(())
    }

    fn pending_requests(
        &self,
        _request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<Vec<PendingJoinRequest>>> {
        self.reply(vec![PendingJoinRequest {
            record: JoinRequestRecord {
                catalog_id: catalog(),
                requester: requester(),
                message: JoinRequestMessage::parse("想来听听").unwrap(),
                status: JoinRequestStatus::Pending,
                requested_at: time(2_000),
                decided_at: None,
                decided_by: None,
            },
            requester_display_name: "小林".to_owned(),
        }])
    }

    fn answer(
        &self,
        request: AnswerJoinRequest,
    ) -> PortFuture<'_, PublicJoinResult<JoinRequestStatus>> {
        let status = if request.approve {
            JoinRequestStatus::Approved
        } else {
            JoinRequestStatus::Rejected
        };
        self.answered.lock().unwrap().push(request);
        self.reply(status)
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

#[tokio::test]
async fn 申请带上留言_在等的回答申请时间_撤回回空() {
    let joins = Arc::new(FakeJoins::new(PublicJoinMembership::Invited));
    let app = test_router(joins.clone());
    let path = format!("/private-rooms/{CATALOG_UUID}/join-requests");
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &path,
            Some(json!({ "message": "想来听听" })),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({ "catalogId": CATALOG_UUID, "membership": "pending", "requestedAtUnixMs": 1_000 })
    );
    // 不留话可以不带正文。
    let response = app
        .clone()
        .oneshot(json_request(Method::POST, &path, None, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    {
        let requested = joins.requested.lock().unwrap();
        assert_eq!(requested[0].message, "想来听听");
        assert_eq!(requested[1].message, "");
    }
    for bad in [json!({ "message": 1 }), json!({ "note": "hi" })] {
        let response = app
            .clone()
            .oneshot(json_request(Method::POST, &path, Some(bad.clone()), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(joins.requested.lock().unwrap().len(), 2);

    let response = app
        .clone()
        .oneshot(json_request(Method::DELETE, &path, None, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(joins.withdrawn.lock().unwrap().len(), 1);
    let response = app
        .oneshot(json_request(Method::DELETE, &path, None, false))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "撤回也要来自自己的网页"
    );
}

#[tokio::test]
async fn 管理员看在等的申请_批准或拒绝() {
    let joins = Arc::new(FakeJoins::new(PublicJoinMembership::Invited));
    let app = test_router(joins.clone());
    let response = app
        .clone()
        .oneshot(json_request(
            Method::GET,
            &format!("/private-rooms/{CATALOG_UUID}/join-requests"),
            None,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({ "requests": [{
            "principalId": REQUESTER_UUID,
            "displayName": "小林",
            "message": "想来听听",
            "requestedAtUnixMs": 2_000,
        }] })
    );

    let path = format!("/private-rooms/{CATALOG_UUID}/join-requests/{REQUESTER_UUID}");
    for (decision, status) in [("approve", "approved"), ("reject", "rejected")] {
        let response = app
            .clone()
            .oneshot(json_request(
                Method::PUT,
                &path,
                Some(json!({ "decision": decision })),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, json!({ "status": status }));
    }
    {
        let answered = joins.answered.lock().unwrap();
        assert_eq!(answered.len(), 2);
        assert!(answered[0].approve);
        assert!(!answered[1].approve);
        assert_eq!(answered[0].requester, requester());
        assert_eq!(answered[0].catalog_id, catalog());
    }
    for (uri, body) in [
        (path.clone(), json!({ "decision": "maybe" })),
        (
            format!("/private-rooms/{CATALOG_UUID}/join-requests/not-a-person"),
            json!({ "decision": "approve" }),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(json_request(Method::PUT, &uri, Some(body), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(joins.answered.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 被拒不到一天再申请_说什么时候能再来() {
    let joins = Arc::new(FakeJoins::new(PublicJoinMembership::Invited));
    let retry_at = UtcMillis::new(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
            + 3_600_000,
    )
    .unwrap();
    *joins.failure.lock().unwrap() = Some(PublicJoinFailure::recently_rejected("test", retry_at));
    let response = test_router(joins)
        .oneshot(json_request(
            Method::POST,
            &format!("/private-rooms/{CATALOG_UUID}/join-requests"),
            None,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let retry_after: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((3_590..=3_600).contains(&retry_after), "{retry_after}");
    assert_eq!(
        response_json(response).await["code"],
        "room_join.recently_rejected"
    );
}

fn json_request(method: Method, uri: &str, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
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

fn requester() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(REQUESTER_UUID).unwrap())
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
