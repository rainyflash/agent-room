mod common;

use std::{
    collections::BTreeMap,
    io::Write as _,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_agent_client::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcHostSessionState, IpcHostSessionSummary, IpcMethod, IpcResponse,
};
use agent_room_mcp::http::{HttpConfig, router};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

const TOKEN: &str = "test-only-bearer-token-abcdefghijklmnopqrstuvwxyz-0123456789";
const SESSION: &str = "01990d9e-8400-7000-8000-000000000010";

#[derive(Default)]
struct RecordingBridge {
    calls: Mutex<Vec<IpcMethod>>,
    next: Mutex<Option<IpcResponse>>,
}

impl BridgeToolClient for RecordingBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.clone());
        Box::pin(async move {
            match method {
                IpcMethod::OpenHostSession(_) => Ok(IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: SESSION.into(),
                        state: IpcHostSessionState::Ready,
                        agent_id: None,
                        error_code: None,
                    },
                }),
                IpcMethod::WithSession { session_id, method } if session_id == SESSION => {
                    match *method {
                        IpcMethod::ReadInbox(_) | IpcMethod::WaitInbox(_) => {
                            Ok(self.next.lock().unwrap().take().unwrap_or(
                                IpcResponse::MessagePreviews {
                                    previews: vec![],
                                    next_cursor: None,
                                },
                            ))
                        }
                        _ => Err(BridgeToolFailure::new(
                            "test.unexpected",
                            IpcErrorCategory::Validation,
                            false,
                            BTreeMap::new(),
                        )),
                    }
                }
                _ => Err(BridgeToolFailure::new(
                    "test.unexpected",
                    IpcErrorCategory::Validation,
                    false,
                    BTreeMap::new(),
                )),
            }
        })
    }
}

struct Server {
    url: String,
    client: Client,
    bridge: Arc<RecordingBridge>,
    task: tokio::task::JoinHandle<()>,
    _token: tempfile::NamedTempFile,
}

impl Server {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind = listener.local_addr().unwrap();
        let url = format!("http://{bind}/mcp");
        let mut token = tempfile::NamedTempFile::new().unwrap();
        token.write_all(TOKEN.as_bytes()).unwrap();
        let config = HttpConfig::load(bind, &url, token.path()).unwrap();
        let bridge = Arc::new(RecordingBridge::default());
        let app = router(
            bridge.clone(),
            config,
            tokio_util::sync::CancellationToken::new(),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            bridge,
            task,
            _token: token,
        }
    }

    async fn rpc(&self, request: Value) -> Value {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(TOKEN)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        common::rpc_result(response).await
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn in_memory_app(
    bridge: Arc<RecordingBridge>,
    shutdown: tokio_util::sync::CancellationToken,
) -> axum::Router {
    let mut token = tempfile::NamedTempFile::new().unwrap();
    token.write_all(TOKEN.as_bytes()).unwrap();
    router(
        bridge,
        HttpConfig::load(
            "127.0.0.1:8181".parse().unwrap(),
            "http://127.0.0.1:8181/mcp",
            token.path(),
        )
        .unwrap(),
        shutdown,
    )
}

fn wait_request(version: &str, progress: bool) -> axum::extract::Request {
    let mut meta = json!({});
    if version == "2026-07-28" {
        meta = json!({"io.modelcontextprotocol/protocolVersion":version,
            "io.modelcontextprotocol/clientInfo":{"name":"blocking-test","version":"1"},
            "io.modelcontextprotocol/clientCapabilities":{}});
    }
    if progress {
        meta["progressToken"] = json!("waiting");
    }
    axum::extract::Request::builder().method("POST").uri("/mcp")
        .header("host", "127.0.0.1:8181").header("Authorization", format!("Bearer {TOKEN}"))
        .header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", version).header("Mcp-Method", "tools/call").header("Mcp-Name", "agent_room_wait_for_messages")
        .body(axum::body::Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"agent_room_wait_for_messages","arguments":{"sessionId":SESSION,"afterEventId":"$last"},"_meta":meta
        }}).to_string())).unwrap()
}

