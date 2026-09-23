use std::sync::atomic::AtomicUsize;

use agent_room_bridge_core::{
    join_codes::{
        ControlPlaneJoinCodeGateway, JoinCodeFailure, JoinCodeFailureKind, JoinCodeResult,
        JoinCodeRoom,
    },
    onboarding::{BridgeDefaultAgent, ControlPlaneOnboardingResult, HostAgentRegistrationGateway},
};
use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcBridgeState, IpcOpenContentRequest, IpcRedeemJoinCodeRequest,
    IpcResolveJoinCodeRequest, IpcSelfSummary,
};
use agent_room_domain::ids::AgentCreationRequestId;
use tokio::sync::Notify;

use super::*;

#[derive(Default)]
struct TestFactory {
    starts: AtomicUsize,
    stops: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

struct TestHandler {
    summary: IpcSelfSummary,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl BridgeIpcRequestHandler for TestHandler {
    fn dispatch(&self, method: IpcMethod) -> BridgeIpcDispatchFuture<'_> {
        Box::pin(async move {
            if let IpcMethod::ListPreviews(request) = &method {
                if request.room_id.as_deref() == Some("!denied:test.invalid") {
                    return Err(session_failure("bridge.test.forbidden", false));
                }
                return Ok(IpcResponse::MessagePreviews {
                    previews: vec![],
                    next_cursor: None,
                });
            }
            if matches!(method, IpcMethod::OpenContent(_)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(IpcResponse::SelfSummary {
                summary: self.summary.clone(),
            })
        })
    }
}

#[tokio::test]
async fn 诊断不伪造取信证据也不延长空闲寿命() {
    let registry = HostSessionRegistry::new(Arc::new(TestFactory::default()));
    let id = open(&registry, request("Receiver")).await;
    identity(&registry, &id).await;
    let entry = registry.find(&id).await.expect("会话存在");
    let activity = *entry.activity.lock().await;
    for _ in 0..2 {
        let IpcResponse::HostSessionDiagnostics { sessions } = registry.diagnostics().await else {
            panic!("诊断响应");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].last_inbox_read_ago_ms, None);
    }
    assert_eq!(*entry.activity.lock().await, activity);
    let inbox = |room| {
        IpcMethod::ListPreviews(agent_room_bridge_ipc::IpcListPreviewsRequest {
            room_id: room,
            after_event_id: None,
            before_event_id: None,
            limit: 20,
        })
    };
    assert!(
        registry
            .execute(&id, inbox(Some("!denied:test.invalid".into())))
            .await
            .is_err()
    );
    assert!(entry.evidence.lock().await.inbox_read.is_none());
    registry
        .execute(&id, inbox(None))
        .await
        .expect("成功空收件箱");
    let IpcResponse::HostSessionDiagnostics { sessions } = registry.diagnostics().await else {
        panic!("诊断响应");
    };
    assert!(sessions[0].last_inbox_read_ago_ms.is_some());
    assert!(sessions[0].last_message_received_ago_ms.is_none());
    registry.close(&id).await.expect("关闭");
    let IpcResponse::HostSessionDiagnostics { sessions } = registry.diagnostics().await else {
        panic!("诊断响应");
    };
    assert!(sessions.is_empty());
}

#[test]
fn 未确认的提交不能被诊断为发信成功() {
    use agent_room_bridge_ipc::{IpcSentMessage, IpcSubmissionState};
    let mut evidence = HostCallEvidence::default();
    for state in [
        IpcSubmissionState::UnknownCommit,
        IpcSubmissionState::BindingPending,
        IpcSubmissionState::Submitted,
    ] {
        evidence.record(&IpcResponse::SentMessage {
            message: IpcSentMessage {
                submission_id: Uuid::now_v7().to_string(),
                state,
                event_id: None,
            },
        });
        assert_eq!(
            evidence.message_sent.is_some(),
            state == IpcSubmissionState::Submitted
        );
    }
}

impl HostSessionFactory for TestFactory {
    fn prepare(
        &self,
        request: IpcOpenHostSessionRequest,
        mut shutdown: watch::Receiver<bool>,
    ) -> PortFuture<'_, Result<PreparedHostSession, BridgeIpcDispatchFailure>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::AcqRel);
            let handler = TestHandler {
                summary: IpcSelfSummary {
                    room_catalog_id: None,
                    agent: IpcAgentSummary {
                        agent_id: request.session_key.clone(),
                        display_name: request.display_name,
                        matrix_user_id: format!("@agent_{}:test.invalid", request.session_key),
                        avatar_url: None,
                    },
                    instance_id: Uuid::now_v7().to_string(),
                    matrix_device_id: Uuid::now_v7().to_string(),
                    room_id: "!lobby:test.invalid".to_owned(),
                    connection_state: IpcBridgeState::Ready,
                    granted_capabilities: vec![],
                },
                entered: self.entered.clone(),
                release: self.release.clone(),
            };
            let stops = self.stops.clone();
            Ok(PreparedHostSession {
                handler: Arc::new(handler),
                run: Box::pin(async move {
                    if !*shutdown.borrow() {
                        let _ = shutdown.changed().await;
                    }
                    stops.fetch_add(1, Ordering::AcqRel);
                }),
            })
        })
    }
}

