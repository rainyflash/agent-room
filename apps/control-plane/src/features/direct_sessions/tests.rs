use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    direct_sessions::{
        DirectContactView, DirectSessionResult, DirectSessionUseCases, DirectSessionView,
        InspectDirectSession, ListDirectSessions, OpenDirectSession, SetDirectAgentBlock,
    },
    ports::{DirectAgentProfile, DirectSessionRecord, MatrixUserId, PortFuture, SecretValue},
};
use agent_room_domain::{
    direct_sessions::{DirectContactPolicy, DirectSession},
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility,
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

use super::{DirectSessionHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e53";
const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e54";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedCall {
    operation: &'static str,
    target_agent_id: Option<AgentId>,
    catalog_id: Option<RoomCatalogId>,
    blocked: Option<bool>,
}

struct FakeDirectSessions {
    calls: Mutex<Vec<ObservedCall>>,
    view: DirectSessionView,
}

impl FakeDirectSessions {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            view: session_view(false),
        }
    }

    fn record_session(
        &self,
        call: ObservedCall,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>> {
        self.calls
            .lock()
            .expect("直接会话调用记录锁可用")
            .push(call);
        let view = self.view.clone();
        Box::pin(async move { Ok(view) })
    }
}

impl DirectSessionUseCases for FakeDirectSessions {
    fn open(
        &self,
        request: OpenDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>> {
        self.record_session(ObservedCall {
            operation: "open",
            target_agent_id: Some(request.target_agent_id),
            catalog_id: None,
            blocked: None,
        })
    }

    fn inspect(
        &self,
        request: InspectDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>> {
        self.record_session(ObservedCall {
            operation: "inspect",
            target_agent_id: None,
            catalog_id: Some(request.catalog_id),
            blocked: None,
        })
    }

    fn list(
        &self,
        _request: ListDirectSessions,
    ) -> PortFuture<'_, DirectSessionResult<Vec<DirectSessionView>>> {
        self.calls
            .lock()
            .expect("直接会话调用记录锁可用")
            .push(ObservedCall {
                operation: "list",
                target_agent_id: None,
                catalog_id: None,
                blocked: None,
            });
        let view = self.view.clone();
        Box::pin(async move { Ok(vec![view]) })
    }

    fn set_block(
        &self,
        request: SetDirectAgentBlock,
    ) -> PortFuture<'_, DirectSessionResult<DirectContactView>> {
        self.calls
            .lock()
            .expect("直接会话调用记录锁可用")
            .push(ObservedCall {
                operation: "set-block",
                target_agent_id: Some(request.target_agent_id),
                catalog_id: None,
                blocked: Some(request.blocked),
            });
        let target = target_profile();
        let policy =
            DirectContactPolicy::restore(principal_id(), agent_id(), request.blocked, false);
        Box::pin(async move {
            Ok(DirectContactView {
                target,
                contact_policy: policy,
            })
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
        Box::pin(async { unreachable!("直接会话路由不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("直接会话路由不会完成登录") })
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
        Box::pin(async { unreachable!("直接会话路由不会注销") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("直接会话路由不会暂停主体") })
    }
}

#[tokio::test]
async fn 打开直接会话要求同源活动会话并返回权威目标资料() {
    let sessions = Arc::new(FakeDirectSessions::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let response = test_router(sessions.clone(), authentication.clone())
        .oneshot(request(
            Method::POST,
            "/direct-sessions",
            &json!({ "targetAgentId": AGENT_UUID }),
            true,
        ))
        .await
        .expect("打开直接会话路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let payload = response_json(response).await;
    assert_eq!(payload["catalogId"], CATALOG_UUID);
    assert_eq!(payload["matrixRoomId"], "!direct:matrix.agent-room.test");
    assert_eq!(payload["target"]["agentId"], AGENT_UUID);
    assert_eq!(payload["target"]["displayName"], "Build Agent");
    assert_eq!(payload["contactPolicy"]["presenceDisclosure"], "coarse");
    assert_eq!(
        authentication
            .calls
            .lock()
            .expect("认证记录锁可用")
            .as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
    assert_eq!(
        sessions
            .calls
            .lock()
            .expect("直接会话记录锁可用")
            .as_slice(),
        &[ObservedCall {
            operation: "open",
            target_agent_id: Some(agent_id()),
            catalog_id: None,
            blocked: None,
        }]
    );
}

#[tokio::test]
async fn 屏蔽接口返回隐藏在线状态并停止投递() {
    let sessions = Arc::new(FakeDirectSessions::new());
    let response = test_router(sessions.clone(), Arc::new(FakeAuthentication::default()))
        .oneshot(request(
            Method::PUT,
            &format!("/direct-contacts/{AGENT_UUID}/block"),
            &json!({ "blocked": true }),
            true,
        ))
        .await
        .expect("屏蔽路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["contactPolicy"]["principalBlocksAgent"], true);
    assert_eq!(payload["contactPolicy"]["deliveryAllowed"], false);
    assert_eq!(payload["contactPolicy"]["presenceDisclosure"], "hidden");
    assert_eq!(
        sessions
            .calls
            .lock()
            .expect("直接会话记录锁可用")
            .as_slice(),
        &[ObservedCall {
            operation: "set-block",
            target_agent_id: Some(agent_id()),
            catalog_id: None,
            blocked: Some(true),
        }]
    );
}

#[tokio::test]
async fn 错误来源在认证和用例之前失败关闭() {
    let sessions = Arc::new(FakeDirectSessions::new());
    let authentication = Arc::new(FakeAuthentication::default());
    let mut request = request(
        Method::POST,
        "/direct-sessions",
        &json!({ "targetAgentId": AGENT_UUID }),
        true,
    );
    request.headers_mut().insert(
        header::ORIGIN,
        header::HeaderValue::from_static("https://evil.test"),
    );
    let response = test_router(sessions.clone(), authentication.clone())
        .oneshot(request)
        .await
        .expect("错误来源请求可调用");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(authentication.attempts.load(Ordering::SeqCst), 0);
    assert!(
        sessions
            .calls
            .lock()
            .expect("直接会话记录锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 会话列表只需活动会话并返回稳定索引() {
    let response = test_router(
        Arc::new(FakeDirectSessions::new()),
        Arc::new(FakeAuthentication::default()),
    )
    .oneshot(request(Method::GET, "/direct-sessions", &json!({}), false))
    .await
    .expect("列表路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["sessions"][0]["catalogId"], CATALOG_UUID);
}

fn test_router(
    sessions: Arc<FakeDirectSessions>,
    authentication: Arc<FakeAuthentication>,
) -> axum::Router {
    let state = DirectSessionHttpState::new(
        sessions,
        authentication,
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &Url::parse("http://tauri.localhost").expect("桌面地址有效"),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn request(method: Method, uri: &str, body: &Value, include_origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if include_origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("HTTP 请求有效")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 8 * 1_024)
        .await
        .expect("响应正文可读");
    serde_json::from_slice(&body).expect("响应是 JSON")
}

fn session_view(blocked: bool) -> DirectSessionView {
    DirectSessionView {
        record: direct_record(),
        target: target_profile(),
        contact_policy: DirectContactPolicy::restore(principal_id(), agent_id(), blocked, false),
    }
}

fn direct_record() -> DirectSessionRecord {
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::Direct,
            slug: None,
            name: "Build Agent".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(principal_id()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: RoomCatalogStatus::Frozen,
        },
    )
    .expect("直接会话目录有效");
    DirectSessionRecord::new(
        catalog,
        None,
        DirectSession::reserve(catalog_id(), principal_id(), agent_id()),
    )
    .expect("直接会话预留有效")
    .activate(
        RoomInstanceId::from_uuid(uuid(INSTANCE_UUID)),
        MatrixRoomReference::new("!direct:matrix.agent-room.test".to_owned())
            .expect("Matrix 房间标识有效"),
    )
    .expect("直接会话可激活")
}

fn target_profile() -> DirectAgentProfile {
    DirectAgentProfile {
        agent_id: agent_id(),
        matrix_user_id: MatrixUserId::new("@_agent_build:matrix.agent-room.test")
            .expect("Agent Matrix 标识有效"),
        display_name: "Build Agent".to_owned(),
        avatar_content_id: None,
    }
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@principal:matrix.agent-room.test".to_owned(),
        display_name: "Principal".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_028_800_000),
        recently_authenticated: true,
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_UUID))
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
