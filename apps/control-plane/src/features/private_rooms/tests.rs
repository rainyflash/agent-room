use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, PrivateRoomSnapshot, SecretValue},
    private_rooms::{
        ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom,
        GovernPrivateRoomMember, InspectPrivateRoom, InvitePrivateRoomMember, ListPrivateRooms,
        PrivateRoomMembershipAction, PrivateRoomResult, PrivateRoomUseCases,
        TransferPrivateRoomOwnership,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{PrivateRoomHttpState, router};

const OWNER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const TARGET_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedCall {
    operation: &'static str,
    catalog_id: RoomCatalogId,
    target: Option<PrincipalId>,
    permissions: Option<PrivateRoomPermissions>,
    name: Option<String>,
}

struct FakeRooms {
    calls: Mutex<Vec<ObservedCall>>,
    snapshot: PrivateRoomSnapshot,
}

impl FakeRooms {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            snapshot: snapshot(),
        }
    }

    fn record(&self, call: ObservedCall) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.calls.lock().expect("房间调用记录锁可用").push(call);
        let snapshot = self.snapshot.clone();
        Box::pin(async move { Ok(snapshot) })
    }
}

impl PrivateRoomUseCases for FakeRooms {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        assert_eq!(request.invitations.len(), 1);
        self.record(ObservedCall {
            operation: "create",
            catalog_id: request.catalog_id,
            target: request
                .invitations
                .first()
                .map(|invitation| invitation.principal_id),
            permissions: request
                .invitations
                .first()
                .map(|invitation| invitation.permissions),
            name: Some(request.name),
        })
    }

    fn inspect(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call("inspect", request.catalog_id, None, None))
    }

    fn list(
        &self,
        _request: ListPrivateRooms,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        let snapshot = self.snapshot.clone();
        Box::pin(async move { Ok(vec![snapshot]) })
    }

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "invite",
            request.catalog_id,
            Some(request.target_principal_id),
            Some(request.permissions),
        ))
    }

    fn accept(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call("accept", request.catalog_id, None, None))
    }

    fn decline(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call("decline", request.catalog_id, None, None))
    }

    fn leave(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call("leave", request.catalog_id, None, None))
    }

    fn remove(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "remove",
            request.catalog_id,
            Some(request.target_principal_id),
            None,
        ))
    }

    fn ban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "ban",
            request.catalog_id,
            Some(request.target_principal_id),
            None,
        ))
    }

    fn update_permissions(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "permissions",
            request.catalog_id,
            Some(request.target_principal_id),
            Some(request.permissions),
        ))
    }

    fn transfer_ownership(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "transfer",
            request.catalog_id,
            Some(request.target_principal_id),
            Some(request.former_owner_permissions),
        ))
    }

    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call("archive", request.catalog_id, None, None))
    }
}

#[derive(Default)]
struct FakeAuthentication {
    calls: Mutex<Vec<AuthenticationRequirement>>,
    attempts: AtomicUsize,
}

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("私人房间路由不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("私人房间路由不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.calls
            .lock()
            .expect("认证调用记录锁可用")
            .push(requirement);
        Box::pin(async move {
            assert_eq!(session_secret.expose(), "session-secret");
            Ok(actor())
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("私人房间路由不会注销") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("私人房间路由不会暂停主体") })
    }
}

#[tokio::test]
async fn 创建要求同源会话与_uuidv7_幂等键并返回真实房间投影() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let payload = json!({
        "name": "Incident Room",
        "description": "Coordinate recovery",
        "retentionDays": 30,
        "invitations": [{
            "principalId": TARGET_UUID,
            "permissions": {
                "capabilities": ["view", "speak", "automate"]
            }
        }]
    });
    let response = test_router(rooms.clone(), authentication.clone())
        .oneshot(request(
            Method::POST,
            "/private-rooms",
            &payload,
            true,
            true,
        ))
        .await
        .expect("创建路由可调用");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let response = response_json(response).await;
    assert_eq!(response["catalogId"], CATALOG_UUID);
    assert_eq!(response["matrixRoomId"], "!private:matrix.agent-room.test");
    assert_eq!(
        response["members"][0]["permissions"]["capabilities"][0],
        "view"
    );
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
    assert_eq!(
        rooms.calls.lock().expect("房间记录锁可用").as_slice(),
        &[ObservedCall {
            operation: "create",
            catalog_id: catalog_id(),
            target: Some(target_id()),
            permissions: Some(speaker_permissions()),
            name: Some("Incident Room".to_owned()),
        }]
    );
}

