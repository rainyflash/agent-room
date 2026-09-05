use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcBridgeState, IpcCloseHostSessionRequest, IpcErrorCategory,
    IpcHostSessionState, IpcHostSessionSummary, IpcMethod, IpcOpenHostSessionRequest, IpcResponse,
    IpcSelfSummary,
};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    sync::Barrier,
    task::JoinHandle,
    time::timeout,
};

use super::{
    super::{BridgeToolClient, BridgeToolFailure, bridge::BridgeToolFuture},
    AgentRoomMcpServer,
};

const SESSION_A: &str = "01990d9e-8400-7000-8000-000000000010";
const SESSION_B: &str = "01990d9e-8400-7000-8000-000000000011";
const SESSION_C: &str = "01990d9e-8400-7000-8000-000000000012";
const SESSION_KEY: &str = "01990d9e-8400-7000-8000-000000000020";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

// Exercise the real rmcp transport and parameter extraction without a live Bridge or network.
struct McpHarness {
    transport: BufReader<DuplexStream>,
    server_task: JoinHandle<()>,
    next_id: u64,
}

impl McpHarness {
    async fn start(backend: Arc<dyn BridgeToolClient>) -> Self {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            AgentRoomMcpServer::new(backend)
                .serve(server_io)
                .await
                .expect("MCP 服务应完成握手")
                .waiting()
                .await
                .expect("MCP 服务应正常关闭");
        });
        let mut harness = Self {
            transport: BufReader::new(client_io),
            server_task,
            next_id: 1,
        };
        harness
            .send(json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "session-routing-test", "version": "1" }
                }
            }))
            .await;
        assert!(harness.receive().await.get("result").is_some());
        harness
            .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        harness
    }

    async fn send(&mut self, frame: Value) {
        let mut bytes = serde_json::to_vec(&frame).expect("测试帧可以编码");
        bytes.push(b'\n');
        timeout(IO_TIMEOUT, self.transport.get_mut().write_all(&bytes))
            .await
            .expect("写入 MCP 帧不能挂起")
            .expect("MCP 传输可以写入");
    }

    async fn send_tool(&mut self, tool: &str, arguments: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }))
        .await;
        id
    }

    async fn receive(&mut self) -> Value {
        loop {
            let mut line = String::new();
            let count = timeout(IO_TIMEOUT, self.transport.read_line(&mut line))
                .await
                .expect("MCP 响应不能挂起")
                .expect("MCP 响应可以读取");
            assert_ne!(count, 0, "MCP 不能在回复前关闭");
            let frame: Value = serde_json::from_str(&line).expect("MCP 回复必须是 JSON");
            if frame.get("id").is_some() {
                return frame;
            }
        }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let id = self.send_tool(tool, arguments).await;
        let frame = self.receive().await;
        assert_eq!(frame["id"], id);
        frame.get("result").expect("MCP 工具须返回结果").clone()
    }

    async fn stop(self) {
        drop(self.transport);
        timeout(IO_TIMEOUT, self.server_task)
            .await
            .expect("MCP 服务应在传输结束后停止")
            .expect("MCP 服务任务不能失败");
    }
}

struct ExpectedCall {
    method: IpcMethod,
    response: Result<IpcResponse, BridgeToolFailure>,
}

#[derive(Default)]
struct ScriptedBridge {
    expected: Mutex<VecDeque<ExpectedCall>>,
}

impl ScriptedBridge {
    fn new(expected: Vec<ExpectedCall>) -> Self {
        Self {
            expected: Mutex::new(expected.into()),
        }
    }

    fn assert_finished(&self) {
        assert!(self.expected.lock().expect("响应队列锁有效").is_empty());
    }
}

impl BridgeToolClient for ScriptedBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let expected = self
            .expected
            .lock()
            .expect("响应队列锁有效")
            .pop_front()
            .expect("未经预期的调用不得进入 Bridge");
        assert_eq!(method, expected.method);
        Box::pin(async move { expected.response })
    }
}

struct ConcurrentBridge {
    barrier: Barrier,
}

impl BridgeToolClient for ConcurrentBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let IpcMethod::WithSession { session_id, method } = method else {
            panic!("每次调用都必须携带显式会话");
        };
        assert_eq!(*method, IpcMethod::GetSelf);
        Box::pin(async move {
            self.barrier.wait().await;
            Ok(self_summary(&session_id))
        })
    }
}