#[tokio::test(start_paused = true)]
async fn http等待超过旧期限只发送传输保活且断开立即释放等待() {
    use futures_util::StreamExt as _;
    use tower::ServiceExt as _;
    for version in ["2025-06-18", "2026-07-28"] {
        let bridge = Arc::new(RecordingBridge::default());
        let app = in_memory_app(bridge.clone(), tokio_util::sync::CancellationToken::new());
        let response = app.oneshot(wait_request(version, true)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        let first = stream.next().await.unwrap().unwrap();
        let first = std::str::from_utf8(&first).unwrap();
        assert!(first.contains("notifications/progress"));
        let start = tokio::time::Instant::now();
        for _ in 0..12 {
            // The SDK SSE timer mixes std and Tokio clocks; advance explicitly in this test.
            tokio::time::advance(Duration::from_secs(15)).await;
            tokio::task::yield_now().await;
            let chunk = stream.next().await.unwrap().unwrap();
            let text = std::str::from_utf8(&chunk).unwrap();
            assert!(
                text.lines()
                    .all(|line| line.is_empty() || line.starts_with(':')),
                "transport keepalive only: {text}"
            );
        }
        assert!(start.elapsed() >= Duration::from_mins(3));
        assert!(bridge.calls.lock().unwrap().len() > 1);
        drop(stream);
        tokio::task::yield_now().await;
        let count = bridge.calls.lock().unwrap().len();
        tokio::time::sleep(Duration::from_mins(2)).await;
        assert_eq!(bridge.calls.lock().unwrap().len(), count);
    }
}

#[tokio::test(start_paused = true)]
async fn http未请求进度通知也不会在一百五十五秒截断并可正常返回消息() {
    use tower::ServiceExt as _;
    let bridge = Arc::new(RecordingBridge::default());
    let app = in_memory_app(bridge.clone(), tokio_util::sync::CancellationToken::new());
    let response = app.oneshot(wait_request("2026-07-28", false));
    tokio::pin!(response);
    assert!(
        tokio::time::timeout(Duration::from_mins(3), &mut response)
            .await
            .is_err()
    );
    *bridge.next.lock().unwrap() = Some(message_page("$next"));
    let response = response.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("$next"));
    assert!(!text.contains("notifications/progress"));
    assert!(bridge.calls.lock().unwrap().iter().all(|method| matches!(method, IpcMethod::WithSession { method, .. } if matches!(method.as_ref(), IpcMethod::WaitInbox(request) if request.after_event_id.as_deref() == Some("$last")))));
}

#[tokio::test(start_paused = true)]
async fn http并发限制覆盖整个等待流并在取消后释放额度() {
    use futures_util::StreamExt as _;
    use tower::ServiceExt as _;
    let app = in_memory_app(
        Arc::new(RecordingBridge::default()),
        tokio_util::sync::CancellationToken::new(),
    );
    let mut streams = vec![];
    for _ in 0..32 {
        let response = app
            .clone()
            .oneshot(wait_request("2026-07-28", true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        stream.next().await.unwrap().unwrap();
        streams.push(stream);
    }
    assert_eq!(
        app.clone()
            .oneshot(wait_request("2026-07-28", true))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(streams.pop());
    assert_eq!(
        app.oneshot(wait_request("2026-07-28", true))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn 真实_http_协商列出工具打开会话并保持等待身份() {
    let server = Server::start().await;
    let init = server.rpc(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"integration","version":"1"}
    }})).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    let list = server
        .rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .await;
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 16);
    let open = server.rpc(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"agent_room_open_session","arguments":{"sessionKey":SESSION,"displayName":"HTTP test"}
    }})).await;
    assert_ne!(open["result"]["isError"], true);
    let wait = server.rpc(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
        "name":"agent_room_wait_for_messages","arguments":{"sessionId":SESSION,"afterEventId":"$last","waitSeconds":0}
    }})).await;
    assert_ne!(wait["result"]["isError"], true);
    let calls = server.bridge.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(
        matches!(&calls[1], IpcMethod::WithSession { session_id, method }
        if session_id == SESSION && matches!(method.as_ref(), IpcMethod::ReadInbox(request)
            if request.after_event_id.as_deref() == Some("$last")))
    );
}

#[tokio::test(start_paused = true)]
async fn http服务器关闭也会结束阻塞流并停止读取() {
    use futures_util::StreamExt as _;
    use tower::ServiceExt as _;
    let bridge = Arc::new(RecordingBridge::default());
    let shutdown = tokio_util::sync::CancellationToken::new();
    let app = in_memory_app(bridge.clone(), shutdown.clone());
    let response = app.oneshot(wait_request("2026-07-28", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    stream.next().await.unwrap().unwrap();
    shutdown.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .is_none()
    );
    drop(stream);
    tokio::task::yield_now().await;
    let count = bridge.calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_mins(2)).await;
    assert_eq!(bridge.calls.lock().unwrap().len(), count);
}