#[tokio::test]
async fn 错误_origin_在认证和用例之前失败关闭() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let mut request = request(
        Method::POST,
        "/private-rooms",
        &valid_creation_body(),
        true,
        true,
    );
    request.headers_mut().insert(
        header::ORIGIN,
        header::HeaderValue::from_static("https://evil.test"),
    );
    let response = test_router(rooms.clone(), authentication.clone())
        .oneshot(request)
        .await
        .expect("错误来源请求可调用");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(authentication.attempts.load(Ordering::SeqCst), 0);
    assert!(rooms.calls.lock().expect("房间记录锁可用").is_empty());
}

#[tokio::test]
async fn 不可执行权限组合在进入用例前被拒绝() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let payload = json!({
        "name": "Invalid",
        "invitations": [{
            "principalId": TARGET_UUID,
            "permissions": { "capabilities": ["view", "automate"] }
        }]
    });
    let response = test_router(rooms.clone(), authentication)
        .oneshot(request(
            Method::POST,
            "/private-rooms",
            &payload,
            true,
            true,
        ))
        .await
        .expect("非法权限请求可调用");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(rooms.calls.lock().expect("房间记录锁可用").is_empty());
}

#[tokio::test]
async fn 房主转移和归档明确要求近期认证() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(rooms.clone(), authentication.clone());
    let transfer = app
        .clone()
        .oneshot(request(
            Method::PUT,
            &format!("/private-rooms/{CATALOG_UUID}/owner"),
            &json!({
                "targetPrincipalId": TARGET_UUID,
                "formerOwnerPermissions": { "capabilities": ["view", "speak"] }
            }),
            true,
            false,
        ))
        .await
        .expect("转移路由可调用");
    let archive = app
        .oneshot(request(
            Method::DELETE,
            &format!("/private-rooms/{CATALOG_UUID}"),
            &json!({}),
            true,
            false,
        ))
        .await
        .expect("归档路由可调用");

    assert_eq!(transfer.status(), StatusCode::OK);
    assert_eq!(archive.status(), StatusCode::OK);
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[
            AuthenticationRequirement::RecentAuthentication,
            AuthenticationRequirement::RecentAuthentication,
        ]
    );
    let calls = rooms.calls.lock().expect("房间记录锁可用");
    assert_eq!(calls[0].operation, "transfer");
    assert_eq!(calls[1].operation, "archive");
}

#[tokio::test]
async fn 全部成员生命周期端点只调用对应应用用例() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(rooms.clone(), authentication);
    let cases = [
        (
            Method::POST,
            format!("/private-rooms/{CATALOG_UUID}/membership/accept"),
            "accept",
        ),
        (
            Method::POST,
            format!("/private-rooms/{CATALOG_UUID}/membership/decline"),
            "decline",
        ),
        (
            Method::POST,
            format!("/private-rooms/{CATALOG_UUID}/membership/leave"),
            "leave",
        ),
        (
            Method::DELETE,
            format!("/private-rooms/{CATALOG_UUID}/members/{TARGET_UUID}"),
            "remove",
        ),
        (
            Method::POST,
            format!("/private-rooms/{CATALOG_UUID}/members/{TARGET_UUID}/ban"),
            "ban",
        ),
    ];
    for (method, uri, expected) in cases {
        let response = app
            .clone()
            .oneshot(request(method, &uri, &json!({}), true, false))
            .await
            .expect("成员生命周期路由可调用");
        assert_eq!(response.status(), StatusCode::OK, "端点 {expected} 失败");
    }

    let operations = rooms
        .calls
        .lock()
        .expect("房间记录锁可用")
        .iter()
        .map(|call| call.operation)
        .collect::<Vec<_>>();
    assert_eq!(operations, ["accept", "decline", "leave", "remove", "ban"]);
}

