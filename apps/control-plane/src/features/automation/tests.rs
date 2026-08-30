use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    automation::{
        AuthorizeAutomationSend, AutomationAuthorizationOutcome, AutomationResult,
        AutomationSendDenial, AutomationUseCases, CreateAutomationGrant, ListAutomationGrants,
        RevokeAutomationGrant,
    },
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
        DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        RevokedDevice,
    },
    ports::{AutomationGrantRecord, PortFuture, PrincipalAccount, SecretFactory, SecretValue},
};
use agent_room_domain::{
    devices::Device,
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, DeviceId, MessageSubmissionId, PrincipalId,
        RoomCatalogId,
    },
    policy::{
        AutomationAudience, AutomationGrant, AutomationGrantFields, AutomationGrantLimits,
        AutomationGrantScope, AutomationMessageKind, AutomationMessageKinds,
        AutomationUsageSnapshot,
    },
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use axum_extra::extract::cookie::Cookie;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{AutomationHttpDependencies, AutomationHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const GRANT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const SUBMISSION_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

#[derive(Default)]
struct FakeAutomation {
    created: Mutex<Option<CreateAutomationGrant>>,
    listed: AtomicUsize,
    revoked: Mutex<Option<RevokeAutomationGrant>>,
    authorized: Mutex<Option<AuthorizeAutomationSend>>,
}

impl AutomationUseCases for FakeAutomation {
    fn create(
        &self,
        request: CreateAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>> {
        *self.created.lock().expect("授权创建记录锁可用") = Some(request);
        Box::pin(async { Ok(grant_record()) })
    }

    fn list(
        &self,
        _request: ListAutomationGrants,
    ) -> PortFuture<'_, AutomationResult<Vec<AutomationGrantRecord>>> {
        self.listed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(vec![grant_record()]) })
    }

    fn revoke(
        &self,
        request: RevokeAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>> {
        *self.revoked.lock().expect("授权撤销记录锁可用") = Some(request);
        Box::pin(async { Ok(grant_record()) })
    }

    fn authorize_send(
        &self,
        request: AuthorizeAutomationSend,
    ) -> PortFuture<'_, AutomationResult<AutomationAuthorizationOutcome>> {
        *self.authorized.lock().expect("发送授权记录锁可用") = Some(request);
        Box::pin(async {
            Ok(AutomationAuthorizationOutcome::Denied(
                AutomationSendDenial::MatrixPermissionDenied,
            ))
        })
    }
}

#[derive(Default)]
struct FakeAuthentication {
    requirements: Mutex<Vec<AuthenticationRequirement>>,
    attempts: AtomicUsize,
}

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("自动授权路由不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("自动授权路由不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.requirements
            .lock()
            .expect("认证要求记录锁可用")
            .push(requirement);
        Box::pin(async move {
            assert_eq!(session_secret.expose(), "session-secret");
            Ok(authenticated_principal())
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("自动授权路由不会注销") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("自动授权路由不会暂停主体") })
    }
}

#[derive(Default)]
struct FakeDevices {
    expected_body: Mutex<Option<String>>,
    authentications: AtomicUsize,
}

impl DeviceAuthorizationUseCases for FakeDevices {
    fn register_device(
        &self,
        _request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("自动授权路由不会注册设备") })
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.access_token.expose(), "device-access-token");
        assert_eq!(request.proof.device_id(), device_id());
        assert_eq!(request.proof.method(), "POST");
        assert_eq!(request.proof.request_target(), authorization_target());
        let expected = self
            .expected_body
            .lock()
            .expect("设备正文记录锁可用")
            .take()
            .expect("测试必须登记原始正文");
        assert_eq!(
            request.proof.body_digest(),
            &SecureSecretFactory.digest(&expected)
        );
        Box::pin(async { Ok(authenticated_device()) })
    }

    fn refresh_device_session<'a>(
        &'a self,
        _request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("自动授权路由不会刷新设备会话") })
    }

    fn list_devices(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(async { unreachable!("自动授权路由不会列出设备") })
    }

    fn revoke_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(async { unreachable!("自动授权路由不会撤销设备") })
    }
}