fn request(name: &str) -> IpcOpenHostSessionRequest {
    IpcOpenHostSessionRequest {
        room: None,
        session_key: Uuid::now_v7().to_string(),
        display_name: name.into(),
    }
}

async fn open(registry: &HostSessionRegistry, request: IpcOpenHostSessionRequest) -> String {
    let IpcResponse::HostSession { session } = registry.open(request).await.expect("创建会话")
    else {
        panic!("必须返回会话")
    };
    session.session_id
}

async fn identity(registry: &HostSessionRegistry, id: &str) -> IpcSelfSummary {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match registry.execute(id, IpcMethod::GetSelf).await {
                Ok(IpcResponse::SelfSummary { summary }) => return summary,
                Err(error) if error.retryable() => tokio::task::yield_now().await,
                result => panic!("身份不可用：{result:?}"),
            }
        }
    })
    .await
    .expect("身份应及时就绪")
}

#[tokio::test]
async fn 桌面恢复清单只列活动会话且关闭后移除() {
    let registry = HostSessionRegistry::new(Arc::new(TestFactory::default()));
    let a = open(&registry, request("Builder")).await;
    let b = open(&registry, request("Scout")).await;
    identity(&registry, &a).await;
    identity(&registry, &b).await;
    let IpcResponse::RecoverySessions { sessions } = registry.recovery_sessions().await else {
        panic!("必须返回恢复会话摘要");
    };
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|entry| entry.session_id == a
        && entry.display_name == "Builder"
        && entry.state == IpcHostSessionState::Ready));
    registry.close(&a).await.expect("关闭 Builder");
    let IpcResponse::RecoverySessions { sessions } = registry.recovery_sessions().await else {
        panic!("必须返回恢复会话摘要");
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, b);
    registry.shutdown().await;
}

#[tokio::test]
async fn 三个人物并发路由到独立身份且关闭一个不影响其他人() {
    let factory = Arc::new(TestFactory::default());
    let registry = HostSessionRegistry::new(factory.clone());
    let (a, b, c) = tokio::join!(
        open(&registry, request("发起者")),
        open(&registry, request("应答者")),
        open(&registry, request("观察者"))
    );
    let (a_identity, b_identity, c_identity) = tokio::join!(
        identity(&registry, &a),
        identity(&registry, &b),
        identity(&registry, &c)
    );
    assert_eq!(a_identity.agent.display_name, "发起者");
    assert_eq!(b_identity.agent.display_name, "应答者");
    assert_eq!(c_identity.agent.display_name, "观察者");
    assert_ne!(a_identity.agent.agent_id, b_identity.agent.agent_id);
    assert_ne!(a_identity.instance_id, b_identity.instance_id);
    assert_ne!(b_identity.agent.agent_id, c_identity.agent.agent_id);
    registry.close(&a).await.expect("关闭发起者");
    assert_eq!(
        registry
            .execute(&a, IpcMethod::GetSelf)
            .await
            .expect_err("关闭后拒绝")
            .code(),
        "bridge.host_session.not_found"
    );
    assert_eq!(identity(&registry, &b).await, b_identity);
    assert_eq!(identity(&registry, &c).await, c_identity);
    assert_eq!(factory.stops.load(Ordering::Acquire), 1);
    registry.shutdown().await;
    assert_eq!(factory.stops.load(Ordering::Acquire), 3);
}

