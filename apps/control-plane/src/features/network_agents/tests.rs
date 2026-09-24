use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentFailure, NetworkAgentFailureKind,
        NetworkAgentPendingExit, NetworkAgentResult, NetworkAgentRoom, NetworkAgentSession,
        NetworkAgentUseCases, NetworkAgentView,
    },
    ports::{Clock, NetworkAgentAckOutcome, PortFuture, SecretValue},
};
use agent_room_domain::{
    ids::{AgentId, NetworkAgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use agent_room_identity_adapter::{NetworkAgentSealKey, NetworkSourceDigester};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{NetworkAgentHttpState, render_guide, router};
use crate::network_gateway::{
    NetworkAgentMessageDraft, NetworkAgentMessages, NetworkAgentMessaging, NetworkAgentSentMessage,
    NetworkGatewayFailure,
};
use agent_room_application::network_agents::NetworkAgentPolicy;
use agent_room_domain::ids::MessageSubmissionId;

const NETWORK_AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";
const TOKEN: &str = "network-agent-token";
const SUBMISSION_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e53";

#[derive(Default)]
struct FakeAgents {
    created: Mutex<Vec<CreateNetworkAgent>>,
    tokens: Mutex<Vec<String>>,
    failure: Mutex<Option<NetworkAgentFailure>>,
}

impl FakeAgents {
    fn failing(failure: NetworkAgentFailure) -> Arc<Self> {
        let agents = Arc::new(Self::default());
        *agents.failure.lock().unwrap() = Some(failure);
        agents
    }

    fn created(&self) -> Vec<CreateNetworkAgent> {
        self.created.lock().unwrap().clone()
    }

    fn tokens(&self) -> Vec<String> {
        self.tokens.lock().unwrap().clone()
    }

    fn authenticate(&self, token: &str) -> NetworkAgentResult<()> {
        self.tokens.lock().unwrap().push(token.to_owned());
        if let Some(failure) = self.failure.lock().unwrap().clone() {
            return Err(failure);
        }
        if token == TOKEN {
            Ok(())
        } else {
            Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ))
        }
    }
}

impl NetworkAgentUseCases for FakeAgents {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        self.created.lock().unwrap().push(request.clone());
        let failure = self.failure.lock().unwrap().clone();
        Box::pin(async move {
            if let Some(failure) = failure {
                return Err(failure);
            }
            Ok(CreatedNetworkAgent {
                network_agent_id: NetworkAgentId::from_uuid(uuid(NETWORK_AGENT_UUID)),
                agent_id: agent_id(),
                display_name: format!("{} 2", request.name),
                token: SecretValue::new(TOKEN).unwrap(),
                room: NetworkAgentRoom {
                    catalog_id: RoomCatalogId::from_uuid(uuid(CATALOG_UUID)),
                    matrix_room_id: MatrixRoomReference::new("!lobby:matrix.test".to_owned())
                        .unwrap(),
                    name: "Agent Room 大厅".to_owned(),
                },
            })
        })
    }

    fn me<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
        let result = self.authenticate(token).map(|()| NetworkAgentView {
            network_agent_id: NetworkAgentId::from_uuid(uuid(NETWORK_AGENT_UUID)),
            agent_id: agent_id(),
            display_name: "Scout".to_owned(),
            created_at: time(1_700_000_000_000),
            rooms: vec![lobby()],
        });
        Box::pin(async move { result })
    }

    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
        let result = self.authenticate(token);
        Box::pin(async move { result })
    }

    fn session<'a>(
        &'a self,
        _token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        unreachable!("路由测试里收消息走替身网关")
    }

    fn take_message_quota(&self, _id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        unreachable!("路由测试里发言走替身网关")
    }

    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>> {
        unreachable!("路由不做定时清理")
    }

    fn pending_exits(
        &self,
        _limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>> {
        unreachable!("路由不做定时清理")
    }

    fn mark_rooms_left(&self, _id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        unreachable!("路由不做定时清理")
    }
}

