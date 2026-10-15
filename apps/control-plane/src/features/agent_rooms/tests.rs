use std::sync::{Arc, Mutex};

use agent_room_application::{
    agent_lobbies::{AgentLobbyEntryFailure, AgentLobbyEntryResult},
    agent_room_placements::{
        AgentRoom, AgentRoomActor, AgentRoomPlacementUseCases, BringAgentIntoRoom, BroughtAgent,
        LeaveAgentRoom, LeaveAllAgentRooms, LeftAgentRoom, ListAgentRooms,
    },
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
        DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        RevokedDevice,
    },
    ports::{AgentRoomKind, PortFuture, PrincipalAccount, SecretFactory, SecretValue},
};
use agent_room_domain::{
    devices::Device,
    identity::Principal,
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId, RoomCatalogId},
    rooms::RoomCatalogStatus,
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
    routing::post,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{AgentRoomHttpDependencies, AgentRoomHttpState, router};

const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

/// 记下收到的请求，按预先给的结果回答。
#[derive(Default)]
struct FakePlacements {
    listed: Mutex<Vec<ListAgentRooms>>,
    brought: Mutex<Vec<BringAgentIntoRoom>>,
    left: Mutex<Vec<LeaveAgentRoom>>,
    /// 带进房间回答什么；没给就是进去了一个。
    bring_outcome: Mutex<Option<BroughtAgent>>,
    failure: Mutex<Option<AgentLobbyEntryFailure>>,
}

impl AgentRoomPlacementUseCases for FakePlacements {
    fn rooms(
        &self,
        request: ListAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<Vec<AgentRoom>>> {
        self.listed.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move {
            failure.map_or_else(
                || {
                    Ok(vec![
                        AgentRoom {
                            catalog_id: catalog_id(),
                            kind: AgentRoomKind::PrivateRoom,
                            name: "周报".to_owned(),
                            status: RoomCatalogStatus::Archived,
                        },
                        AgentRoom {
                            catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(9)),
                            kind: AgentRoomKind::PublicLobby,
                            name: "大厅".to_owned(),
                            status: RoomCatalogStatus::Active,
                        },
                    ])
                },
                Err,
            )
        })
    }

    fn bring(
        &self,
        request: BringAgentIntoRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<BroughtAgent>> {
        self.brought.lock().unwrap().push(request);
        let outcome = self
            .bring_outcome
            .lock()
            .unwrap()
            .take()
            .unwrap_or(BroughtAgent::Entered(1));
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or(Ok(outcome), Err) })
    }

    fn leave(
        &self,
        request: LeaveAgentRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<LeftAgentRoom>> {
        let left = matches!(request.actor, AgentRoomActor::Device { .. });
        self.left.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or(Ok(LeftAgentRoom { left }), Err) })
    }

    fn leave_all(
        &self,
        _request: LeaveAllAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<usize>> {
        Box::pin(async { unreachable!("房间路由不离开所有房间") })
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

/// 核对签名证明里的方法、路径和空正文，认出这台设备。
struct FakeDevices;

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
        assert_eq!(request.proof.method(), "DELETE");
        assert_eq!(request.proof.request_target(), entry_path());
        assert_eq!(request.proof.body_digest(), &SecureSecretFactory.digest(""));
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
async fn 设备签名的离开和进房间同一个路径_合并以后两个方法各走各的() {
    let placements = Arc::new(FakePlacements::default());
    // 大厅路由在同一个路径上挂着进房间的 POST，和控制面组装时一样合并。
    let app = Router::new()
        .route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            post(|| async { StatusCode::IM_A_TEAPOT }),
        )
        .merge(test_router(placements.clone()))
        .layer(middleware::from_fn(crate::correlation::attach));

    let entered = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(entry_path())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let left = app.oneshot(signed_leave()).await.unwrap();

    assert_eq!(entered.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(left.status(), StatusCode::OK);
    assert_eq!(
        left.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert_eq!(response_json(left).await, json!({ "left": true }));
    let requests = placements.left.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].agent_id, agent_id());
    assert_eq!(requests[0].catalog_id, catalog_id());
    assert_eq!(
        requests[0].actor,
        AgentRoomActor::Device {
            device: device(),
            agent_instance_id: AgentInstanceId::from_uuid(uuid(INSTANCE_UUID)),
        }
    );
}

#[tokio::test]
async fn 设备离开不带签名时不碰用例() {
    let placements = Arc::new(FakePlacements::default());
    let response = test_router(placements.clone())
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(entry_path())
                .header(header::AUTHORIZATION, "Bearer device-access-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(placements.left.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 主人看到它在哪些房间_归档的写明() {
    let placements = Arc::new(FakePlacements::default());
    let response = test_router(placements.clone())
        .oneshot(web(
            Method::GET,
            &format!("/agents/{AGENT_UUID}/rooms"),
            false,
        ))
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
            "rooms": [
                {
                    "catalogId": CATALOG_UUID,
                    "kind": "private_room",
                    "name": "周报",
                    "archived": true
                },
                {
                    "catalogId": "00000000-0000-0000-0000-000000000009",
                    "kind": "public_lobby",
                    "name": "大厅",
                    "archived": false
                }
            ]
        })
    );
    let requests = placements.listed.lock().unwrap();
    assert_eq!(requests[0].actor.principal_id, principal().principal_id);
    assert_eq!(requests[0].agent_id, agent_id());
}