#[tokio::test]
async fn 并发打开同一会话仅初始化一次且名称冲突不会覆盖身份() {
    let factory = Arc::new(TestFactory::default());
    let registry = HostSessionRegistry::new(factory.clone());
    let request = request("同一个任务");
    let (first, retry) = tokio::join!(
        open(&registry, request.clone()),
        open(&registry, request.clone())
    );
    assert_eq!(first, retry);
    let original = identity(&registry, &first).await;
    assert_eq!(factory.starts.load(Ordering::Acquire), 1);
    let conflicting = IpcOpenHostSessionRequest {
        room: None,
        display_name: "另一个名字".into(),
        ..request
    };
    assert_eq!(
        registry
            .open(conflicting)
            .await
            .expect_err("冲突不能覆盖")
            .code(),
        "bridge.host_session.binding_conflict"
    );
    assert_eq!(identity(&registry, &first).await, original);
    registry.shutdown().await;
}

#[tokio::test]
async fn 关闭等待在途调用且立即拒绝该人物的新调用() {
    let factory = Arc::new(TestFactory::default());
    let registry = Arc::new(HostSessionRegistry::new(factory.clone()));
    let id = open(&registry, request("有在途操作的人物")).await;
    identity(&registry, &id).await;
    let call_registry = registry.clone();
    let call_id = id.clone();
    let call = tokio::spawn(async move {
        call_registry
            .execute(
                &call_id,
                IpcMethod::OpenContent(IpcOpenContentRequest {
                    room_id: None,
                    content_id: Uuid::now_v7().to_string(),
                }),
            )
            .await
    });
    factory.entered.notified().await;
    let close_registry = registry.clone();
    let close_id = id.clone();
    let close = tokio::spawn(async move { close_registry.close(&close_id).await });
    let session = registry.find(&id).await.unwrap();
    while !session.closing.load(Ordering::Acquire) {
        tokio::task::yield_now().await;
    }
    assert!(!close.is_finished());
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(1),
            registry.execute(&id, IpcMethod::GetSelf)
        )
        .await
        .expect("关闭中的新调用应立即拒绝")
        .expect_err("关闭中不得读身份")
        .code(),
        "bridge.host_session.closed"
    );
    assert_eq!(factory.stops.load(Ordering::Acquire), 0);
    factory.release.notify_one();
    assert!(call.await.unwrap().is_ok());
    assert!(close.await.unwrap().is_ok());
    assert!(registry.execute(&id, IpcMethod::GetSelf).await.is_err());
    assert_eq!(factory.stops.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn 未绑定的会话不退回默认身份且关闭后同键可恢复为新连接() {
    let registry = HostSessionRegistry::new(Arc::new(TestFactory::default()));
    assert!(
        registry
            .execute(&Uuid::now_v7().to_string(), IpcMethod::GetSelf)
            .await
            .is_err()
    );
    let request = request("可重连人物");
    let first = open(&registry, request.clone()).await;
    let original = identity(&registry, &first).await;
    registry.close(&first).await.unwrap();
    let second = open(&registry, request).await;
    assert_ne!(first, second);
    assert_eq!(identity(&registry, &second).await.agent, original.agent);
    assert!(registry.execute(&first, IpcMethod::GetSelf).await.is_err());
    registry.shutdown().await;
}

#[tokio::test]
async fn 超时空闲会话会停止续租并释放容量() {
    let registry = HostSessionRegistry::new(Arc::new(TestFactory::default()));
    let id = open(&registry, request("已离开的任务")).await;
    identity(&registry, &id).await;
    *registry.find(&id).await.unwrap().activity.lock().await = Instant::now() - IDLE_LIFETIME;
    registry.expire_idle().await;
    assert!(registry.find(&id).await.is_err());
}

#[tokio::test]
async fn 重复关闭和未知规范句柄的关闭均为幂等() {
    let factory = Arc::new(TestFactory::default());
    let registry = HostSessionRegistry::new(factory.clone());
    let id = open(&registry, request("幂等关闭人物")).await;
    identity(&registry, &id).await;
    let closed = registry.close(&id).await.unwrap();
    assert_eq!(registry.close(&id).await.unwrap(), closed);
    assert_eq!(factory.stops.load(Ordering::Acquire), 1);
    assert!(registry.close(&Uuid::now_v7().to_string()).await.is_ok());
    assert!(registry.close("invalid-id").await.is_err());
}

#[derive(Default)]
struct PendingFactory {
    delegate: TestFactory,
    entered: Notify,
    release: Notify,
}

impl HostSessionFactory for PendingFactory {
    fn prepare(
        &self,
        request: IpcOpenHostSessionRequest,
        shutdown: watch::Receiver<bool>,
    ) -> PortFuture<'_, Result<PreparedHostSession, BridgeIpcDispatchFailure>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            self.delegate.prepare(request, shutdown).await
        })
    }
}