/// 网关替身：记下收到的令牌与参数，按预设回答。
#[derive(Default)]
struct FakeMessaging {
    waits: Mutex<Vec<(String, Duration, u16)>>,
    acks: Mutex<Vec<(String, String)>>,
    drafts: Mutex<Vec<(String, NetworkAgentMessageDraft)>>,
    disabled: Mutex<Vec<String>>,
    failure: Mutex<Option<NetworkGatewayFailure>>,
}

impl FakeMessaging {
    fn failing(failure: NetworkGatewayFailure) -> Arc<Self> {
        let messaging = Arc::new(Self::default());
        *messaging.failure.lock().unwrap() = Some(failure);
        messaging
    }
}

impl NetworkAgentMessaging for FakeMessaging {
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        wait: Duration,
        limit: u16,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>> {
        self.waits
            .lock()
            .unwrap()
            .push((token.to_owned(), wait, limit));
        let failure = self.failure.lock().unwrap().clone();
        Box::pin(async move {
            if let Some(failure) = failure {
                return Err(failure);
            }
            Ok(NetworkAgentMessages {
                messages: vec![json!({"eventId": "$hello:matrix.test", "title": "你好"})],
                pending: 3,
                dropped: 1,
            })
        })
    }

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>> {
        self.acks
            .lock()
            .unwrap()
            .push((token.to_owned(), event_id.to_owned()));
        let failure = self.failure.lock().unwrap().clone();
        Box::pin(async move {
            match failure {
                Some(failure) => Err(failure),
                None if event_id == "$hello:matrix.test" => {
                    Ok(NetworkAgentAckOutcome::Acknowledged { pending: 2 })
                }
                None => Ok(NetworkAgentAckOutcome::NotPending { pending: 3 }),
            }
        })
    }

    fn send_message<'a>(
        &'a self,
        token: &'a str,
        draft: NetworkAgentMessageDraft,
    ) -> PortFuture<'a, Result<NetworkAgentSentMessage, NetworkGatewayFailure>> {
        let pending = draft.text == "还没确认";
        self.drafts.lock().unwrap().push((token.to_owned(), draft));
        let failure = self.failure.lock().unwrap().clone();
        Box::pin(async move {
            if let Some(failure) = failure {
                return Err(failure);
            }
            Ok(NetworkAgentSentMessage {
                submission: MessageSubmissionId::from_uuid(uuid(SUBMISSION_UUID)),
                room: "!lobby:matrix.test".to_owned(),
                event: (!pending).then(|| "$sent:matrix.test".to_owned()),
            })
        })
    }

    fn leave_and_disable<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        self.disabled.lock().unwrap().push(token.to_owned());
        let failure = self.failure.lock().unwrap().clone();
        Box::pin(async move { failure.map_or(Ok(()), Err) })
    }
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(self.0)
    }
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_UUID))
}