fn self_summary(session_id: &str) -> IpcResponse {
    let (agent_id, instance_id, matrix_user_id) = match session_id {
        SESSION_A => (
            "01990d9e-8400-7000-8000-000000000110",
            "01990d9e-8400-7000-8000-000000000210",
            "@a:example.test",
        ),
        SESSION_B => (
            "01990d9e-8400-7000-8000-000000000111",
            "01990d9e-8400-7000-8000-000000000211",
            "@b:example.test",
        ),
        SESSION_C => (
            "01990d9e-8400-7000-8000-000000000112",
            "01990d9e-8400-7000-8000-000000000212",
            "@c:example.test",
        ),
        _ => panic!("测试只登记了三个独立会话"),
    };
    IpcResponse::SelfSummary {
        summary: IpcSelfSummary {
            agent: IpcAgentSummary {
                agent_id: agent_id.to_owned(),
                display_name: "测试 Agent".to_owned(),
                matrix_user_id: matrix_user_id.to_owned(),
                avatar_url: None,
            },
            instance_id: instance_id.to_owned(),
            matrix_device_id: instance_id.to_owned(),
            room_id: "!room:example.test".to_owned(),
            connection_state: IpcBridgeState::Ready,
            granted_capabilities: vec!["self.read".to_owned()],
        },
    }
}

fn open_method(display_name: &str) -> IpcMethod {
    IpcMethod::OpenHostSession(IpcOpenHostSessionRequest {
        session_key: SESSION_KEY.to_owned(),
        display_name: display_name.to_owned(),
    })
}

fn session_response(state: IpcHostSessionState, error_code: Option<&str>) -> IpcResponse {
    IpcResponse::HostSession {
        session: IpcHostSessionSummary {
            session_id: SESSION_A.to_owned(),
            state,
            agent_id: None,
            error_code: error_code.map(str::to_owned),
        },
    }
}

fn scoped_get_self(session_id: &str) -> IpcMethod {
    IpcMethod::WithSession {
        session_id: session_id.to_owned(),
        method: Box::new(IpcMethod::GetSelf),
    }
}

fn failure(code: &str, retryable: bool) -> BridgeToolFailure {
    BridgeToolFailure::new(
        code,
        IpcErrorCategory::Conflict,
        retryable,
        BTreeMap::from([("context".to_owned(), "session-test".to_owned())]),
    )
}

#[tokio::test]
async fn 同一_mcp_连接的三个并发会话分别返回各自_bridge_响应() {
    let mut harness = McpHarness::start(Arc::new(ConcurrentBridge {
        barrier: Barrier::new(3),
    }))
    .await;
    let mut expected = BTreeMap::new();
    for session_id in [SESSION_A, SESSION_B, SESSION_C] {
        let id = harness
            .send_tool("agent_room_get_self", json!({ "sessionId": session_id }))
            .await;
        expected.insert(
            id,
            serde_json::to_value(self_summary(session_id)).expect("身份可编码"),
        );
    }
    for _ in 0..3 {
        let frame = harness.receive().await;
        let id = frame["id"].as_u64().expect("请求 ID 是整数");
        assert_ne!(frame["result"]["isError"], true);
        assert_eq!(
            frame["result"]["structuredContent"],
            expected.remove(&id).expect("只回复已发出的请求")
        );
    }
    assert!(expected.is_empty());
    harness.stop().await;
}

#[tokio::test]
async fn 未绑定参数在真实工具边界被拒绝且没有默认身份回退() {
    let mut harness = McpHarness::start(Arc::new(ScriptedBridge::default())).await;
    for (tool, arguments) in [
        ("agent_room_get_self", json!({})),
        ("agent_room_close_session", json!({})),
        ("agent_room_list_previews", json!({})),
        (
            "agent_room_get_presence",
            json!({"roomId": "!room:example.test"}),
        ),
        ("agent_room_open_content", json!({"contentId": SESSION_KEY})),
        (
            "agent_room_publish_status",
            json!({"roomId": "!room:example.test", "status": "idle"}),
        ),
        (
            "agent_room_send_message",
            json!({"chat": true, "roomId": "!room:example.test", "body": "测试", "provenance": "human_confirmed_agent"}),
        ),
        ("agent_room_list_handoffs", json!({})),
        (
            "agent_room_consume_handoff",
            json!({"handoffId": SESSION_KEY}),
        ),
        (
            "agent_room_decline_handoff",
            json!({"handoffId": SESSION_KEY}),
        ),
    ] {
        let result = harness.call(tool, arguments).await;
        assert_eq!(result["isError"], true, "{tool}");
        assert!(
            result["content"].to_string().contains("sessionId"),
            "{tool}: {result}"
        );
    }
    for arguments in [
        json!({"sessionId": null}),
        json!({"sessionId": 7}),
        json!({"sessionId": SESSION_A, "currentSession": SESSION_B}),
    ] {
        assert_eq!(
            harness.call("agent_room_get_self", arguments).await["isError"],
            true
        );
    }
    harness.stop().await;
}

