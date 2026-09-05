use std::sync::atomic::AtomicUsize;

use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcBridgeState, IpcOpenContentRequest, IpcSelfSummary,
};
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