#[tokio::test]
async fn 创建授权要求同源近期认证并完整映射限制() {
    let automation = Arc::new(FakeAutomation::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let response = test_router(
        automation.clone(),
        authentication.clone(),
        Arc::new(FakeDevices::default()),
    )
    .oneshot(session_request(
        Method::POST,
        "/automation-grants",
        &creation_body(),
        true,
        true,
    ))
    .await
    .expect("创建授权路由可调用");

    assert_eq!(response.status(), StatusCode::CREATED);
    let payload = response_json(response).await;
    assert_eq!(payload["grantId"], GRANT_UUID);
    assert_eq!(payload["messageKinds"], json!(["room_message", "reply"]));
    assert_eq!(payload["audience"], "known_room_members");
    assert_eq!(
        authentication
            .requirements
            .lock()
            .expect("认证要求记录锁可用")
            .as_slice(),
        &[AuthenticationRequirement::RecentAuthentication]
    );
    let request = automation
        .created
        .lock()
        .expect("授权创建记录锁可用")
        .clone()
        .expect("创建用例已调用");
    assert_eq!(request.grant_id, grant_id());
    assert_eq!(request.scope.agent_id(), agent_id());
    assert_eq!(request.scope.agent_instance_id(), Some(instance_id()));
    assert_eq!(request.scope.room_catalog_id(), catalog_id());
    assert_eq!(request.max_messages_per_minute, 12);
    assert_eq!(request.max_total_messages, Some(240));
    assert_eq!(request.lifetime.value(), 3_600_000);
    assert!(request.impact_acknowledged);
}

#[tokio::test]
async fn 错误来源在认证和创建用例之前失败关闭() {
    let automation = Arc::new(FakeAutomation::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let mut request = session_request(
        Method::POST,
        "/automation-grants",
        &creation_body(),
        true,
        true,
    );
    request.headers_mut().insert(
        header::ORIGIN,
        header::HeaderValue::from_static("https://evil.test"),
    );
    let response = test_router(
        automation.clone(),
        authentication.clone(),
        Arc::new(FakeDevices::default()),
    )
    .oneshot(request)
    .await
    .expect("错误来源请求可调用");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(authentication.attempts.load(Ordering::SeqCst), 0);
    assert!(
        automation
            .created
            .lock()
            .expect("授权创建记录锁可用")
            .is_none()
    );
}

#[tokio::test]
async fn 列表与撤销分别要求活动会话和近期认证() {
    let automation = Arc::new(FakeAutomation::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(
        automation.clone(),
        authentication.clone(),
        Arc::new(FakeDevices::default()),
    );
    let listed = app
        .clone()
        .oneshot(session_request(
            Method::GET,
            "/automation-grants",
            &json!({}),
            false,
            false,
        ))
        .await
        .expect("授权列表路由可调用");
    let revoked = app
        .oneshot(session_request(
            Method::DELETE,
            &format!("/automation-grants/{GRANT_UUID}"),
            &json!({}),
            true,
            false,
        ))
        .await
        .expect("授权撤销路由可调用");

    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(revoked.status(), StatusCode::OK);
    assert_eq!(
        authentication
            .requirements
            .lock()
            .expect("认证要求记录锁可用")
            .as_slice(),
        &[
            AuthenticationRequirement::ActiveSession,
            AuthenticationRequirement::RecentAuthentication,
        ]
    );
    assert_eq!(automation.listed.load(Ordering::SeqCst), 1);
    assert_eq!(
        automation
            .revoked
            .lock()
            .expect("授权撤销记录锁可用")
            .as_ref()
            .expect("撤销用例已调用")
            .grant_id,
        grant_id()
    );
}

#[tokio::test]
async fn 设备签名覆盖精确正文且业务拒绝以显式决策返回() {
    let automation = Arc::new(FakeAutomation::default());
    let devices = Arc::new(FakeDevices::default());
    let body = authorization_body().to_string();
    *devices.expected_body.lock().expect("设备正文记录锁可用") = Some(body.clone());
    let response = test_router(
        automation.clone(),
        Arc::new(FakeAuthentication::default()),
        devices.clone(),
    )
    .oneshot(device_request(&body, true))
    .await
    .expect("发送授权路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "decision": "denied",
            "reason": "automation.matrix_permission_denied",
            "reused": false
        })
    );
    assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
    let request = automation
        .authorized
        .lock()
        .expect("发送授权记录锁可用")
        .clone()
        .expect("发送授权用例已调用");
    assert_eq!(request.grant_id, grant_id());
    assert_eq!(request.submission_id, submission_id());
    assert_eq!(request.agent_instance_id, instance_id());
    assert!(request.is_reply);
}

#[tokio::test]
async fn 缺失设备证明时不触碰设备认证或授权用例() {
    let automation = Arc::new(FakeAutomation::default());
    let devices = Arc::new(FakeDevices::default());
    let response = test_router(
        automation.clone(),
        Arc::new(FakeAuthentication::default()),
        devices.clone(),
    )
    .oneshot(device_request(&authorization_body().to_string(), false))
    .await
    .expect("缺失证明请求可调用");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(devices.authentications.load(Ordering::SeqCst), 0);
    assert!(
        automation
            .authorized
            .lock()
            .expect("发送授权记录锁可用")
            .is_none()
    );
}

fn test_router(
    automation: Arc<FakeAutomation>,
    authentication: Arc<FakeAuthentication>,
    devices: Arc<FakeDevices>,
) -> axum::Router {
    let state = AutomationHttpState::new(
        AutomationHttpDependencies {
            automation,
            authentication,
            devices,
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &Url::parse("http://tauri.localhost").expect("桌面地址有效"),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn session_request(
    method: Method,
    uri: &str,
    body: &Value,
    include_origin: bool,
    include_idempotency_key: bool,
) -> Request<Body> {
    let session = Cookie::new("__Host-agent-room-session", "session-secret");
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, session.to_string());
    if include_origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    if include_idempotency_key {
        request = request.header("idempotency-key", GRANT_UUID);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("会话 HTTP 请求有效")
}

fn device_request(body: &str, include_proof: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(authorization_target())
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header(header::CONTENT_TYPE, "application/json");
    if include_proof {
        request = request
            .header("x-agent-room-device-id", DEVICE_UUID)
            .header("x-agent-room-proof-issued-at", "1700000000000")
            .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
            .header(
                "x-agent-room-proof-signature",
                URL_SAFE_NO_PAD.encode([7_u8; 64]),
            );
    }
    request
        .body(Body::from(body.to_owned()))
        .expect("设备 HTTP 请求有效")
}

fn creation_body() -> Value {
    json!({
        "agentId": AGENT_UUID,
        "agentInstanceId": INSTANCE_UUID,
        "roomCatalogId": CATALOG_UUID,
        "messageKinds": ["room_message", "reply"],
        "audience": "known_room_members",
        "requiresRiskScan": true,
        "maxMessagesPerMinute": 12,
        "maxTotalMessages": 240,
        "lifetimeSeconds": 3600,
        "impactAcknowledged": true
    })
}

fn authorization_body() -> Value {
    json!({
        "submissionId": SUBMISSION_UUID,
        "agentId": AGENT_UUID,
        "agentInstanceId": INSTANCE_UUID,
        "roomCatalogId": CATALOG_UUID,
        "matrixRoomId": "!lobby:matrix.agent-room.test",
        "messageKind": "reply",
        "riskScan": "passed"
    })
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024)
        .await
        .expect("响应正文可读取");
    serde_json::from_slice(&body).expect("响应正文是 JSON")
}

fn grant_record() -> AutomationGrantRecord {
    let starts_at = time(1_700_000_000_000);
    let limits = AutomationGrantLimits::new(12, Some(240), starts_at, time(1_700_003_600_000))
        .expect("测试授权限制有效");
    let message_kinds = AutomationMessageKinds::new([
        AutomationMessageKind::RoomMessage,
        AutomationMessageKind::Reply,
    ])
    .expect("测试消息类型有效");
    let scope = AutomationGrantScope::new(
        agent_id(),
        Some(instance_id()),
        catalog_id(),
        message_kinds,
        AutomationAudience::KnownRoomMembers,
        true,
    )
    .expect("测试授权作用域有效");
    AutomationGrantRecord {
        grant: AutomationGrant::issue(AutomationGrantFields {
            id: grant_id(),
            grantor_id: principal_id(),
            scope,
            limits,
            created_at: starts_at,
        })
        .expect("测试授权有效"),
        usage: AutomationUsageSnapshot::default(),
    }
}

fn authenticated_principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: true,
    }
}

fn authenticated_device() -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(principal_id()),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: device_id(),
        access_token_expires_at: time(1_700_000_900_000),
    }
}

fn authorization_target() -> String {
    format!("/automation-grants/{GRANT_UUID}/authorizations")
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(uuid(DEVICE_UUID))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_UUID))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid(INSTANCE_UUID))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(CATALOG_UUID))
}

fn grant_id() -> AutomationGrantId {
    AutomationGrantId::from_uuid(uuid(GRANT_UUID))
}

fn submission_id() -> MessageSubmissionId {
    MessageSubmissionId::from_uuid(uuid(SUBMISSION_UUID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