#[tokio::test]
async fn 建立会话重试保留同一_key_与名称并返回_starting() {
    let bridge = Arc::new(ScriptedBridge::new(
        (0..2)
            .map(|_| ExpectedCall {
                method: open_method("任务 A"),
                response: Ok(session_response(IpcHostSessionState::Starting, None)),
            })
            .collect(),
    ));
    let mut harness = McpHarness::start(bridge.clone()).await;
    for _ in 0..2 {
        let result = harness
            .call(
                "agent_room_open_session",
                json!({"sessionKey": SESSION_KEY, "displayName": "任务 A"}),
            )
            .await;
        assert_ne!(result["isError"], true);
        assert_eq!(
            result["structuredContent"]["session"]["sessionId"],
            SESSION_A
        );
        assert_eq!(result["structuredContent"]["session"]["state"], "starting");
    }
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 会话未就绪失败未知或关闭均保留_bridge_错误而不尝试其他身份() {
    let failures = [
        ("bridge.host_session_starting", true),
        ("bridge.host_session_failed", false),
        ("bridge.host_session_not_found", false),
        ("bridge.host_session_closed", false),
        ("bridge.ipc.session_id_invalid", false),
    ];
    let bridge = Arc::new(ScriptedBridge::new(
        failures
            .iter()
            .map(|(code, retryable)| ExpectedCall {
                method: scoped_get_self(SESSION_A),
                response: Err(failure(code, *retryable)),
            })
            .collect(),
    ));
    let mut harness = McpHarness::start(bridge.clone()).await;
    for (code, retryable) in failures {
        let result = harness
            .call("agent_room_get_self", json!({"sessionId": SESSION_A}))
            .await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], code);
        assert_eq!(result["structuredContent"]["retryable"], retryable);
        assert_eq!(result["structuredContent"]["category"], "conflict");
        assert_eq!(
            result["structuredContent"]["details"]["context"],
            "session-test"
        );
    }
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 关闭指定会话可重复且不影响其他会话路由() {
    let mut expected = (0..2)
        .map(|_| ExpectedCall {
            method: IpcMethod::CloseHostSession(IpcCloseHostSessionRequest {
                session_id: SESSION_A.to_owned(),
            }),
            response: Ok(session_response(IpcHostSessionState::Closed, None)),
        })
        .collect::<Vec<_>>();
    expected.extend([
        ExpectedCall {
            method: scoped_get_self(SESSION_A),
            response: Err(failure("bridge.host_session_closed", false)),
        },
        ExpectedCall {
            method: scoped_get_self(SESSION_B),
            response: Ok(self_summary(SESSION_B)),
        },
    ]);
    let bridge = Arc::new(ScriptedBridge::new(expected));
    let mut harness = McpHarness::start(bridge.clone()).await;
    for _ in 0..2 {
        let result = harness
            .call("agent_room_close_session", json!({"sessionId": SESSION_A}))
            .await;
        assert_ne!(result["isError"], true);
        assert_eq!(result["structuredContent"]["session"]["state"], "closed");
    }
    assert_eq!(
        harness
            .call("agent_room_get_self", json!({"sessionId": SESSION_A}))
            .await["isError"],
        true
    );
    let result = harness
        .call("agent_room_get_self", json!({"sessionId": SESSION_B}))
        .await;
    assert_eq!(
        result["structuredContent"],
        serde_json::to_value(self_summary(SESSION_B)).expect("身份可编码")
    );
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 建立会话失败摘要与响应类型错误不能伪装为成功() {
    let bridge = Arc::new(ScriptedBridge::new(vec![
        ExpectedCall {
            method: open_method("任务 A"),
            response: Ok(session_response(
                IpcHostSessionState::Failed,
                Some("bridge.registration_denied"),
            )),
        },
        ExpectedCall {
            method: open_method("任务 A"),
            response: Ok(self_summary(SESSION_A)),
        },
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    let arguments = json!({"sessionKey": SESSION_KEY, "displayName": "任务 A"});
    let failed = harness
        .call("agent_room_open_session", arguments.clone())
        .await;
    assert_eq!(failed["isError"], true);
    assert_eq!(
        failed["structuredContent"]["session"]["errorCode"],
        "bridge.registration_denied"
    );
    assert_eq!(failed["structuredContent"]["session"]["state"], "failed");
    let mismatch = harness.call("agent_room_open_session", arguments).await;
    assert_eq!(mismatch["isError"], true);
    assert_eq!(
        mismatch["structuredContent"]["code"],
        "bridge.ipc.response_mismatch"
    );
    assert!(mismatch["structuredContent"].get("summary").is_none());
    bridge.assert_finished();
    harness.stop().await;
}

#[test]
fn 所有工具的_schema_都公开强制的会话边界() {
    let server = AgentRoomMcpServer::new(Arc::new(ScriptedBridge::default()));
    for tool in server.tool_router.list_all() {
        let schema = &tool.input_schema;
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "{}",
            tool.name
        );
        let required = schema["required"]
            .as_array()
            .expect("每个工具都有必填会话参数");
        let id_field = if tool.name == "agent_room_open_session" {
            assert!(required.contains(&json!("displayName")));
            assert!(schema["properties"].get("sessionId").is_none());
            "sessionKey"
        } else {
            "sessionId"
        };
        assert!(required.contains(&json!(id_field)), "{}", tool.name);
        assert_eq!(schema["properties"][id_field]["type"], "string");
        assert_eq!(schema["properties"][id_field]["minLength"], 36);
        assert_eq!(schema["properties"][id_field]["maxLength"], 36);
    }
}
