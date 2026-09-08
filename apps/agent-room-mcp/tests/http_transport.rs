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
struct RecordingBridge(Mutex<Vec<IpcMethod>>);

impl BridgeToolClient for RecordingBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.0.lock().unwrap().push(method.clone());
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
                        IpcMethod::ReadInbox(_) => Ok(IpcResponse::MessagePreviews {
                            previews: vec![],
                            next_cursor: None,
                        }),
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
        let app = router(bridge.clone(), config);
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
        response.json().await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
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
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 13);
    let open = server.rpc(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"agent_room_open_session","arguments":{"sessionKey":SESSION,"displayName":"HTTP test"}
    }})).await;
    assert_ne!(open["result"]["isError"], true);
    let wait = server.rpc(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
        "name":"agent_room_wait_for_messages","arguments":{"sessionId":SESSION,"afterEventId":"$last","waitSeconds":0}
    }})).await;
    assert_ne!(wait["result"]["isError"], true);
    let calls = server.bridge.0.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(
        matches!(&calls[1], IpcMethod::WithSession { session_id, method }
        if session_id == SESSION && matches!(method.as_ref(), IpcMethod::ReadInbox(request)
            if request.after_event_id.as_deref() == Some("$last")))
    );
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
    assert!(server.bridge.0.lock().unwrap().is_empty());
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
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["result"]["tools"].as_array().unwrap().len(), 13);
    let response = server
        .client
        .post(&server.url)
        .bearer_auth(TOKEN)
        .header("Authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(server.bridge.0.lock().unwrap().is_empty());
}
