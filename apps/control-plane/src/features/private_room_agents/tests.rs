use std::sync::{Arc, Mutex};

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
    ports::{
        MatrixUserId, PortFuture, PrincipalAccount, PrivateRoomAgentMemberRecord,
        PrivateRoomJoinCodeRecord, SecretFactory, SecretValue,
    },
    private_rooms::{
        AgentAccessFailure, AgentAccessFailureKind, AgentAccessResult, AgentAccessView,
        GeneratedJoinCode, InspectAgentAccess, JoinCodeCaller, ManageJoinCode,
        PrivateRoomAgentAccessUseCases, RedeemJoinCode, RedeemedRoom, RemoveAgentMember,
        ResolveJoinCode,
    },
};
use agent_room_domain::{
    devices::Device,
    identity::Principal,
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId},
    join_codes::{PrivateRoomAgentMemberStatus, PrivateRoomJoinCode},
    private_rooms::PrivateRoomPermissions,
    rooms::MatrixRoomReference,
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

use super::{PrivateRoomAgentHttpDependencies, PrivateRoomAgentHttpState, router};

const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const OWNER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

#[derive(Default)]
struct FakeAccess {
    calls: Mutex<Vec<String>>,
    redeemed: Mutex<Option<RedeemJoinCode>>,
    resolved: Mutex<Option<ResolveJoinCode>>,
    redeem_failure: Mutex<Option<AgentAccessFailureKind>>,
}

impl FakeAccess {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl PrivateRoomAgentAccessUseCases for FakeAccess {
    fn inspect(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<AgentAccessView>> {
        self.record(format!("inspect:{}", request.catalog_id));
        Box::pin(async move {
            Ok(AgentAccessView {
                join_code: Some(PrivateRoomJoinCodeRecord {
                    catalog_id: request.catalog_id,
                    permissions: PrivateRoomPermissions::AGENT_MEMBER,
                    created_by: owner_id(),
                    created_at: time(1_700_000_000_000),
                }),
                agents: vec![PrivateRoomAgentMemberRecord {
                    catalog_id: request.catalog_id,
                    agent_id: agent_id(),
                    display_name: "Scout".to_owned(),
                    matrix_user_id: MatrixUserId::new("@_agent_scout:matrix.test").unwrap(),
                    owner_display_name: Some("Bob".to_owned()),
                    status: PrivateRoomAgentMemberStatus::Joined,
                    permissions: PrivateRoomPermissions::AGENT_MEMBER,
                    joined_at: time(1_700_000_100_000),
                    status_changed_at: time(1_700_000_100_000),
                }],
            })
        })
    }

    fn generate_code(
        &self,
        request: ManageJoinCode,
    ) -> PortFuture<'_, AgentAccessResult<GeneratedJoinCode>> {
        self.record(format!("generate:{}", request.catalog_id));
        Box::pin(async move {
            Ok(GeneratedJoinCode {
                code: PrivateRoomJoinCode::parse("K7P3-Q9XW-2DMA").unwrap(),
                record: PrivateRoomJoinCodeRecord {
                    catalog_id: request.catalog_id,
                    permissions: PrivateRoomPermissions::AGENT_MEMBER,
                    created_by: request.actor.principal_id,
                    created_at: time(1_700_000_200_000),
                },
            })
        })
    }

    fn disable_code(&self, request: ManageJoinCode) -> PortFuture<'_, AgentAccessResult<()>> {
        self.record(format!("disable:{}", request.catalog_id));
        Box::pin(async { Ok(()) })
    }

    fn remove_agent(&self, request: RemoveAgentMember) -> PortFuture<'_, AgentAccessResult<()>> {
        self.record(format!(
            "remove:{}:{}",
            request.catalog_id, request.agent_id
        ));
        Box::pin(async { Ok(()) })
    }

    fn resolve(&self, request: ResolveJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        *self.resolved.lock().unwrap() = Some(request);
        Box::pin(async { Ok(room()) })
    }

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        *self.redeemed.lock().unwrap() = Some(request);
        let failure = *self.redeem_failure.lock().unwrap();
        Box::pin(async move {
            if let Some(kind) = failure {
                return Err(failure_of(kind));
            }
            Ok(room())
        })
    }
}

fn room() -> RedeemedRoom {
    RedeemedRoom {
        catalog_id: catalog_id(),
        matrix_room_id: MatrixRoomReference::new("!project:matrix.test".to_owned()).unwrap(),
        name: "项目室".to_owned(),
    }
}

