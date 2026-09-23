use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_agent_client::BridgeToolFuture;
use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcBridgeState, IpcCloseHostSessionRequest, IpcErrorCategory,
    IpcHostSessionState, IpcHostSessionSummary, IpcMethod, IpcOpenHostSessionRequest, IpcResponse,
    IpcRoomKind, IpcRoomMembership, IpcRoomSummary, IpcSelfSummary,
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
    super::{BridgeToolClient, BridgeToolFailure},
    AgentRoomMcpServer,
};

const SESSION_A: &str = "01990d9e-8400-7000-8000-000000000010";
const SESSION_B: &str = "01990d9e-8400-7000-8000-000000000011";
const SESSION_C: &str = "01990d9e-8400-7000-8000-000000000012";

#[tokio::test]
async fn 接待登记使用宿主任务元数据并拒绝错绑或猜测() {
    let bridge = Arc::new(ScriptedBridge::new(vec![ExpectedCall {
        method: IpcMethod::WithSession {
            session_id: SESSION_A.into(),
            method: Box::new(IpcMethod::RegisterReception(
                agent_room_bridge_ipc::IpcRegisterReceptionRequest {
                    host_type: agent_room_bridge_ipc::IpcReceptionHost::Codex,
                    task_id: SESSION_B.into(),
                    workspace: "/project".into(),
                },
            )),
        },
        response: Ok(IpcResponse::HostSession {
            session: IpcHostSessionSummary {
                session_id: SESSION_A.into(),
                state: IpcHostSessionState::Ready,
                agent_id: None,
                error_code: None,
            },
        }),
    }]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    harness.send(json!({"jsonrpc":"2.0","id":100,"method":"tools/call","params":{
        "name":"agent_room_register_reception","arguments":{"sessionId":SESSION_A,"workspace":"/project"},"_meta":{"threadId":SESSION_B}
    }})).await;
    assert_ne!(harness.receive().await["result"]["isError"], true);
    harness.send(json!({"jsonrpc":"2.0","id":101,"method":"tools/call","params":{
        "name":"agent_room_register_reception","arguments":{"sessionId":SESSION_A,"taskId":SESSION_C,"workspace":"/project"},"_meta":{"threadId":SESSION_B}
    }})).await;
    assert_eq!(
        harness.receive().await["result"]["structuredContent"]["code"],
        "receiver.host_task_mismatch"
    );
    let missing = harness
        .call(
            "agent_room_register_reception",
            json!({"sessionId":SESSION_A,"workspace":"/project"}),
        )
        .await;
    assert_eq!(
        missing["structuredContent"]["code"],
        "receiver.host_task_required"
    );
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 等待工具通过真实_mcp_协议保持身份和正向游标且拒绝历史参数() {
    let bridge = Arc::new(ScriptedBridge::new(vec![ExpectedCall {
        method: IpcMethod::WithSession {
            session_id: SESSION_A.into(),
            method: Box::new(IpcMethod::ReadInbox(
                agent_room_bridge_ipc::IpcListPreviewsRequest {
                    room_id: None,
                    after_event_id: Some("$last".into()),
                    before_event_id: None,
                    limit: 20,
                },
            )),
        },
        response: Ok(IpcResponse::MessagePreviews {
            previews: vec![],
            next_cursor: None,
        }),
    }]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    let result = harness
        .call(
            "agent_room_wait_for_messages",
            json!({"sessionId":SESSION_A,"afterEventId":"$last","waitSeconds":0}),
        )
        .await;
    assert_ne!(result["isError"], true);
    let invalid = harness
        .call(
            "agent_room_wait_for_messages",
            json!({"sessionId":SESSION_A,"beforeEventId":"$past"}),
        )
        .await;
    assert_eq!(invalid["isError"], true);
    for seconds in [json!(-1), json!(86401), json!(1.5)] {
        let invalid = harness
            .call(
                "agent_room_wait_for_messages",
                json!({"sessionId":SESSION_A,"waitSeconds":seconds}),
            )
            .await;
        assert_eq!(invalid["isError"], true);
    }
    bridge.assert_finished();
    harness.stop().await;
}
const SESSION_KEY: &str = "01990d9e-8400-7000-8000-000000000020";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct WaitingBridge {
    reads: std::sync::atomic::AtomicUsize,
    fail: std::sync::atomic::AtomicBool,
}

impl BridgeToolClient for WaitingBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        use std::sync::atomic::Ordering;
        let IpcMethod::WithSession { session_id, method } = method else {
            panic!("scoped call")
        };
        assert_eq!(session_id, SESSION_A);
        let response = match *method {
            IpcMethod::GetSelf => Ok(self_summary(&session_id)),
            IpcMethod::ReadInbox(request) | IpcMethod::WaitInbox(request) => {
                assert_eq!(request.after_event_id.as_deref(), Some("$last"));
                self.reads.fetch_add(1, Ordering::SeqCst);
                if self.fail.load(Ordering::SeqCst) {
                    Err(BridgeToolFailure::new(
                        "test.connection_lost",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                        BTreeMap::new(),
                    ))
                } else {
                    Ok(IpcResponse::MessagePreviews {
                        previews: vec![],
                        next_cursor: None,
                    })
                }
            }
            _ => panic!("unexpected method"),
        };
        Box::pin(async move { response })
    }
}

#[tokio::test(start_paused = true)]
async fn mcp空闲五分钟不返回空批次且真实故障立即结束等待() {
    use std::sync::atomic::Ordering;
    let bridge = Arc::new(WaitingBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;
    let id = harness
        .send_tool(
            "agent_room_wait_for_messages",
            json!({"sessionId":SESSION_A,"afterEventId":"$last"}),
        )
        .await;
    let mut line = String::new();
    assert!(
        timeout(
            Duration::from_mins(5),
            harness.transport.read_line(&mut line)
        )
        .await
        .is_err()
    );
    assert!(line.is_empty());
    assert!(bridge.reads.load(Ordering::SeqCst) > 25);
    // Another tool on this same stdio connection remains available while waiting.
    assert_ne!(
        harness
            .call("agent_room_get_self", json!({"sessionId":SESSION_A}))
            .await["isError"],
        true
    );
    bridge.fail.store(true, Ordering::SeqCst);
    let response = harness.receive().await;
    assert_eq!(response["id"], id);
    assert_eq!(
        response["result"]["structuredContent"]["code"],
        "test.connection_lost"
    );
    harness.stop().await;
}

#[tokio::test(start_paused = true)]
async fn mcp取消通知与传输关闭都会停止内部等待且不确认消息() {
    use std::sync::atomic::Ordering;
    let bridge = Arc::new(WaitingBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;
    let id = harness
        .send_tool(
            "agent_room_wait_for_messages",
            json!({"sessionId":SESSION_A,"afterEventId":"$last"}),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    harness.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"user stopped"}})).await;
    harness
        .call("agent_room_get_self", json!({"sessionId":SESSION_A}))
        .await;
    let count = bridge.reads.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_mins(2)).await;
    assert_eq!(bridge.reads.load(Ordering::SeqCst), count);
    harness
        .send_tool(
            "agent_room_wait_for_messages",
            json!({"sessionId":SESSION_A,"afterEventId":"$last"}),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(bridge.reads.load(Ordering::SeqCst) > count);
    harness.stop().await;
    let count = bridge.reads.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_mins(2)).await;
    assert_eq!(bridge.reads.load(Ordering::SeqCst), count);
}

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
        // rmcp drains in-flight responses for up to five seconds after stdin closes.
        timeout(IO_TIMEOUT + Duration::from_secs(1), self.server_task)
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
            room_catalog_id: None,
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
        room: None,
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
            "agent_room_matrix_security",
            json!({"request":{"action":"inspect"}}),
        ),
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