#[tokio::test]
async fn 初始化中关闭不会取消共享认证且关闭请求取消后仍可等待清理() {
    let factory = Arc::new(PendingFactory::default());
    let registry = Arc::new(HostSessionRegistry::new(factory.clone()));
    let request = request("初始化中的人物");
    let id = open(&registry, request.clone()).await;
    factory.entered.notified().await;
    let worker_registry = registry.clone();
    let worker_id = id.clone();
    let close = tokio::spawn(async move { worker_registry.close(&worker_id).await });
    let session = registry.find(&id).await.unwrap();
    while !*session.shutdown.borrow() {
        tokio::task::yield_now().await;
    }
    assert!(!close.is_finished());
    close.abort();
    assert!(close.await.unwrap_err().is_cancelled());
    assert!(registry.open(request).await.is_err());
    factory.release.notify_one();
    registry.close(&id).await.unwrap();
    assert_eq!(factory.delegate.starts.load(Ordering::Acquire), 1);
    assert_eq!(factory.delegate.stops.load(Ordering::Acquire), 1);
    assert!(registry.execute(&id, IpcMethod::GetSelf).await.is_err());
}

#[tokio::test]
async fn 达到容量时拒绝新人物而已有会话仍可重试和操作() {
    let registry = HostSessionRegistry::new(Arc::new(TestFactory::default()));
    let first_request = request("保留的人物");
    let first = open(&registry, first_request.clone()).await;
    for _ in 1..MAX_HOST_SESSIONS {
        open(&registry, request("其他人物")).await;
    }
    assert_eq!(
        registry.open(request("超限人物")).await.unwrap_err().code(),
        "bridge.host_session.limit_reached"
    );
    assert_eq!(open(&registry, first_request).await, first);
    identity(&registry, &first).await;
    registry.close(&first).await.unwrap();
    open(&registry, request("空位人物")).await;
    registry.shutdown().await;
    assert!(registry.open(request("退出后人物")).await.is_err());
}

#[derive(Default)]
struct RecoveringFactory {
    delegate: TestFactory,
    attempts: AtomicUsize,
    failed: Notify,
}

impl HostSessionFactory for RecoveringFactory {
    fn prepare(
        &self,
        request: IpcOpenHostSessionRequest,
        shutdown: watch::Receiver<bool>,
    ) -> PortFuture<'_, Result<PreparedHostSession, BridgeIpcDispatchFailure>> {
        Box::pin(async move {
            if self.attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                self.failed.notify_one();
                return Err(session_failure(
                    "bridge.host_session.registration_unavailable",
                    true,
                ));
            }
            self.delegate.prepare(request, shutdown).await
        })
    }
}

#[tokio::test]
async fn 注册暂时失败会按原会话绑定重试并恢复同一连接() {
    let factory = Arc::new(RecoveringFactory::default());
    let registry = HostSessionRegistry::new(factory.clone());
    let request = request("网络恢复后上线的人物");
    let id = open(&registry, request.clone()).await;
    factory.failed.notified().await;
    assert_eq!(open(&registry, request.clone()).await, id);
    assert_eq!(
        identity(&registry, &id).await.agent.agent_id,
        request.session_key
    );
    assert_eq!(factory.attempts.load(Ordering::Acquire), 2);
    assert_eq!(factory.delegate.starts.load(Ordering::Acquire), 1);
    registry.shutdown().await;
}

