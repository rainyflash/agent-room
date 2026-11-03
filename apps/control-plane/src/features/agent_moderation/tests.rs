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
    moderation::{
        AgentModerate, AgentModerationFailure, AgentModerationFailureKind, AgentModerationLedger,
        AgentModerationResult, AgentModerationScope, AgentModerationUseCases, AgentModerationView,
        ListAgentModeration,
    },
    ports::{
        AgentModerationAttribution, AgentModerationRecord, MatrixUserId, PortFuture,
        PrincipalAccount, SecretFactory, SecretValue,
    },
};
use agent_room_domain::{
    devices::Device,
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, DeviceId, ModerationActionId, PrincipalId, RoomCatalogId,
        RoomModerationGrantId,
    },
    moderation::{
        ModerationAction, ModerationActionKind, ModerationReason, ModerationTarget,
        ModerationTargetKind,
    },
    room_moderation_grants::{
        AgentModerationNote, RoomModerationPower, RoomModerationPowers, RoomModerationRules,
    },
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{AgentModerationHttpDependencies, AgentModerationHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const ACTION_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const GRANT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e4a";
const TARGET_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e4b";
const NOW: i64 = 1_700_000_100_000;
const EXPIRES_AT: i64 = 1_702_592_100_000;

/// 记下收到的请求；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeModeration {
    scopes: Mutex<Vec<AgentModerationScope>>,
    moderated: Mutex<Vec<AgentModerate>>,
    listed: Mutex<Vec<ListAgentModeration>>,
    failure: Mutex<Option<AgentModerationFailure>>,
}

impl FakeModeration {
    fn answer<T>(&self, value: T) -> AgentModerationResult<T> {
        self.failure.lock().unwrap().take().map_or(Ok(value), Err)
    }
}

impl AgentModerationUseCases for FakeModeration {
    fn rules(
        &self,
        scope: AgentModerationScope,
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationView>> {
        self.scopes.lock().unwrap().push(scope);
        let answer = self.answer(AgentModerationView {
            grant_id: grant_id(),
            rules: RoomModerationRules::parse("发广告的禁言十分钟。").unwrap(),
            powers: RoomModerationPowers::new([
                RoomModerationPower::Mute,
                RoomModerationPower::Kick,
            ])
            .unwrap(),
            expires_at: time(EXPIRES_AT),
        });
        Box::pin(async move { answer })
    }

    fn moderate(
        &self,
        request: AgentModerate,
    ) -> PortFuture<'_, AgentModerationResult<ModerationAction>> {
        let expires_at = request
            .duration
            .map(|duration| time(NOW).checked_add(duration).unwrap());
        // 解封交回它撤回的那条封禁。
        let kind = match request.power {
            RoomModerationPower::Mute => ModerationActionKind::Mute,
            RoomModerationPower::Kick => ModerationActionKind::Kick,
            RoomModerationPower::Ban | RoomModerationPower::Unban => ModerationActionKind::Ban,
        };
        let mut action = ModerationAction::reserve(
            request.action_id,
            None,
            PrincipalId::from_uuid(uuid(PRINCIPAL_UUID)),
            request.scope.room_catalog_id,
            kind,
            ModerationTarget::new(ModerationTargetKind::Principal, TARGET_UUID).unwrap(),
            request.reason.unwrap_or(ModerationReason::Spam),
            time(NOW),
            expires_at,
        )
        .unwrap();
        action.mark_applied().unwrap();
        if request.power == RoomModerationPower::Unban {
            action.reverse(time(NOW)).unwrap();
        }
        self.moderated.lock().unwrap().push(request);
        let answer = self.answer(action);
        Box::pin(async move { answer })
    }

    fn list(
        &self,
        request: ListAgentModeration,
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationLedger>> {
        self.listed.lock().unwrap().push(request);
        let record = |note: &str| AgentModerationRecord {
            attribution: AgentModerationAttribution {
                action_id: action_id(),
                agent_id: AgentId::from_uuid(uuid(AGENT_UUID)),
                grant_id: grant_id(),
                note: AgentModerationNote::parse(note).unwrap(),
            },
            agent_display_name: "门卫 #2316".to_owned(),
        };
        let answer = self.answer(AgentModerationLedger {
            actions: vec![record("连发了五条广告链接。")],
            reversals: vec![record("他说以后不发了。")],
        });
        Box::pin(async move { answer })
    }
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
        Box::pin(async { Ok(principal()) })
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

/// 核对签名证明里的方法、路径和正文摘要，认出这台设备。
struct FakeDevices {
    method: &'static str,
    target: String,
    body: String,
}

impl DeviceAuthorizationUseCases for FakeDevices {
    fn register_device(
        &self,
        _request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("不注册设备") })
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        assert_eq!(request.access_token.expose(), "device-access-token");
        assert_eq!(request.proof.method(), self.method);
        assert_eq!(request.proof.request_target(), self.target);
        assert_eq!(
            request.proof.body_digest(),
            &SecureSecretFactory.digest(&self.body)
        );
        Box::pin(async { Ok(device()) })
    }

