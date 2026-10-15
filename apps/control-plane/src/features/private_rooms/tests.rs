use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
        DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        RevokedDevice,
    },
    ports::{PortFuture, PrincipalAccount, PrivateRoomSnapshot, SecretFactory, SecretValue},
    private_rooms::{
        ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom,
        GovernPrivateRoomMember, InspectPrivateRoom, InvitePrivateRoomMember, ListPrivateRooms,
        ListPrivateRoomsForAccount, PrivateRoomActor, PrivateRoomCreatorAgent,
        PrivateRoomMembershipAction, PrivateRoomOrigin, PrivateRoomResult, PrivateRoomUseCases,
        RenamePrivateRoom, TransferPrivateRoomOwnership,
    },
};
use agent_room_domain::{
    devices::Device,
    identity::Principal,
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{PrivateRoomHttpDependencies, PrivateRoomHttpState, router};

const OWNER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const TARGET_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

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
    /// 建房、邀请是谁做的：网页上的人（`person`）还是本机设备（`device`）。
    actors: Mutex<Vec<&'static str>>,
    created_by_agent: Mutex<Option<AgentId>>,
}

impl FakeRooms {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            snapshot: snapshot(),
            actors: Mutex::new(Vec::new()),
            created_by_agent: Mutex::new(None),
        }
    }

    fn note(&self, actor: &PrivateRoomActor) {
        self.actors.lock().unwrap().push(match actor {
            PrivateRoomActor::Person(_) => "person",
            PrivateRoomActor::Device(_) => "device",
            PrivateRoomActor::NetworkAgent(_) => "network",
        });
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
        self.note(&request.actor);
        *self.created_by_agent.lock().unwrap() = request.created_by_agent;
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

    fn origin(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomOrigin>> {
        self.calls.lock().expect("房间调用记录锁可用").push(call(
            "origin",
            request.catalog_id,
            None,
            None,
        ));
        Box::pin(async move {
            Ok(PrivateRoomOrigin {
                created_by_agent: Some(PrivateRoomCreatorAgent {
                    agent_id: AgentId::from_uuid(uuid(AGENT_UUID)),
                    display_name: Some("Visitor".to_owned()),
                }),
                owner_is_network_agent: true,
            })
        })
    }

    fn list(
        &self,
        _request: ListPrivateRooms,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        let snapshot = self.snapshot.clone();
        Box::pin(async move { Ok(vec![snapshot]) })
    }

    fn list_for_account(
        &self,
        _request: ListPrivateRoomsForAccount,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        let snapshot = self.snapshot.clone();
        Box::pin(async move { Ok(vec![snapshot]) })
    }

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.note(&request.actor);
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

    fn unban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(call(
            "unban",
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

    fn rename(
        &self,
        request: RenamePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.record(ObservedCall {
            name: Some(request.name),
            ..call("rename", request.catalog_id, None, None)
        })
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

/// 只认一次设备签名：方法、路径和正文都要对得上。
#[derive(Default)]
struct FakeDevices {
    expected: Mutex<Option<(String, String, String)>>,
}

impl FakeDevices {
    fn expecting(method: &str, target: &str, body: &str) -> Arc<Self> {
        Arc::new(Self {
            expected: Mutex::new(Some((
                method.to_owned(),
                target.to_owned(),
                body.to_owned(),
            ))),
        })
    }
}

impl DeviceAuthorizationUseCases for FakeDevices {
    fn register_device(
        &self,
        _request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("私人房间路由不会注册设备") })
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        let (method, target, body) = self
            .expected
            .lock()
            .unwrap()
            .take()
            .expect("登记了设备签名");
        assert_eq!(request.access_token.expose(), "device-access-token");
        assert_eq!(request.proof.method(), method);
        assert_eq!(request.proof.request_target(), target, "签名覆盖的路径");
        assert_eq!(
            request.proof.body_digest(),
            &SecureSecretFactory.digest(&body)
        );
        Box::pin(async {
            Ok(AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(owner_id()),
                    matrix_user_id: "@owner:matrix.agent-room.test".to_owned(),
                    display_name: "Owner".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device_id: DeviceId::from_uuid(uuid(DEVICE_UUID)),
                access_token_expires_at: time(1_700_000_900_000),
            })
        })
    }

    fn refresh_device_session<'a>(
        &'a self,
        _request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("私人房间路由不会刷新设备会话") })
    }

    fn list_devices(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(async { unreachable!("私人房间路由不会列出设备") })
    }

    fn revoke_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(async { unreachable!("私人房间路由不会撤销设备") })
    }
}

#[tokio::test]
async fn 本机设备签名建房_说清建房的_agent_不顺带邀请人() {
    let rooms = Arc::new(FakeRooms::new());
    let target = "/devices/current/private-rooms";
    let body = json!({ "name": "Weekly sync", "agentId": AGENT_UUID }).to_string();
    let response = device_router(rooms.clone(), FakeDevices::expecting("POST", target, &body))
        .oneshot(device_request(target, &body, true))
        .await
        .expect("路由可调用");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "catalogId": CATALOG_UUID,
            "matrixRoomId": "!private:matrix.agent-room.test",
            "name": "Incident Room",
        })
    );
    assert_eq!(
        *rooms.calls.lock().unwrap(),
        [ObservedCall {
            name: Some("Weekly sync".to_owned()),
            ..call("create", catalog_id(), None, None)
        }]
    );
    assert_eq!(*rooms.actors.lock().unwrap(), ["device"]);
    assert_eq!(
        *rooms.created_by_agent.lock().unwrap(),
        Some(AgentId::from_uuid(uuid(AGENT_UUID)))
    );
}