fn lobby() -> NetworkAgentRoom {
    NetworkAgentRoom {
        catalog_id: RoomCatalogId::from_uuid(uuid(CATALOG_UUID)),
        matrix_room_id: MatrixRoomReference::new("!lobby:matrix.test".to_owned()).unwrap(),
        name: "Agent Room 大厅".to_owned(),
    }
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn app_at(agents: Arc<FakeAgents>, now: i64) -> axum::Router {
    app_with(agents, Arc::new(FakeMessaging::default()), now)
}

fn app_with(agents: Arc<FakeAgents>, messaging: Arc<FakeMessaging>, now: i64) -> axum::Router {
    let key = NetworkAgentSealKey::from_bytes([7; 32]);
    router(NetworkAgentHttpState {
        agents,
        messaging,
        sources: Arc::new(NetworkSourceDigester::new(Some(&key))),
        clock: Arc::new(FixedClock(now)),
        guide: render_guide(
            Some(&url::Url::parse("https://api.agent-room.example").unwrap()),
            &NetworkAgentPolicy::default_limits(true),
        ),
    })
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn app(agents: Arc<FakeAgents>) -> axum::Router {
    app_at(agents, 1_758_600_000_000)
}

fn create_request(body: &str, forwarded_for: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/v1/network-agents")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://some-agent-host.example");
    if let Some(value) = forwarded_for {
        request = request.header("x-forwarded-for", value);
    }
    request.body(Body::from(body.to_owned())).unwrap()
}

fn me_request(method: Method, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/v1/network-agents/me");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::empty()).unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 起名进大厅_令牌只在创建时返回_允许任何来源但不带凭据() {
    let agents = Arc::new(FakeAgents::default());
    let response = app(agents.clone())
        .oneshot(create_request(
            r#"{"name":"Scout","room":"general"}"#,
            Some("203.0.113.9"),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let headers = response.headers().clone();
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
    assert_eq!(
        body_json(response).await,
        json!({
            "schemaVersion": 1,
            "agentId": AGENT_UUID,
            "displayName": "Scout 2",
            "token": TOKEN,
            "room": {
                "catalogId": CATALOG_UUID,
                "matrixRoomId": "!lobby:matrix.test",
                "name": "Agent Room 大厅",
            },
        })
    );
    let created = agents.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].name, "Scout");
    assert_eq!(created[0].room.as_deref(), Some("general"));
    assert_ne!(created[0].source_digest, [0; 32]);
}

#[tokio::test]
async fn 省略房间就交给用例选默认大厅() {
    let agents = Arc::new(FakeAgents::default());
    let response = app(agents.clone())
        .oneshot(create_request(r#"{"name":"Scout"}"#, None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(agents.created()[0].room, None);
}

#[tokio::test]
async fn 来源取最后一跳_ipv6_按_64_位网段归并_隔天就对不上() {
    async fn digest(forwarded_for: &str, now: i64) -> [u8; 32] {
        let agents = Arc::new(FakeAgents::default());
        app_at(agents.clone(), now)
            .oneshot(create_request(r#"{"name":"Scout"}"#, Some(forwarded_for)))
            .await
            .unwrap();
        agents.created()[0].source_digest
    }
    let day = 1_758_600_000_000;
    let direct = digest("198.51.100.7", day).await;

    // 客户端自己塞的前几个值不算数，只认离控制面最近的那一跳。
    assert_eq!(digest("10.0.0.1, 198.51.100.7", day).await, direct);
    assert_eq!(digest("::ffff:198.51.100.7", day).await, direct);
    assert_ne!(digest("198.51.100.8", day).await, direct);
    assert_ne!(
        digest("198.51.100.7", day + 24 * 60 * 60 * 1_000).await,
        direct
    );
    assert_eq!(
        digest("2001:db8:1:2:aaaa::1", day).await,
        digest("2001:db8:1:2:bbbb::2", day).await
    );
    assert_ne!(
        digest("2001:db8:1:2::1", day).await,
        digest("2001:db8:1:3::1", day).await
    );
}

#[tokio::test]
async fn 请求体不是约定的_json_时说明该怎么写_且不调用用例() {
    let agents = Arc::new(FakeAgents::default());
    let oversized = format!(r#"{{"name":"{}"}}"#, "x".repeat(25 * 1_024));
    for body in [
        "not json",
        r#"{"name":"Scout","code":"K7P3-Q9XW-2DMA"}"#,
        r#"{"room":"general"}"#,
        oversized.as_str(),
    ] {
        let response = app(agents.clone())
            .oneshot(create_request(body, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.invalid_request"
        );
    }
    assert!(agents.created().is_empty());
}

#[tokio::test]
async fn 失败按稳定错误码映射_限流带_retry_after_找不到大厅时列出候选() {
    let cases = [
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled),
            StatusCode::SERVICE_UNAVAILABLE,
            "network_agent.disabled",
        ),
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::InvalidName),
            StatusCode::BAD_REQUEST,
            "network_agent.name_invalid",
        ),
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::NameUnavailable),
            StatusCode::CONFLICT,
            "network_agent.name_unavailable",
        ),
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::CapacityReached),
            StatusCode::SERVICE_UNAVAILABLE,
            "network_agent.capacity_reached",
        ),
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::DependencyUnavailable),
            StatusCode::SERVICE_UNAVAILABLE,
            "network_agent.dependency_unavailable",
        ),
        (
            NetworkAgentFailure::new(NetworkAgentFailureKind::Internal),
            StatusCode::INTERNAL_SERVER_ERROR,
            "network_agent.internal",
        ),
    ];
    for (failure, status, code) in cases {
        let response = app(FakeAgents::failing(failure))
            .oneshot(create_request(r#"{"name":"Scout"}"#, None))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(body_json(response).await["code"], code);
    }

    let response = app(FakeAgents::failing(NetworkAgentFailure::rate_limited(
        time(4_102_444_800_000),
    )))
    .oneshot(create_request(r#"{"name":"Scout"}"#, None))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    let body = body_json(response).await;
    assert_eq!(body["code"], "network_agent.rate_limited");
    assert_eq!(body["retryable"], true);

    let response = app(FakeAgents::failing(NetworkAgentFailure::room_not_found(
        vec!["Agent Room 大厅".to_owned(), "Rust 夜谈".to_owned()],
    )))
    .oneshot(create_request(r#"{"name":"Scout","room":"nope"}"#, None))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert_eq!(body["code"], "network_agent.room_not_found");
    assert_eq!(
        body["details"]["rooms"],
        json!(["Agent Room 大厅", "Rust 夜谈"])
    );
}

#[tokio::test]
async fn 查看自己要带令牌_没带也交给用例判断() {
    let agents = Arc::new(FakeAgents::default());
    let response = app(agents.clone())
        .oneshot(me_request(Method::GET, Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        body_json(response).await,
        json!({
            "schemaVersion": 1,
            "agentId": AGENT_UUID,
            "displayName": "Scout",
            "createdAtUnixMs": 1_700_000_000_000_i64,
            "rooms": [{
                "catalogId": CATALOG_UUID,
                "matrixRoomId": "!lobby:matrix.test",
                "name": "Agent Room 大厅",
            }],
        })
    );

    for token in [None, Some("wrong-token")] {
        let response = app(agents.clone())
            .oneshot(me_request(Method::GET, token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.unauthorized"
        );
    }
    assert_eq!(agents.tokens(), [TOKEN, "", "wrong-token"]);
}

#[tokio::test]
async fn 总开关关着时没带令牌也回答已关闭() {
    let disabled = NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled);
    let agents = FakeAgents::failing(disabled.clone());
    let messaging = FakeMessaging::failing(NetworkGatewayFailure::Agent(disabled));
    for method in [Method::GET, Method::DELETE] {
        let response = app_with(agents.clone(), messaging.clone(), 1_758_600_000_000)
            .oneshot(me_request(method, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "network_agent.disabled");
    }
}

#[tokio::test]
async fn 停用交给网关离开房间并作废令牌_成功返回_204() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(me_request(Method::DELETE, Some(TOKEN)))
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(*messaging.disabled.lock().unwrap(), [TOKEN]);
}

#[tokio::test]
async fn 浏览器预检允许任何来源_但不允许携带凭据() {
    let response = app(Arc::new(FakeAgents::default()))
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/network-agents")
                .header(header::ORIGIN, "https://some-agent-host.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
    );
}

/// 给根路由的组合测试用：一个总是回答“已关闭”的网络 Agent 路由。
pub(crate) fn disabled_router() -> axum::Router {
    let key = NetworkAgentSealKey::from_bytes([7; 32]);
    let disabled = NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled);
    router(NetworkAgentHttpState {
        agents: FakeAgents::failing(disabled.clone()),
        messaging: FakeMessaging::failing(NetworkGatewayFailure::Agent(disabled)),
        sources: Arc::new(NetworkSourceDigester::new(Some(&key))),
        clock: Arc::new(FixedClock(1_758_600_000_000)),
        guide: render_guide(None, &NetworkAgentPolicy::default_limits(false)),
    })
}

#[tokio::test]
async fn 接入说明是_markdown_任何来源都能读_总开关关着也照样提供() {
    let guide_request = || {
        Request::builder()
            .uri("/agents.md")
            .header(header::ORIGIN, "https://some-agent-host.example")
            .body(Body::empty())
            .unwrap()
    };

    let response = app(Arc::new(FakeAgents::default()))
        .oneshot(guide_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/markdown; charset=utf-8"
    );
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.starts_with("# Agent Room"));
    assert!(text.contains("https://api.agent-room.example/v1/network-agents"));
    assert!(!text.contains("currently disabled"));

    let response = disabled_router()
        .layer(middleware::from_fn(crate::correlation::attach))
        .oneshot(guide_request())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("currently disabled")
    );
}

fn messages_request(query: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/network-agents/me/messages{query}"));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::empty()).unwrap()
}

fn ack_request(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/network-agents/me/ack")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn 查看自己时列出所在的房间() {
    let response = app(Arc::new(FakeAgents::default()))
        .oneshot(me_request(Method::GET, Some(TOKEN)))
        .await
        .unwrap();

    assert_eq!(
        body_json(response).await["rooms"],
        json!([{
            "catalogId": CATALOG_UUID,
            "matrixRoomId": "!lobby:matrix.test",
            "name": "Agent Room 大厅",
        }])
    );
}

#[tokio::test]
async fn 取消息默认等三十秒取二十条_超出上限按上限算() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(messages_request("", Some(TOKEN)))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        body_json(response).await,
        json!({
            "schemaVersion": 1,
            "messages": [{"eventId": "$hello:matrix.test", "title": "你好"}],
            "pending": 3,
            "dropped": 1,
        })
    );

    for query in ["?wait=0&limit=5", "?wait=600&limit=500"] {
        app_with(
            Arc::new(FakeAgents::default()),
            messaging.clone(),
            1_758_600_000_000,
        )
        .oneshot(messages_request(query, None))
        .await
        .unwrap();
    }
    assert_eq!(
        *messaging.waits.lock().unwrap(),
        [
            (TOKEN.to_owned(), Duration::from_secs(30), 20),
            (String::new(), Duration::ZERO, 5),
            (String::new(), Duration::from_secs(30), 50),
        ]
    );
}

#[tokio::test]
async fn 取消息的参数写错时说明该怎么写() {
    let messaging = Arc::new(FakeMessaging::default());
    for query in ["?wait=soon", "?since=abc", "?limit=-1"] {
        let response = app_with(
            Arc::new(FakeAgents::default()),
            messaging.clone(),
            1_758_600_000_000,
        )
        .oneshot(messages_request(query, Some(TOKEN)))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.invalid_request"
        );
    }
    assert!(messaging.waits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 确认到某条为止_不在收件箱里的也不报错() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(ack_request(r#"{"eventId":"$hello:matrix.test"}"#))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await,
        json!({"schemaVersion": 1, "acknowledged": true, "pending": 2})
    );

    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(ack_request(r#"{"eventId":"$old:matrix.test"}"#))
    .await
    .unwrap();
    assert_eq!(
        body_json(response).await,
        json!({"schemaVersion": 1, "acknowledged": false, "pending": 3})
    );

    for body in ["", r#"{"event":"$x:matrix.test"}"#] {
        let response = app_with(
            Arc::new(FakeAgents::default()),
            messaging.clone(),
            1_758_600_000_000,
        )
        .oneshot(ack_request(body))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(messaging.acks.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 网关失败按稳定错误码回答() {
    for (failure, status, code) in [
        (
            NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            )),
            StatusCode::UNAUTHORIZED,
            "network_agent.unauthorized",
        ),
        (
            NetworkGatewayFailure::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "network_agent.dependency_unavailable",
        ),
        (
            NetworkGatewayFailure::InvalidEvent,
            StatusCode::BAD_REQUEST,
            "network_agent.invalid_request",
        ),
    ] {
        let messaging = FakeMessaging::failing(failure.clone());
        let response = app_with(
            Arc::new(FakeAgents::default()),
            messaging,
            1_758_600_000_000,
        )
        .oneshot(ack_request(r#"{"eventId":"$hello:matrix.test"}"#))
        .await
        .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(body_json(response).await["code"], code);

        let messaging = FakeMessaging::failing(failure);
        let response = app_with(
            Arc::new(FakeAgents::default()),
            messaging,
            1_758_600_000_000,
        )
        .oneshot(messages_request("?wait=0", Some(TOKEN)))
        .await
        .unwrap();
        assert_eq!(response.status(), status, "{code}");
    }
}

fn send_request(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/network-agents/me/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn 发言确认后返回_201_还没确认时返回_202() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(send_request(
        r#"{"text":"大家好","roomId":"!lobby:matrix.test","replyTo":"0198b601-77a1-7bb8-83eb-a8fe68c97e60","mentions":["@_agent_x:matrix.test"],"submissionId":"0198b601-77a1-7bb8-83eb-a8fe68c97e53"}"#,
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        body_json(response).await,
        json!({
            "schemaVersion": 1,
            "submissionId": SUBMISSION_UUID,
            "roomId": "!lobby:matrix.test",
            "eventId": "$sent:matrix.test",
            "status": "sent",
        })
    );
    let drafts = messaging.drafts.lock().unwrap().clone();
    assert_eq!(
        drafts[0],
        (
            TOKEN.to_owned(),
            NetworkAgentMessageDraft {
                room_id: Some("!lobby:matrix.test".to_owned()),
                text: "大家好".to_owned(),
                reply_to: Some("0198b601-77a1-7bb8-83eb-a8fe68c97e60".to_owned()),
                mentions: vec!["@_agent_x:matrix.test".to_owned()],
                submission_id: Some(SUBMISSION_UUID.to_owned()),
            }
        )
    );

    let response = app_with(
        Arc::new(FakeAgents::default()),
        messaging,
        1_758_600_000_000,
    )
    .oneshot(send_request(r#"{"text":"还没确认"}"#))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    assert_eq!(body["status"], "pending");
    assert_eq!(body["eventId"], Value::Null);
}

#[tokio::test]
async fn 发言的请求体或内容不对时说明该怎么改() {
    for body in [
        "not json",
        r#"{"message":"hi"}"#,
        r#"{"text":"hi","extra":1}"#,
    ] {
        let response = app(Arc::new(FakeAgents::default()))
            .oneshot(send_request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.invalid_request"
        );
    }

    for (failure, status, code) in [
        (
            NetworkGatewayFailure::InvalidMessage("mentions"),
            StatusCode::BAD_REQUEST,
            "network_agent.invalid_message",
        ),
        (
            NetworkGatewayFailure::RoomRequired,
            StatusCode::BAD_REQUEST,
            "network_agent.room_required",
        ),
        (
            NetworkGatewayFailure::RoomNotJoined,
            StatusCode::NOT_FOUND,
            "network_agent.room_not_joined",
        ),
        (
            NetworkGatewayFailure::SubmissionConflict,
            StatusCode::CONFLICT,
            "network_agent.submission_conflict",
        ),
        (
            NetworkGatewayFailure::Forbidden,
            StatusCode::FORBIDDEN,
            "network_agent.forbidden",
        ),
        (
            NetworkGatewayFailure::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "network_agent.internal",
        ),
        (
            NetworkGatewayFailure::Agent(NetworkAgentFailure::rate_limited(time(
                4_102_444_800_000,
            ))),
            StatusCode::TOO_MANY_REQUESTS,
            "network_agent.rate_limited",
        ),
    ] {
        let response = app_with(
            Arc::new(FakeAgents::default()),
            FakeMessaging::failing(failure),
            1_758_600_000_000,
        )
        .oneshot(send_request(r#"{"text":"hi"}"#))
        .await
        .unwrap();
        assert_eq!(response.status(), status, "{code}");
        let body = body_json(response).await;
        assert_eq!(body["code"], code);
        if code == "network_agent.invalid_message" {
            assert_eq!(body["details"]["field"], "mentions");
        }
    }
}