#[tokio::test]
async fn 主人让它离开要可信的来源() {
    let placements = Arc::new(FakePlacements::default());
    let path = format!("/agents/{AGENT_UUID}/rooms/{CATALOG_UUID}");

    let refused = test_router(placements.clone())
        .oneshot(web(Method::DELETE, &path, false))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(refused).await["code"],
        "agent_room.invalid_origin"
    );
    assert!(placements.left.lock().unwrap().is_empty());

    let response = test_router(placements.clone())
        .oneshot(web(Method::DELETE, &path, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({ "left": false }));
    let requests = placements.left.lock().unwrap();
    assert_eq!(requests[0].actor, AgentRoomActor::Owner(principal()));
    assert_eq!(requests[0].catalog_id, catalog_id());
}

#[tokio::test]
async fn 主人带进房间_要可信的来源和房间编号_回进去了几个() {
    let placements = Arc::new(FakePlacements::default());
    let path = format!("/agents/{AGENT_UUID}/rooms");
    let body = json!({ "catalogId": CATALOG_UUID }).to_string();

    let refused = test_router(placements.clone())
        .oneshot(web_with_body(Method::POST, &path, false, &body))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(refused).await["code"],
        "agent_room.invalid_origin"
    );

    for invalid in [
        String::new(),
        json!({ "catalogId": "not-a-uuid" }).to_string(),
        json!({ "catalogId": CATALOG_UUID, "extra": true }).to_string(),
    ] {
        let response = test_router(placements.clone())
            .oneshot(web_with_body(Method::POST, &path, true, &invalid))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{invalid}");
        assert_eq!(
            response_json(response).await["code"],
            "agent_room.invalid_body"
        );
    }
    assert!(placements.brought.lock().unwrap().is_empty());

    let response = test_router(placements.clone())
        .oneshot(web_with_body(Method::POST, &path, true, &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert_eq!(response_json(response).await, json!({ "entered": 1 }));
    let requests = placements.brought.lock().unwrap();
    assert_eq!(requests[0].actor, principal());
    assert_eq!(requests[0].agent_id, agent_id());
    assert_eq!(requests[0].catalog_id, catalog_id());
}

#[tokio::test]
async fn 带进房间时大厅正忙回_503_带_retry_after_带不了回_403() {
    let placements = Arc::new(FakePlacements::default());
    let path = format!("/agents/{AGENT_UUID}/rooms");
    let body = json!({ "catalogId": CATALOG_UUID }).to_string();
    let retry_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    *placements.bring_outcome.lock().unwrap() = Some(BroughtAgent::Busy {
        retry_at: time(i64::try_from(retry_at).unwrap() + 2_500),
    });
    let busy = test_router(placements.clone())
        .oneshot(web_with_body(Method::POST, &path, true, &body))
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    let retry_after: u64 = busy.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=3).contains(&retry_after), "{retry_after}");
    assert_eq!(response_json(busy).await["code"], "agent_room.busy");

    *placements.failure.lock().unwrap() = Some(AgentLobbyEntryFailure::Unauthorized);
    let refused = test_router(placements.clone())
        .oneshot(web_with_body(Method::POST, &path, true, &body))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(refused).await["code"], "lobby.unauthorized");
}

#[tokio::test]
async fn 找不到_agent_回_404_编号不对回_400() {
    let placements = Arc::new(FakePlacements::default());
    *placements.failure.lock().unwrap() = Some(AgentLobbyEntryFailure::NotFound);
    let missing = test_router(placements.clone())
        .oneshot(web(
            Method::GET,
            &format!("/agents/{AGENT_UUID}/rooms"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(missing).await["code"], "lobby.not_found");

    let invalid = test_router(placements.clone())
        .oneshot(web(Method::GET, "/agents/not-a-uuid/rooms", false))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(invalid).await["code"],
        "agent_room.invalid_resource_id"
    );
}

fn test_router(placements: Arc<FakePlacements>) -> Router {
    let state = AgentRoomHttpState::new(
        AgentRoomHttpDependencies {
            placements,
            authentication: Arc::new(FakeAuthentication),
            devices: Arc::new(FakeDevices),
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn signed_leave() -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(entry_path())
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        )
        .body(Body::empty())
        .unwrap()
}

fn web(method: Method, path: &str, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).unwrap()
}

fn web_with_body(method: Method, path: &str, origin: bool, body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .header(header::CONTENT_TYPE, "application/json");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::from(body.to_owned())).unwrap()
}

fn entry_path() -> String {
    format!("/agents/{AGENT_UUID}/instances/{INSTANCE_UUID}/lobbies/{CATALOG_UUID}/entry")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
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

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_UUID))
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