#[tokio::test(start_paused = true)]
async fn 阻塞工具不会取消对慢请求体的期限限制() {
    use tower::ServiceExt as _;
    let app = in_memory_app(
        Arc::new(RecordingBridge::default()),
        tokio_util::sync::CancellationToken::new(),
    );
    let (parts, _) = wait_request("2026-07-28", true).into_parts();
    let body = axum::body::Body::from_stream(futures_util::stream::pending::<
        Result<axum::body::Bytes, std::convert::Infallible>,
    >());
    let started = tokio::time::Instant::now();
    let response = app
        .oneshot(axum::extract::Request::from_parts(parts, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(started.elapsed(), Duration::from_secs(15));
}

#[tokio::test]
async fn 未授权错误来源和超大请求不会访问_bridge() {
    let server = Server::start().await;
    for token in [None, Some("wrong-token")] {
        let mut request = server.client.post(&server.url);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for (header, value) in [
        ("Origin", "https://attacker.invalid"),
        ("Host", "attacker.invalid"),
    ] {
        let response = server
            .client
            .post(&server.url)
            .bearer_auth(TOKEN)
            .header(header, value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = server
        .client
        .post(&server.url)
        .bearer_auth(TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body("x".repeat(65_537))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(server.bridge.calls.lock().unwrap().is_empty());
}

#[test]
fn 配置拒绝公网明文无效地址和短令牌() {
    let mut token = tempfile::NamedTempFile::new().unwrap();
    token.write_all(TOKEN.as_bytes()).unwrap();
    let bind = "0.0.0.0:8181".parse().unwrap();
    for url in [
        "http://localhost/mcp",
        "http://example.com/mcp",
        "https://example.com/other",
        "https://user@example.com/mcp",
        "https://example.com/mcp?token=secret",
    ] {
        assert!(HttpConfig::load(bind, url, token.path()).is_err());
    }
    assert!(HttpConfig::load(bind, "https://example.com/mcp", token.path()).is_ok());
    token.as_file_mut().set_len(0).unwrap();
    assert!(HttpConfig::load(bind, "https://example.com/mcp", token.path()).is_err());
}

#[tokio::test]
async fn 新版无状态协议可直接发现工具并拒绝重复认证头() {
    let server = Server::start().await;
    let response = server
        .client
        .post(&server.url)
        .bearer_auth(TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list")
        .json(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientInfo":{"name":"integration","version":"1"},
                "io.modelcontextprotocol/clientCapabilities":{}
            }}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result = common::rpc_result(response).await;
    assert_eq!(result["result"]["tools"].as_array().unwrap().len(), 16);
    let response = server
        .client
        .post(&server.url)
        .bearer_auth(TOKEN)
        .header("Authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(server.bridge.calls.lock().unwrap().is_empty());
}

fn message_page(event: &str) -> IpcResponse {
    use agent_room_bridge_ipc::{
        IpcActorSummary, IpcContentReference, IpcConversationMessage, IpcMessagePreviewSummary,
        IpcMessageSensitivity,
    };
    let id = "01990d9e-8400-7000-8000-000000000010";
    IpcResponse::MessagePreviews {
        previews: vec![IpcMessagePreviewSummary {
            event_id: event.into(),
            message_id: id.into(),
            room_id: "!room:example.test".into(),
            actor: IpcActorSummary::Human {
                principal_id: id.into(),
                display_name: "Owner".into(),
                matrix_user_id: "@owner:example.test".into(),
                avatar_url: None,
            },
            conversation: Some(IpcConversationMessage {
                attachment_name: None,
                text: "hello".into(),
                mentions: vec![],
            }),
            reply_to_message_id: None,
            created_at_unix_ms: 1,
            title: "hello".into(),
            summary: "hello".into(),
            content: IpcContentReference {
                content_id: id.into(),
                digest_sha256: "0".repeat(64),
                media_type: "text/plain".into(),
                size_bytes: 5,
            },
            language: None,
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: vec![],
        }],
        next_cursor: None,
    }
}