fn failure_of(kind: AgentAccessFailureKind) -> AgentAccessFailure {
    if kind == AgentAccessFailureKind::RateLimited {
        AgentAccessFailure::rate_limited("test.redeem", time(4_102_444_800_000))
    } else {
        AgentAccessFailure::new("test.redeem", kind)
    }
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("口令路由不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("口令路由不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        Box::pin(async move {
            assert_eq!(session_secret.expose(), "session-secret");
            assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
            Ok(AuthenticatedPrincipal {
                principal_id: owner_id(),
                matrix_user_id: "@owner:matrix.agent-room.test".to_owned(),
                display_name: "Owner".to_owned(),
                locale: "zh-CN".to_owned(),
                authenticated_at: time(1_700_000_000_000),
                expires_at: time(1_700_028_800_000),
                recently_authenticated: false,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("口令路由不会注销") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("口令路由不会暂停主体") })
    }
}

#[derive(Default)]
struct FakeDevices {
    expected_body: Mutex<Option<String>>,
    expected_target: Mutex<Option<String>>,
}

impl DeviceAuthorizationUseCases for FakeDevices {
    fn register_device(
        &self,
        _request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("口令路由不会注册设备") })
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        assert_eq!(request.access_token.expose(), "device-access-token");
        assert_eq!(request.proof.device_id(), device_id());
        assert_eq!(request.proof.method(), "POST");
        let target = self.expected_target.lock().unwrap().clone();
        assert_eq!(
            Some(request.proof.request_target().to_owned()),
            target,
            "签名覆盖的路径"
        );
        let expected = self
            .expected_body
            .lock()
            .unwrap()
            .take()
            .expect("登记了正文");
        assert_eq!(
            request.proof.body_digest(),
            &SecureSecretFactory.digest(&expected)
        );
        Box::pin(async {
            Ok(AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(owner_id()),
                    matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                    display_name: "Agent Room User".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device_id: device_id(),
                access_token_expires_at: time(1_700_000_900_000),
            })
        })
    }

    fn refresh_device_session<'a>(
        &'a self,
        _request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("口令路由不会刷新设备会话") })
    }

    fn list_devices(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(async { unreachable!("口令路由不会列出设备") })
    }

    fn revoke_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(async { unreachable!("口令路由不会撤销设备") })
    }
}