#[tokio::test]
async fn 登记仅保存本任务资料且不会改变身份或自动发送() {
    let factory = Arc::new(TestFactory::default());
    let registry = HostSessionRegistry::new(factory.clone());
    let original = request("Receiver");
    let id = open(&registry, original.clone()).await;
    let summary = identity(&registry, &id).await;
    let offer = agent_room_bridge_ipc::IpcRegisterReceptionRequest {
        host_type: agent_room_bridge_ipc::IpcReceptionHost::Codex,
        task_id: Uuid::now_v7().to_string(),
        workspace: "C:/work".into(),
    };
    for _ in 0..2 {
        registry
            .execute(&id, IpcMethod::RegisterReception(offer.clone()))
            .await
            .unwrap();
    }
    let IpcResponse::HostSessionDiagnostics { sessions } = registry.diagnostics().await else {
        panic!("diagnostics");
    };
    assert_eq!(
        sessions[0].session_key.as_deref(),
        Some(original.session_key.as_str())
    );
    assert_eq!(sessions[0].reception_offer.as_ref().unwrap().task, offer);
    assert_eq!(
        sessions[0].reception_offer.as_ref().unwrap().room_id,
        summary.room_id
    );
    assert!(sessions[0].last_message_sent_ago_ms.is_none());
    assert_eq!(factory.starts.load(Ordering::Acquire), 1);
    let mut different = offer;
    different.task_id = Uuid::now_v7().to_string();
    assert_eq!(
        registry
            .execute(&id, IpcMethod::RegisterReception(different))
            .await
            .unwrap_err()
            .code(),
        "bridge.host_session.reception_already_bound"
    );
    assert_eq!(identity(&registry, &id).await, summary);
    registry.shutdown().await;
}

/// 可进房间目录：CLI/MCP 在开会话之前就能按名字解析房间。
struct DirectoryFake {
    rooms: Vec<AccessibleRoom>,
}

impl ControlPlaneRoomDirectoryGateway for DirectoryFake {
    fn list_accessible(
        &self,
    ) -> PortFuture<
        '_,
        agent_room_bridge_core::room_directory::RoomDirectoryResult<Vec<AccessibleRoom>>,
    > {
        let rooms = self.rooms.clone();
        Box::pin(async move { Ok(rooms) })
    }
}

/// 控制面的口令接口：只认一个口令，记下查看与兑换的调用。
#[derive(Default)]
struct JoinCodesFake {
    resolved: std::sync::Mutex<Vec<String>>,
    redeemed: std::sync::Mutex<Vec<(agent_room_domain::ids::AgentId, String)>>,
}

impl ControlPlaneJoinCodeGateway for JoinCodesFake {
    fn resolve<'a>(&'a self, code: &'a str) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>> {
        self.resolved.lock().unwrap().push(code.to_owned());
        Box::pin(async move { code_room(code) })
    }

    fn redeem<'a>(
        &'a self,
        agent_id: agent_room_domain::ids::AgentId,
        code: &'a str,
    ) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>> {
        self.redeemed
            .lock()
            .unwrap()
            .push((agent_id, code.to_owned()));
        Box::pin(async move { code_room(code) })
    }
}

const JOIN_CODE: &str = "K7P3-Q9XW-2DMA";

fn code_room(code: &str) -> JoinCodeResult<JoinCodeRoom> {
    if code != JOIN_CODE {
        return Err(JoinCodeFailure::new(JoinCodeFailureKind::NotFound));
    }
    Ok(JoinCodeRoom {
        catalog_id: agent_room_domain::ids::RoomCatalogId::from_uuid(Uuid::from_u128(7)),
        matrix_room_id: agent_room_domain::rooms::MatrixRoomReference::new(
            "!project:matrix.test".to_owned(),
        )
        .expect("Matrix 房间标识有效"),
        name: "项目室".to_owned(),
    })
}

/// 控制面按会话键幂等地建宿主人物：同一个键总是同一个 Agent。
#[derive(Default)]
struct HostAgentsFake {
    created: std::sync::Mutex<Vec<(AgentCreationRequestId, String)>>,
}