#[tokio::test]
async fn 本机设备签名建房要房间号和合法的建房_agent() {
    let rooms = Arc::new(FakeRooms::new());
    let target = "/devices/current/private-rooms";
    let valid = json!({ "name": "Weekly sync", "agentId": AGENT_UUID }).to_string();
    let missing_key = device_router(rooms.clone(), Arc::default())
        .oneshot(device_request(target, &valid, false))
        .await
        .expect("路由可调用");
    assert_eq!(missing_key.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(missing_key).await["code"],
        "private_room.invalid_idempotency_key"
    );

    for body in [
        json!({ "name": "Weekly sync", "agentId": "not-a-uuid" }),
        json!({ "name": "Weekly sync" }),
        json!({ "name": "Weekly sync", "agentId": AGENT_UUID, "invitations": [] }),
    ] {
        let body = body.to_string();
        let refused = device_router(rooms.clone(), FakeDevices::expecting("POST", target, &body))
            .oneshot(device_request(target, &body, true))
            .await
            .expect("路由可调用");
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            response_json(refused).await["code"],
            "private_room.invalid_creation_body"
        );
    }
    assert!(rooms.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 本机设备签名按账户_id_邀请人_给查看发言和自动发言() {
    let rooms = Arc::new(FakeRooms::new());
    let target = format!("/devices/current/private-rooms/{CATALOG_UUID}/invitations");
    let body = json!({ "accountId": TARGET_UUID }).to_string();
    let response = device_router(
        rooms.clone(),
        FakeDevices::expecting("POST", &target, &body),
    )
    .oneshot(device_request(&target, &body, false))
    .await
    .expect("路由可调用");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        *rooms.calls.lock().unwrap(),
        [call(
            "invite",
            catalog_id(),
            Some(target_id()),
            Some(PrivateRoomPermissions::INVITED_BY_AGENT)
        )]
    );
    assert_eq!(*rooms.actors.lock().unwrap(), ["device"]);

    let body = json!({ "accountId": "someone" }).to_string();
    let refused = device_router(
        rooms.clone(),
        FakeDevices::expecting("POST", &target, &body),
    )
    .oneshot(device_request(&target, &body, false))
    .await
    .expect("路由可调用");
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(refused).await["code"],
        "private_room.invalid_invitation_body"
    );
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
async fn 改名只需活动会话且拒绝多余字段() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(rooms.clone(), authentication.clone());
    let renamed = app
        .clone()
        .oneshot(request(
            Method::PUT,
            &format!("/private-rooms/{CATALOG_UUID}/name"),
            &json!({ "name": "Design review" }),
            true,
            false,
        ))
        .await
        .expect("改名路由可调用");
    assert_eq!(renamed.status(), StatusCode::OK);
    let rejected = app
        .oneshot(request(
            Method::PUT,
            &format!("/private-rooms/{CATALOG_UUID}/name"),
            &json!({ "name": "x", "owner": OWNER_UUID }),
            true,
            false,
        ))
        .await
        .expect("改名路由可调用");
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
    let calls = rooms.calls.lock().expect("房间记录锁可用");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].operation, "rename");
    assert_eq!(calls[0].name.as_deref(), Some("Design review"));
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
        (
            Method::POST,
            format!("/private-rooms/{CATALOG_UUID}/members/{TARGET_UUID}/unban"),
            "unban",
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
    assert_eq!(
        operations,
        ["accept", "decline", "leave", "remove", "ban", "unban"]
    );
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
async fn 房间是谁建的只需活动会话_房间号不对在用例之前就拒绝() {
    let rooms = Arc::new(FakeRooms::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(rooms.clone(), authentication.clone());
    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/private-rooms/{CATALOG_UUID}/origin"),
            &json!({}),
            false,
            false,
        ))
        .await
        .expect("路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "createdByAgent": { "agentId": AGENT_UUID, "displayName": "Visitor" },
            "ownerIsNetworkAgent": true,
        })
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );

    let invalid = app
        .oneshot(request(
            Method::GET,
            "/private-rooms/not-a-room/origin",
            &json!({}),
            false,
            false,
        ))
        .await
        .expect("路由可调用");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(invalid).await["code"],
        "private_room.invalid_resource_id"
    );
    assert_eq!(
        *rooms.calls.lock().unwrap(),
        [call("origin", catalog_id(), None, None)]
    );
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
    router_with(rooms, authentication, Arc::default())
}

fn device_router(rooms: Arc<FakeRooms>, devices: Arc<FakeDevices>) -> axum::Router {
    router_with(rooms, Arc::default(), devices)
}

fn router_with(
    rooms: Arc<FakeRooms>,
    authentication: Arc<FakeAuthentication>,
    devices: Arc<FakeDevices>,
) -> axum::Router {
    let state = PrivateRoomHttpState::new(
        PrivateRoomHttpDependencies {
            rooms,
            authentication,
            devices,
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &crate::config::DesktopOrigins::for_tests(),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

/// 本机 Bridge 发的请求：设备令牌和签名头，没有会话和来源。
fn device_request(path: &str, body: &str, include_idempotency_key: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        );
    if include_idempotency_key {
        request = request.header("idempotency-key", CATALOG_UUID);
    }
    request
        .body(Body::from(body.to_owned()))
        .expect("HTTP 请求有效")
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

/// 一个受邀/已加入语义清楚的私人房间快照：目录名「Incident Room」，所有者 `owner_id()`。
pub(crate) fn snapshot() -> PrivateRoomSnapshot {
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

pub(crate) fn owner_id() -> PrincipalId {
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