    fn refresh_device_session<'a>(
        &'a self,
        _request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("不刷新设备会话") })
    }

    fn list_devices(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(async { unreachable!("不列设备") })
    }

    fn revoke_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(async { unreachable!("不撤销设备") })
    }
}

#[tokio::test]
async fn agent_读授权_签名核对方法和路径_回规则和能做的几样() {
    let moderation = Arc::new(FakeModeration::default());
    let response = test_router(moderation.clone(), devices("GET", &rules_path(), ""))
        .oneshot(signed(Method::GET, &rules_path(), None, ""))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert_eq!(
        response_json(response).await,
        json!({
            "grantId": GRANT_UUID,
            "rules": "发广告的禁言十分钟。",
            "actions": ["mute", "kick"],
            "expiresAtUnixMs": EXPIRES_AT
        })
    );
    assert_eq!(*moderation.scopes.lock().unwrap(), vec![scope()]);
}

#[tokio::test]
async fn agent_禁言带着动作_id_和理由_回_201() {
    let moderation = Arc::new(FakeModeration::default());
    let body = json!({
        "action": "mute",
        "target": " @spammer:matrix.agent-room.test ",
        "reason": "spam",
        "note": "  连发了五条广告链接，按“发广告的禁言十分钟”。 ",
        "durationSeconds": 600
    })
    .to_string();
    let response = test_router(moderation.clone(), devices("POST", &actions_path(), &body))
        .oneshot(signed(
            Method::POST,
            &actions_path(),
            Some(ACTION_UUID),
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response_json(response).await,
        json!({
            "actionId": ACTION_UUID,
            "action": "mute",
            "status": "applied",
            "expiresAtUnixMs": NOW + 600_000
        })
    );
    let requests = moderation.moderated.lock().unwrap();
    assert_eq!(
        *requests,
        vec![AgentModerate {
            scope: scope(),
            action_id: action_id(),
            power: RoomModerationPower::Mute,
            target: MatrixUserId::new("@spammer:matrix.agent-room.test").unwrap(),
            reason: Some(ModerationReason::Spam),
            note: AgentModerationNote::parse("连发了五条广告链接，按“发广告的禁言十分钟”。")
                .unwrap(),
            duration: Some(DurationMillis::new(600_000).unwrap()),
        }]
    );
}

#[tokio::test]
async fn agent_解封不用理由_回它撤回的那条封禁() {
    let moderation = Arc::new(FakeModeration::default());
    let body = json!({
        "action": "unban",
        "target": "@spammer:matrix.agent-room.test",
        "note": "他说以后不发了。"
    })
    .to_string();
    let response = test_router(moderation.clone(), devices("POST", &actions_path(), &body))
        .oneshot(signed(
            Method::POST,
            &actions_path(),
            Some(ACTION_UUID),
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response_json(response).await,
        json!({"actionId": ACTION_UUID, "action": "ban", "status": "reversed"})
    );
    let requests = moderation.moderated.lock().unwrap();
    assert_eq!(requests[0].power, RoomModerationPower::Unban);
    assert_eq!(requests[0].reason, None);
    assert_eq!(requests[0].duration, None);
}

#[tokio::test]
async fn 没有动作_id_或者正文认不出来时不碰用例() {
    let moderation = Arc::new(FakeModeration::default());
    let good = json!({"action": "mute", "target": "@a:matrix.agent-room.test",
        "reason": "spam", "note": "广告"})
    .to_string();
    let cases = [
        (
            None,
            good.clone(),
            "agent_moderation.invalid_idempotency_key",
        ),
        (
            Some("not-a-uuid"),
            good,
            "agent_moderation.invalid_idempotency_key",
        ),
        (
            Some(ACTION_UUID),
            json!({"action": "warn", "target": "@a:matrix.agent-room.test",
                "reason": "spam", "note": "广告"})
            .to_string(),
            "agent_moderation.invalid",
        ),
        (
            Some(ACTION_UUID),
            json!({"action": "mute", "target": "@a:matrix.agent-room.test",
                "reason": "spam", "note": "  "})
            .to_string(),
            "agent_moderation.invalid",
        ),
        (
            Some(ACTION_UUID),
            json!({"action": "ban", "target": "@a:matrix.agent-room.test",
                "reason": "rude", "note": "广告"})
            .to_string(),
            "agent_moderation.invalid",
        ),
        (
            Some(ACTION_UUID),
            json!({"action": "mute", "target": "not-a-matrix-id",
                "reason": "spam", "note": "广告", "extra": 1})
            .to_string(),
            "agent_moderation.invalid",
        ),
    ];
    for (key, body, code) in cases {
        let response = test_router(moderation.clone(), devices("POST", &actions_path(), &body))
            .oneshot(signed(Method::POST, &actions_path(), key, &body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(response_json(response).await["code"], code, "{body}");
    }
    assert!(moderation.moderated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 用例的失败照种类回英文说明() {
    let cases = [
        (
            AgentModerationFailureKind::NotGranted,
            StatusCode::NOT_FOUND,
            "agent_moderation.not_granted",
        ),
        (
            AgentModerationFailureKind::NotAllowed,
            StatusCode::FORBIDDEN,
            "agent_moderation.not_allowed",
        ),
        (
            AgentModerationFailureKind::Protected,
            StatusCode::FORBIDDEN,
            "agent_moderation.protected",
        ),
        (
            AgentModerationFailureKind::TargetNotFound,
            StatusCode::NOT_FOUND,
            "agent_moderation.target_not_found",
        ),
        (
            AgentModerationFailureKind::NotInRoom,
            StatusCode::CONFLICT,
            "agent_moderation.not_in_room",
        ),
        (
            AgentModerationFailureKind::NotBanned,
            StatusCode::CONFLICT,
            "agent_moderation.not_banned",
        ),
        (
            AgentModerationFailureKind::BannedByPerson,
            StatusCode::FORBIDDEN,
            "agent_moderation.banned_by_person",
        ),
        (
            AgentModerationFailureKind::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "agent_moderation.unauthorized",
        ),
    ];
    for (kind, status, code) in cases {
        let moderation = Arc::new(FakeModeration::default());
        *moderation.failure.lock().unwrap() = Some(AgentModerationFailure::new("test", kind));
        let response = test_router(moderation, devices("GET", &rules_path(), ""))
            .oneshot(signed(Method::GET, &rules_path(), None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{kind:?}");
        let body = response_json(response).await;
        assert_eq!(body["code"], code, "{kind:?}");
        assert!(body["message"].as_str().unwrap().is_ascii(), "{kind:?}");
    }
}

#[tokio::test]
async fn 房主看台账时知道哪些是_agent_做的() {
    let moderation = Arc::new(FakeModeration::default());
    let path = format!("/rooms/{CATALOG_UUID}/moderation/actions/agents");
    let response = test_router(moderation.clone(), devices("GET", "", ""))
        .oneshot(
            Request::builder()
                .uri(&path)
                .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "actions": [{
                "actionId": ACTION_UUID,
                "agentId": AGENT_UUID,
                "agentName": "门卫 #2316",
                "grantId": GRANT_UUID,
                "note": "连发了五条广告链接。"
            }],
            "reversals": [{
                "actionId": ACTION_UUID,
                "agentId": AGENT_UUID,
                "agentName": "门卫 #2316",
                "grantId": GRANT_UUID,
                "note": "他说以后不发了。"
            }]
        })
    );
    let listed = moderation.listed.lock().unwrap();
    assert_eq!(listed[0].actor, principal());
    assert_eq!(listed[0].room_catalog_id, catalog_id());
}

fn test_router(moderation: Arc<FakeModeration>, devices: FakeDevices) -> Router {
    let state = AgentModerationHttpState::new(AgentModerationHttpDependencies {
        moderation,
        authentication: Arc::new(FakeAuthentication),
        devices: Arc::new(devices),
        secrets: Arc::new(SecureSecretFactory),
    });
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn devices(method: &'static str, target: &str, body: &str) -> FakeDevices {
    FakeDevices {
        method,
        target: target.to_owned(),
        body: body.to_owned(),
    }
}

fn signed(method: Method, path: &str, idempotency_key: Option<&str>, body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        )
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = idempotency_key {
        request = request.header("idempotency-key", key);
    }
    request.body(Body::from(body.to_owned())).unwrap()
}

fn rules_path() -> String {
    format!("/agents/{AGENT_UUID}/instances/{INSTANCE_UUID}/rooms/{CATALOG_UUID}/moderation")
}

fn actions_path() -> String {
    format!("{}/actions", rules_path())
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn scope() -> AgentModerationScope {
    AgentModerationScope {
        device: device(),
        agent_id: AgentId::from_uuid(uuid(AGENT_UUID)),
        agent_instance_id: AgentInstanceId::from_uuid(uuid(INSTANCE_UUID)),
        room_catalog_id: catalog_id(),
    }
}

fn principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(uuid(PRINCIPAL_UUID)),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: false,
    }
}

fn device() -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: DeviceId::from_uuid(uuid(DEVICE_UUID)),
        access_token_expires_at: time(1_700_000_900_000),
    }
}

fn action_id() -> ModerationActionId {
    ModerationActionId::from_uuid(uuid(ACTION_UUID))
}

fn grant_id() -> RoomModerationGrantId {
    RoomModerationGrantId::from_uuid(uuid(GRANT_UUID))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(CATALOG_UUID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