impl HostAgentRegistrationGateway for HostAgentsFake {
    fn create_host_agent<'a>(
        &'a self,
        session_key: AgentCreationRequestId,
        display_name: &'a str,
    ) -> PortFuture<'a, ControlPlaneOnboardingResult<BridgeDefaultAgent>> {
        self.created
            .lock()
            .unwrap()
            .push((session_key, display_name.to_owned()));
        let agent = BridgeDefaultAgent {
            agent_id: agent_room_domain::ids::AgentId::from_uuid(session_key.as_uuid()),
            display_name: display_name.to_owned(),
        };
        Box::pin(async move { Ok(agent) })
    }
}

fn join_codes(gateway: Arc<JoinCodesFake>, host_agents: Arc<HostAgentsFake>) -> JoinCodeAccess {
    JoinCodeAccess {
        gateway,
        host_agents,
    }
}

struct RefusingHandler;

impl BridgeIpcRequestHandler for RefusingHandler {
    fn dispatch(&self, _method: IpcMethod) -> BridgeIpcDispatchFuture<'_> {
        Box::pin(async { Err(session_failure("bridge.test.unexpected_default", false)) })
    }
}

struct ReadyStatus;

impl crate::ipc::BridgeStatusReader for ReadyStatus {
    fn read_status(&self) -> crate::ipc::BridgeStatusSnapshot {
        crate::ipc::BridgeStatusSnapshot {
            state: IpcBridgeState::Ready,
            started_at_unix_ms: 1,
        }
    }
}

#[tokio::test]
async fn 列房间不需要会话_公开大厅与私人房间都按目录原样返回() {
    let catalog = agent_room_domain::ids::RoomCatalogId::from_uuid(Uuid::now_v7());
    let handler = SessionAwareIpcHandler {
        default: Arc::new(RefusingHandler),
        sessions: Arc::new(HostSessionRegistry::new(Arc::new(TestFactory::default()))),
        connection_status: Arc::new(ReadyStatus),
        room_directory: Arc::new(DirectoryFake {
            rooms: vec![
                AccessibleRoom {
                    kind: AccessibleRoomKind::PublicLobby,
                    catalog_id: catalog,
                    matrix_room_id: None,
                    name: "Lobby".to_owned(),
                    slug: Some("lobby".to_owned()),
                    membership: None,
                },
                AccessibleRoom {
                    kind: AccessibleRoomKind::PrivateRoom,
                    catalog_id: catalog,
                    matrix_room_id: Some(
                        agent_room_domain::rooms::MatrixRoomReference::new(
                            "!private:matrix.test".to_owned(),
                        )
                        .expect("Matrix 房间标识有效"),
                    ),
                    name: "game dev".to_owned(),
                    slug: None,
                    membership: Some(AccessibleRoomMembership::Joined),
                },
            ],
        }),
        invitations: InvitationSlot::default(),
        join_codes: join_codes(Arc::new(JoinCodesFake::default()), Arc::default()),
    };

    let response = handler
        .dispatch(IpcMethod::ListRooms)
        .await
        .expect("目录可读");
    let IpcResponse::Rooms { rooms } = response else {
        panic!("应返回房间列表");
    };
    assert_eq!(rooms.len(), 2);
    assert_eq!(rooms[0].kind, IpcRoomKind::PublicLobby);
    assert_eq!(rooms[0].slug.as_deref(), Some("lobby"));
    assert_eq!(rooms[1].kind, IpcRoomKind::PrivateRoom);
    assert_eq!(
        rooms[1].matrix_room_id.as_deref(),
        Some("!private:matrix.test")
    );
    assert_eq!(rooms[1].membership, Some(IpcRoomMembership::Joined));
    assert_eq!(rooms[1].catalog_id, catalog.to_string());
}