/// 按名字接入时会话键由服务生成，不能用逐次比对的脚本桥；这个假 Bridge 只记录打开请求。
struct JoinBridge {
    rooms: Vec<IpcRoomSummary>,
    opened: Mutex<Vec<IpcOpenHostSessionRequest>>,
    connected_room: Mutex<String>,
    pending: Mutex<Option<agent_room_bridge_ipc::IpcInvitationOffer>>,
    /// 口令 `K7P3-Q9XW-2DMA` 对应的私人房间，以及兑换过的会话键与名字。
    code_room: Mutex<Option<IpcRoomSummary>>,
    redeemed: Mutex<Vec<agent_room_bridge_ipc::IpcRedeemJoinCodeRequest>>,
    /// 会话之外的调用按顺序记下方法名。
    calls: Mutex<Vec<&'static str>>,
}

impl JoinBridge {
    fn new(rooms: Vec<IpcRoomSummary>) -> Self {
        Self {
            rooms,
            opened: Mutex::new(Vec::new()),
            connected_room: Mutex::new("!game:test.invalid".into()),
            pending: Mutex::new(None),
            code_room: Mutex::new(None),
            redeemed: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn code_room(&self, code: &str) -> Result<IpcResponse, BridgeToolFailure> {
        let normalized = code
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_uppercase();
        match self.code_room.lock().unwrap().clone() {
            Some(room) if normalized == "K7P3Q9XW2DMA" => Ok(IpcResponse::JoinCodeRoom { room }),
            _ => Err(failure("bridge.join_code.not_found", false)),
        }
    }
}

impl BridgeToolClient for JoinBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        if !matches!(method, IpcMethod::WithSession { .. }) {
            self.calls.lock().unwrap().push(method.name());
        }
        let response = match method {
            IpcMethod::ResolveJoinCode(request) => self.code_room(&request.code),
            IpcMethod::RedeemJoinCode(request) => {
                let room = self.code_room(&request.code);
                if room.is_ok() {
                    self.redeemed.lock().unwrap().push(request);
                }
                room
            }
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: self.rooms.clone(),
            }),
            IpcMethod::ReadInvitation => Ok(IpcResponse::Invitation {
                invitation: self.pending.lock().unwrap().clone().map(|invitation| {
                    agent_room_bridge_ipc::IpcPendingInvitation {
                        invitation,
                        expires_in_ms: 60_000,
                    }
                }),
            }),
            IpcMethod::OpenHostSession(request) => {
                // 和 Bridge 一样：用等待中的人物开出会话，这份邀请就用掉了。
                let mut pending = self.pending.lock().unwrap();
                if pending
                    .as_ref()
                    .is_some_and(|invitation| invitation.session_key == request.session_key)
                {
                    *pending = None;
                }
                drop(pending);
                self.opened.lock().unwrap().push(request);
                Ok(IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: uuid::Uuid::now_v7().to_string(),
                        state: IpcHostSessionState::Ready,
                        agent_id: None,
                        error_code: None,
                    },
                })
            }
            IpcMethod::WithSession { method, .. } if matches!(*method, IpcMethod::GetSelf) => {
                Ok(IpcResponse::SelfSummary {
                    summary: IpcSelfSummary {
                        room_catalog_id: None,
                        agent: IpcAgentSummary {
                            agent_id: uuid::Uuid::now_v7().to_string(),
                            display_name: "Scout".into(),
                            matrix_user_id: "@scout:test.invalid".into(),
                            avatar_url: None,
                        },
                        instance_id: uuid::Uuid::now_v7().to_string(),
                        matrix_device_id: "TEST".into(),
                        room_id: self.connected_room.lock().unwrap().clone(),
                        connection_state: IpcBridgeState::Ready,
                        granted_capabilities: vec![],
                    },
                })
            }
            other => Err(BridgeToolFailure::new(
                "test.unexpected_call",
                IpcErrorCategory::Internal,
                false,
                BTreeMap::from([("method".to_owned(), format!("{other:?}"))]),
            )),
        };
        Box::pin(async move { response })
    }
}