#[tokio::test]
async fn 查看只返回口令的时间与_agent_成员_不含口令本身() {
    let access = Arc::new(FakeAccess::default());
    let response = app(access.clone(), Arc::default())
        .oneshot(web(Method::GET, "/agent-access", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = json_of(response).await;
    assert_eq!(
        body["joinCode"],
        json!({"createdAtUnixMs": 1_700_000_000_000_i64})
    );
    assert_eq!(body["agents"][0]["agentId"], AGENT_UUID);
    assert_eq!(body["agents"][0]["displayName"], "Scout");
    assert_eq!(body["agents"][0]["ownerDisplayName"], "Bob");
    assert_eq!(body["agents"][0]["status"], "joined");
    assert!(!body.to_string().contains("K7P3"));
    assert_eq!(access.calls(), [format!("inspect:{CATALOG_UUID}")]);
}

#[tokio::test]
async fn 生成与停用口令_移出_agent_都要求可信来源() {
    let access = Arc::new(FakeAccess::default());
    // 没有 Origin 的写请求在认证和用例之前被拒绝。
    let refused = app(access.clone(), Arc::default())
        .oneshot(web(Method::PUT, "/agent-access/code", false))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(access.calls().is_empty());

    let generated = app(access.clone(), Arc::default())
        .oneshot(web(Method::PUT, "/agent-access/code", true))
        .await
        .unwrap();
    assert_eq!(generated.status(), StatusCode::OK);
    assert_eq!(generated.headers()[header::CACHE_CONTROL], "no-store");
    let body = json_of(generated).await;
    assert_eq!(body["code"], "K7P3-Q9XW-2DMA");
    assert_eq!(body["createdAtUnixMs"], 1_700_000_200_000_i64);

    let disabled = app(access.clone(), Arc::default())
        .oneshot(web(Method::DELETE, "/agent-access/code", true))
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::NO_CONTENT);
    let removed = app(access.clone(), Arc::default())
        .oneshot(web(
            Method::DELETE,
            &format!("/agent-access/agents/{AGENT_UUID}"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        access.calls(),
        [
            format!("generate:{CATALOG_UUID}"),
            format!("disable:{CATALOG_UUID}"),
            format!("remove:{CATALOG_UUID}:{AGENT_UUID}"),
        ]
    );
    let invalid = app(access.clone(), Arc::default())
        .oneshot(web(Method::DELETE, "/agent-access/agents/not-a-uuid", true))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn 设备签名查看与兑换口令_转发设备_agent_与口令() {
    let access = Arc::new(FakeAccess::default());
    let body = json!({"code": "k7p3 q9xw 2dma"}).to_string();
    let devices = devices_for(&redeem_path(), &body);
    let response = app(access.clone(), devices)
        .oneshot(signed(&redeem_path(), &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let room = json_of(response).await;
    assert_eq!(room["catalogId"], CATALOG_UUID);
    assert_eq!(room["matrixRoomId"], "!project:matrix.test");
    assert_eq!(room["name"], "项目室");
    let redeemed = access.redeemed.lock().unwrap().clone().expect("转发了请求");
    assert_eq!(redeemed.agent_id, agent_id());
    assert_eq!(redeemed.actor.device_id, device_id());
    assert_eq!(redeemed.code, "k7p3 q9xw 2dma");

    // 只查看房间：同样要求设备签名，转发设备与口令。
    let body = json!({"code": "K7P3-Q9XW-2DMA"}).to_string();
    let devices = devices_for("/join-codes/resolve", &body);
    let response = app(access.clone(), devices)
        .oneshot(signed("/join-codes/resolve", &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_of(response).await["name"], "项目室");
    let resolved = access.resolved.lock().unwrap().clone().expect("转发了查看");
    assert!(
        matches!(&resolved.caller, JoinCodeCaller::Device(actor) if actor.device_id == device_id()),
        "按设备计数：{:?}",
        resolved.caller
    );
    assert_eq!(resolved.code, "K7P3-Q9XW-2DMA");
}

#[tokio::test]
async fn 兑换失败映射成稳定错误码_限流带重试时间() {
    for (kind, status, code) in [
        (
            AgentAccessFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "join_code.not_found",
        ),
        (
            AgentAccessFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "join_code.invalid",
        ),
        (
            AgentAccessFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "join_code.forbidden",
        ),
        (
            AgentAccessFailureKind::RateLimited,
            StatusCode::TOO_MANY_REQUESTS,
            "join_code.rate_limited",
        ),
    ] {
        let access = Arc::new(FakeAccess::default());
        *access.redeem_failure.lock().unwrap() = Some(kind);
        let body = json!({"code": "K7P3-Q9XW-2DMA"}).to_string();
        let devices = devices_for(&redeem_path(), &body);
        let response = app(access, devices)
            .oneshot(signed(&redeem_path(), &body))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        if kind == AgentAccessFailureKind::RateLimited {
            assert!(response.headers().contains_key(header::RETRY_AFTER));
        }
        assert_eq!(json_of(response).await["code"], code);
    }
    // 正文里有多余字段直接拒绝，不进用例。
    let access = Arc::new(FakeAccess::default());
    let body = json!({"code": "K7P3-Q9XW-2DMA", "room": "x"}).to_string();
    let devices = devices_for(&redeem_path(), &body);
    let response = app(access.clone(), devices)
        .oneshot(signed(&redeem_path(), &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(access.redeemed.lock().unwrap().is_none());
}

fn app(access: Arc<FakeAccess>, devices: Arc<FakeDevices>) -> axum::Router {
    let state = PrivateRoomAgentHttpState::new(
        PrivateRoomAgentHttpDependencies {
            access,
            authentication: Arc::new(FakeAuthentication),
            devices,
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn web(method: Method, suffix: &str, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/private-rooms/{CATALOG_UUID}{suffix}"))
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).unwrap()
}

fn redeem_path() -> String {
    format!("/agents/{AGENT_UUID}/join-codes/redeem")
}

fn devices_for(target: &str, body: &str) -> Arc<FakeDevices> {
    let devices = Arc::new(FakeDevices::default());
    *devices.expected_body.lock().unwrap() = Some(body.to_owned());
    *devices.expected_target.lock().unwrap() = Some(target.to_owned());
    devices
}

fn signed(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        )
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn json_of(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn owner_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(OWNER_UUID).unwrap())
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::parse_str(DEVICE_UUID).unwrap())
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(AGENT_UUID).unwrap())
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