#[tokio::test]
async fn 等待接入的邀请可反复查看_开出会话才用掉_已开的键不能再挂() {
    let handler = SessionAwareIpcHandler {
        default: Arc::new(RefusingHandler),
        sessions: Arc::new(HostSessionRegistry::new(Arc::new(TestFactory::default()))),
        connection_status: Arc::new(ReadyStatus),
        room_directory: Arc::new(DirectoryFake { rooms: vec![] }),
        invitations: InvitationSlot::default(),
        join_codes: join_codes(Arc::new(JoinCodesFake::default()), Arc::default()),
    };
    let pending = |response: IpcResponse| {
        let IpcResponse::Invitation { invitation } = response else {
            panic!("应返回邀请");
        };
        invitation.map(|pending| pending.invitation)
    };
    // 面板没定名字：接上的 Agent 用自己起的名字开会话，同样用掉这份邀请。
    let opened = request("Scout");
    let offered = agent_room_bridge_ipc::IpcInvitationOffer {
        session_key: opened.session_key.clone(),
        display_name: None,
        room: None,
    };
    assert_eq!(
        pending(
            handler
                .dispatch(IpcMethod::OfferInvitation(offered.clone()))
                .await
                .expect("可挂出")
        ),
        Some(offered.clone())
    );
    for _ in 0..2 {
        assert_eq!(
            pending(
                handler
                    .dispatch(IpcMethod::ReadInvitation)
                    .await
                    .expect("可查看")
            ),
            Some(offered.clone())
        );
    }
    handler
        .dispatch(IpcMethod::OpenHostSession(
            offered.open_request(|| opened.display_name.clone()),
        ))
        .await
        .expect("用邀请开出会话");
    assert_eq!(
        pending(
            handler
                .dispatch(IpcMethod::ReadInvitation)
                .await
                .expect("可查看")
        ),
        None
    );
    // 面板定时续期时这个人物已经在房间里：不能再挂出去给第二个 Agent。
    assert_eq!(
        pending(
            handler
                .dispatch(IpcMethod::OfferInvitation(offered))
                .await
                .expect("可调用")
        ),
        None
    );
    assert_eq!(
        pending(
            handler
                .dispatch(IpcMethod::ReadInvitation)
                .await
                .expect("可查看")
        ),
        None
    );
}

#[tokio::test]
async fn 凭口令查看房间不建人物_兑换先按会话键建好人物再让它加入() {
    let gateway = Arc::new(JoinCodesFake::default());
    let host_agents = Arc::new(HostAgentsFake::default());
    let handler = SessionAwareIpcHandler {
        default: Arc::new(RefusingHandler),
        sessions: Arc::new(HostSessionRegistry::new(Arc::new(TestFactory::default()))),
        connection_status: Arc::new(ReadyStatus),
        room_directory: Arc::new(DirectoryFake { rooms: vec![] }),
        invitations: InvitationSlot::default(),
        join_codes: join_codes(gateway.clone(), host_agents.clone()),
    };
    let room = |response: IpcResponse| {
        let IpcResponse::JoinCodeRoom { room } = response else {
            panic!("应返回口令对应的房间");
        };
        room
    };

    let resolved = room(
        handler
            .dispatch(IpcMethod::ResolveJoinCode(IpcResolveJoinCodeRequest {
                code: JOIN_CODE.to_owned(),
            }))
            .await
            .expect("查看房间"),
    );
    assert_eq!(resolved.kind, IpcRoomKind::PrivateRoom);
    assert_eq!(resolved.name, "项目室");
    assert_eq!(
        resolved.matrix_room_id.as_deref(),
        Some("!project:matrix.test")
    );
    assert!(
        host_agents.created.lock().unwrap().is_empty(),
        "只看不建人物"
    );
    assert!(gateway.redeemed.lock().unwrap().is_empty());

    let session_key = Uuid::now_v7();
    let redeemed = room(
        handler
            .dispatch(IpcMethod::RedeemJoinCode(IpcRedeemJoinCodeRequest {
                session_key: session_key.to_string(),
                display_name: "Scout".to_owned(),
                code: JOIN_CODE.to_owned(),
            }))
            .await
            .expect("兑换口令"),
    );
    assert_eq!(redeemed, resolved);
    assert_eq!(
        host_agents.created.lock().unwrap().as_slice(),
        &[(
            AgentCreationRequestId::from_uuid(session_key),
            "Scout".to_owned()
        )]
    );
    assert_eq!(
        gateway.redeemed.lock().unwrap().as_slice(),
        &[(
            agent_room_domain::ids::AgentId::from_uuid(session_key),
            JOIN_CODE.to_owned()
        )]
    );

    let wrong = handler
        .dispatch(IpcMethod::ResolveJoinCode(IpcResolveJoinCodeRequest {
            code: "0000-0000-0000".to_owned(),
        }))
        .await
        .expect_err("口令不对");
    assert_eq!(wrong.code(), "bridge.join_code.not_found");
    assert!(!wrong.retryable());
}