fn room(kind: IpcRoomKind, name: &str, slug: Option<&str>) -> IpcRoomSummary {
    IpcRoomSummary {
        kind,
        catalog_id: uuid::Uuid::now_v7().to_string(),
        matrix_room_id: matches!(kind, IpcRoomKind::PrivateRoom)
            .then(|| "!game:test.invalid".to_owned()),
        name: name.to_owned(),
        slug: slug.map(str::to_owned),
        membership: matches!(kind, IpcRoomKind::PrivateRoom).then_some(IpcRoomMembership::Joined),
    }
}

#[tokio::test]
async fn 按房间名接入_同任务复用人物_没有任务时按连接复用() {
    let bridge = Arc::new(JoinBridge::new(vec![
        room(IpcRoomKind::PublicLobby, "Lobby", Some("lobby")),
        room(IpcRoomKind::PrivateRoom, "game dev", Some("game-dev")),
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    let listed = harness.call("agent_room_list_rooms", json!({})).await;
    assert_ne!(listed["isError"], true);
    assert_eq!(
        listed["structuredContent"]["rooms"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        listed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("不可信")
    );

    harness.send(json!({"jsonrpc":"2.0","id":200,"method":"tools/call","params":{
        "name":"agent_room_join","arguments":{"room":"Game-Dev","displayName":"Scout"},"_meta":{"threadId":SESSION_B}
    }})).await;
    let joined = harness.receive().await["result"].clone();
    assert_ne!(joined["isError"], true, "{joined}");
    let first = &joined["structuredContent"];
    assert_eq!(first["state"], "ready");
    assert_eq!(first["identity"], "created");
    assert_eq!(first["displayName"], "Scout");
    assert_eq!(first["room"]["name"], "game dev");
    assert_eq!(first["self"]["roomId"], "!game:test.invalid");
    assert_eq!(first["sessionId"].as_str().unwrap().len(), 36);
    let key = first["sessionKey"].as_str().unwrap().to_owned();

    // 同一任务再次接入：同一会话键与名字，且不必再给名字。
    harness.send(json!({"jsonrpc":"2.0","id":201,"method":"tools/call","params":{
        "name":"agent_room_join","arguments":{"room":"game dev"},"_meta":{"threadId":SESSION_B}
    }})).await;
    let again = harness.receive().await["result"].clone();
    assert_eq!(again["structuredContent"]["identity"], "reused");
    assert_eq!(again["structuredContent"]["hostTask"], SESSION_B);
    assert_eq!(again["structuredContent"]["sessionKey"], key);
    assert_eq!(again["structuredContent"]["displayName"], "Scout");
    {
        let opened = bridge.opened.lock().unwrap();
        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0], opened[1]);
        assert_eq!(opened[0].session_key, key);
        assert_eq!(
            opened[0].room.as_ref().unwrap().room_id.as_deref(),
            Some("!game:test.invalid")
        );
    }

    // 别的任务不共享人物；没有任务标识时每次都是新人物。
    harness.send(json!({"jsonrpc":"2.0","id":202,"method":"tools/call","params":{
        "name":"agent_room_join","arguments":{"room":"game dev"},"_meta":{"threadId":SESSION_C}
    }})).await;
    let other = harness.receive().await["result"].clone();
    assert_eq!(other["structuredContent"]["identity"], "created");
    assert_ne!(other["structuredContent"]["sessionKey"], key);
    // 没有 threadId 元数据时按宿主环境或这条连接复用：重试同一房间仍是同一人物。
    let lobby = harness
        .call("agent_room_join", json!({"room":"lobby"}))
        .await;
    assert_eq!(lobby["structuredContent"]["identity"], "created", "{lobby}");
    assert_eq!(lobby["structuredContent"]["room"]["kind"], "public_lobby");
    assert!(lobby["structuredContent"]["room"]["matrixRoomId"].is_null());
    let lobby_retry = harness
        .call("agent_room_join", json!({"room":"Lobby"}))
        .await;
    assert_eq!(lobby_retry["structuredContent"]["identity"], "reused");
    assert_eq!(
        lobby_retry["structuredContent"]["sessionKey"],
        lobby["structuredContent"]["sessionKey"]
    );

    harness.stop().await;
}

#[tokio::test]
async fn 按房间名接入_默认进大厅_找不到或连错房间都不报成功() {
    let bridge = Arc::new(JoinBridge::new(vec![
        room(IpcRoomKind::PublicLobby, "Lobby", Some("lobby")),
        room(IpcRoomKind::PrivateRoom, "game dev", Some("game-dev")),
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    // 不给房间名就进默认大厅：打开请求不带房间。
    let default = harness.call("agent_room_join", json!({})).await;
    assert_ne!(default["isError"], true);
    assert!(default["structuredContent"]["room"].is_null());
    assert!(bridge.opened.lock().unwrap().last().unwrap().room.is_none());

    // 找不到房间：失败并列出能进的房间，不打开会话。
    let opened_before = bridge.opened.lock().unwrap().len();
    let missing = harness
        .call("agent_room_join", json!({"room":"missing"}))
        .await;
    assert_eq!(missing["isError"], true);
    assert_eq!(
        missing["structuredContent"]["code"],
        "agent.join.room_not_found"
    );
    assert_eq!(
        missing["structuredContent"]["details"]["rooms"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(bridge.opened.lock().unwrap().len(), opened_before);

    // 连上的房间与目录不符不能报成功。
    *bridge.connected_room.lock().unwrap() = "!other:test.invalid".into();
    let mismatch = harness
        .call("agent_room_join", json!({"room":"game dev"}))
        .await;
    assert_eq!(mismatch["isError"], true);
    assert_eq!(
        mismatch["structuredContent"]["code"],
        "agent.join.room_mismatch"
    );
    harness.stop().await;
}

#[tokio::test]
async fn 只说接入时接上面板正在等的人物_之后回到这个任务上次的房间() {
    let bridge = Arc::new(JoinBridge::new(vec![
        room(IpcRoomKind::PublicLobby, "Lobby", Some("lobby")),
        room(IpcRoomKind::PrivateRoom, "game dev", Some("game-dev")),
    ]));
    let waiting = agent_room_bridge_ipc::IpcInvitationOffer {
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: Some("面板里起的名字".into()),
        room: Some(agent_room_bridge_ipc::IpcHostRoomTarget {
            catalog_id: uuid::Uuid::now_v7().to_string(),
            room_id: Some("!game:test.invalid".into()),
        }),
    };
    *bridge.pending.lock().unwrap() = Some(waiting.clone());
    let mut harness = McpHarness::start(bridge.clone()).await;
    let join = |id: u64, arguments: Value| {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
            "name":"agent_room_join","arguments":arguments,"_meta":{"threadId":SESSION_B}
        }})
    };
    harness.send(join(300, json!({}))).await;
    let invited = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(invited["identity"], "invited", "{invited}");
    assert_eq!(invited["sessionKey"], waiting.session_key.as_str());
    assert_eq!(invited["displayName"], "面板里起的名字");
    assert_eq!(invited["target"]["roomId"], "!game:test.invalid");
    assert_eq!(
        bridge.opened.lock().unwrap().as_slice(),
        &[waiting.open_request(|| unreachable!("面板定了名字"))]
    );
    assert!(bridge.pending.lock().unwrap().is_none());

    // 邀请用掉以后再说“接入”，回到同一个人物和房间。
    harness.send(join(301, json!({}))).await;
    let again = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(again["identity"], "reused");
    assert_eq!(again["sessionKey"], waiting.session_key.as_str());

    // 按名字去了别的房间，下次只说“接入”就回到那里。
    harness
        .send(join(302, json!({"room": "Lobby", "displayName": "Scout"})))
        .await;
    let lobby = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(lobby["identity"], "created");
    harness.send(join(303, json!({}))).await;
    let back = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(back["identity"], "reused");
    assert_eq!(back["sessionKey"], lobby["sessionKey"]);

    // 面板又挂了一份没定名字的邀请：用已有的名字说“接入”回到自己的人物，邀请留给别的 Agent。
    let other = agent_room_bridge_ipc::IpcInvitationOffer {
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: None,
        room: None,
    };
    *bridge.pending.lock().unwrap() = Some(other.clone());
    harness
        .send(join(304, json!({"displayName": "Scout"})))
        .await;
    let returning = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(returning["identity"], "reused", "{returning}");
    assert_eq!(returning["sessionKey"], lobby["sessionKey"]);
    assert_eq!(bridge.pending.lock().unwrap().as_ref(), Some(&other));
    // 指了房间就按名字进，也不接面板的邀请。
    harness
        .send(join(305, json!({"room": "Lobby", "displayName": "Pilot"})))
        .await;
    let elsewhere = harness.receive().await["result"]["structuredContent"].clone();
    assert_ne!(elsewhere["sessionKey"], other.session_key.as_str());
    assert_eq!(bridge.pending.lock().unwrap().as_ref(), Some(&other));
    // 起了个新名字、没指房间：接上面板的邀请，名字用自己起的。
    harness
        .send(join(306, json!({"displayName": "另起的名字"})))
        .await;
    let named = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(named["identity"], "invited", "{named}");
    assert_eq!(named["sessionKey"], other.session_key.as_str());
    assert_eq!(named["displayName"], "另起的名字");
    assert!(bridge.pending.lock().unwrap().is_none());
    harness.stop().await;
}

#[tokio::test]
async fn 凭口令接入_先查看房间再按任务选人物_开会话之前兑换() {
    let bridge = Arc::new(JoinBridge::new(vec![room(
        IpcRoomKind::PublicLobby,
        "Lobby",
        Some("lobby"),
    )]));
    let project = IpcRoomSummary {
        kind: IpcRoomKind::PrivateRoom,
        catalog_id: uuid::Uuid::now_v7().to_string(),
        matrix_room_id: Some("!game:test.invalid".to_owned()),
        name: "项目室".to_owned(),
        slug: None,
        membership: None,
    };
    *bridge.code_room.lock().unwrap() = Some(project.clone());
    let mut harness = McpHarness::start(bridge.clone()).await;
    let join = |id: u64, arguments: Value| {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
            "name":"agent_room_join","arguments":arguments,"_meta":{"threadId":SESSION_B}
        }})
    };
    harness
        .send(join(
            400,
            json!({"code": "k7p3 q9xw 2dma", "displayName": "Scout"}),
        ))
        .await;
    let joined = harness.receive().await["result"].clone();
    assert_ne!(joined["isError"], true, "{joined}");
    let first = &joined["structuredContent"];
    assert_eq!(first["identity"], "created");
    assert_eq!(first["displayName"], "Scout");
    assert_eq!(first["room"]["name"], "项目室");
    assert_eq!(first["target"]["catalogId"], project.catalog_id.as_str());
    let key = first["sessionKey"].as_str().unwrap().to_owned();
    assert_eq!(
        bridge.redeemed.lock().unwrap().as_slice(),
        &[agent_room_bridge_ipc::IpcRedeemJoinCodeRequest {
            session_key: key.clone(),
            display_name: "Scout".to_owned(),
            code: "k7p3 q9xw 2dma".to_owned(),
        }]
    );
    assert_eq!(
        bridge.calls.lock().unwrap().as_slice(),
        &["resolve_join_code", "redeem_join_code", "open_host_session"]
    );

    // 同一任务再凭口令接入同一房间：回到同一人物，只是再兑换一次。
    harness
        .send(join(401, json!({"code": "K7P3-Q9XW-2DMA"})))
        .await;
    let again = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(again["identity"], "reused", "{again}");
    assert_eq!(again["sessionKey"], key.as_str());
    // 之后只说“接入”也回到这个房间。
    harness.send(join(402, json!({}))).await;
    let back = harness.receive().await["result"]["structuredContent"].clone();
    assert_eq!(back["sessionKey"], key.as_str());

    // 口令不对：如实失败，不开会话；房间名和口令不能一起给。
    let opened = bridge.opened.lock().unwrap().len();
    harness
        .send(join(
            403,
            json!({"code": "0000-0000-0000", "displayName": "Pilot"}),
        ))
        .await;
    let wrong = harness.receive().await["result"].clone();
    assert_eq!(wrong["isError"], true);
    assert_eq!(
        wrong["structuredContent"]["code"],
        "bridge.join_code.not_found"
    );
    assert!(
        wrong["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("不要猜")
    );
    harness
        .send(join(404, json!({"room": "Lobby", "code": JOIN_CODE})))
        .await;
    let both = harness.receive().await["result"].clone();
    assert_eq!(
        both["structuredContent"]["code"],
        "agent.join.room_and_code"
    );
    assert_eq!(bridge.opened.lock().unwrap().len(), opened);
    assert_eq!(bridge.redeemed.lock().unwrap().len(), 2);
    harness.stop().await;
}

const JOIN_CODE: &str = "K7P3-Q9XW-2DMA";

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
        // 列房间不涉及会话；按名字接入的会话键由服务按宿主任务保存，调用方不传。
        if tool.name == "agent_room_list_rooms" {
            assert!(
                schema
                    .get("required")
                    .is_none_or(|r| r.as_array().is_some_and(Vec::is_empty))
            );
            continue;
        }
        if tool.name == "agent_room_join" {
            assert!(
                schema
                    .get("required")
                    .is_none_or(|r| r.as_array().is_some_and(Vec::is_empty))
            );
            assert!(schema["properties"].get("sessionId").is_none());
            assert!(schema["properties"].get("sessionKey").is_none());
            assert_eq!(schema["properties"]["displayName"]["maxLength"], 128);
            assert_eq!(schema["properties"]["code"]["maxLength"], 64);
            continue;
        }
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