#[tokio::test]
async fn 查看邀请和权限更新分别进入正确边界() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(rooms.clone(), authentication.clone());
    let inspect = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/private-rooms/{CATALOG_UUID}"),
            &json!({}),
            false,
            false,
        ))
        .await
        .expect("查看路由可调用");
    let invite = app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/private-rooms/{CATALOG_UUID}/invitations"),
            &json!({
                "targetPrincipalId": TARGET_UUID,
                "permissions": { "capabilities": ["view", "speak"] }
            }),
            true,
            false,
        ))
        .await
        .expect("邀请路由可调用");
    let permissions = app
        .oneshot(request(
            Method::PUT,
            &format!("/private-rooms/{CATALOG_UUID}/members/{TARGET_UUID}/permissions"),
            &json!({ "capabilities": ["view", "speak"] }),
            true,
            false,
        ))
        .await
        .expect("权限路由可调用");

    for response in [inspect, invite, permissions] {
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[
            AuthenticationRequirement::ActiveSession,
            AuthenticationRequirement::ActiveSession,
            AuthenticationRequirement::ActiveSession,
        ]
    );
    let calls = rooms.calls.lock().expect("房间记录锁可用");
    assert_eq!(calls[0].operation, "inspect");
    assert_eq!(calls[1].operation, "invite");
    assert_eq!(calls[2].operation, "permissions");
    assert_eq!(calls[1].permissions, Some(viewer_speaker_permissions()));
    assert_eq!(calls[2].permissions, Some(viewer_speaker_permissions()));
}

#[tokio::test]
async fn 房间列表只需活动会话并返回权威快照集合() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let response = test_router(rooms, authentication.clone())
        .oneshot(request(
            Method::GET,
            "/private-rooms",
            &json!({}),
            false,
            false,
        ))
        .await
        .expect("列表路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["rooms"][0]["catalogId"], CATALOG_UUID);
    assert_eq!(
        payload["rooms"][0]["matrixRoomId"],
        "!private:matrix.agent-room.test"
    );
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
}

fn test_router(rooms: Arc<FakeRooms>, authentication: Arc<FakeAuthentication>) -> axum::Router {
    let state = PrivateRoomHttpState::new(
        rooms,
        authentication,
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &Url::parse("http://tauri.localhost").expect("桌面地址有效"),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn request(
    method: Method,
    uri: &str,
    body: &Value,
    include_origin: bool,
    include_idempotency_key: bool,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if include_origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    if include_idempotency_key {
        request = request.header("idempotency-key", CATALOG_UUID);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("HTTP 请求有效")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024)
        .await
        .expect("响应正文可读");
    serde_json::from_slice(&body).expect("响应是 JSON")
}

fn valid_creation_body() -> Value {
    json!({
        "name": "Incident Room",
        "invitations": [{
            "principalId": TARGET_UUID,
            "permissions": { "capabilities": ["view", "speak"] }
        }]
    })
}

fn call(
    operation: &'static str,
    catalog_id: RoomCatalogId,
    target: Option<PrincipalId>,
    permissions: Option<PrivateRoomPermissions>,
) -> ObservedCall {
    ObservedCall {
        operation,
        catalog_id,
        target,
        permissions,
        name: None,
    }
}

fn snapshot() -> PrivateRoomSnapshot {
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "Incident Room".to_owned(),
            description: "Coordinate recovery".to_owned(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner_id()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("私人目录有效");
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(uuid(INSTANCE_UUID)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!private:matrix.agent-room.test".to_owned())
                .expect("Matrix 房间标识有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 1,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间实例有效");
    PrivateRoomSnapshot::new(
        catalog,
        instance,
        PrivateRoom::create(catalog_id(), owner_id()),
    )
    .expect("私人房间快照有效")
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: owner_id(),
        matrix_user_id: "@owner:matrix.agent-room.test".to_owned(),
        display_name: "Owner".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_028_800_000),
        recently_authenticated: true,
    }
}

fn speaker_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_bits(0b1_0011).expect("查看、发言和自动化权限有效")
}

fn viewer_speaker_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_bits(0b0_0011).expect("查看和发言权限有效")
}

fn owner_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid(OWNER_UUID))
}

fn target_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid(TARGET_UUID))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(CATALOG_UUID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
