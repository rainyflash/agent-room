use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_room_application::ports::PortFuture;
use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcHostSessionState, IpcHostSessionSummary, IpcMethod,
    IpcOpenHostSessionRequest, IpcResponse,
};
use tokio::{
    sync::{Mutex, RwLock, watch},
    task::JoinHandle,
    time::Instant,
};
use uuid::Uuid;

use crate::ipc::{BridgeIpcDispatchFailure, BridgeIpcDispatchFuture, BridgeIpcRequestHandler};

const MAX_HOST_SESSIONS: usize = 16;
const IDLE_LIFETIME: Duration = Duration::from_mins(15);

/// 工厂负责资源组合；注册表只处理绑定、并发分派和关闭次序。
pub(crate) trait HostSessionFactory: Send + Sync {
    fn prepare(
        &self,
        request: IpcOpenHostSessionRequest,
        shutdown: watch::Receiver<bool>,
    ) -> PortFuture<'_, Result<PreparedHostSession, BridgeIpcDispatchFailure>>;
}

pub(crate) struct PreparedHostSession {
    pub(crate) handler: Arc<dyn BridgeIpcRequestHandler>,
    pub(crate) run: PortFuture<'static, ()>,
}

enum SessionPhase {
    Starting,
    Running(Arc<dyn BridgeIpcRequestHandler>),
    Failed(BridgeIpcDispatchFailure),
    Closed,
}

struct HostSession {
    id: String,
    request: IpcOpenHostSessionRequest,
    phase: RwLock<SessionPhase>,
    closing: AtomicBool,
    activity: Mutex<Instant>,
    /// 读锁覆盖完整业务调用；关闭拿写锁后才能释放身份与存储。
    calls: RwLock<()>,
    shutdown: watch::Sender<bool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl HostSession {
    async fn summary(&self) -> IpcHostSessionSummary {
        let (state, agent_id, error_code) = match &*self.phase.read().await {
            SessionPhase::Starting => (IpcHostSessionState::Starting, None, None),
            SessionPhase::Running(handler) => match handler.dispatch(IpcMethod::GetSelf).await {
                Ok(IpcResponse::SelfSummary { summary }) => (
                    match summary.connection_state {
                        agent_room_bridge_ipc::IpcBridgeState::Ready => IpcHostSessionState::Ready,
                        agent_room_bridge_ipc::IpcBridgeState::Starting
                        | agent_room_bridge_ipc::IpcBridgeState::Reconnecting => {
                            IpcHostSessionState::Starting
                        }
                        agent_room_bridge_ipc::IpcBridgeState::Offline => {
                            IpcHostSessionState::Failed
                        }
                        agent_room_bridge_ipc::IpcBridgeState::ShuttingDown => {
                            IpcHostSessionState::Closed
                        }
                    },
                    Some(summary.agent.agent_id),
                    None,
                ),
                Err(failure) if failure.retryable() => (
                    IpcHostSessionState::Starting,
                    None,
                    Some(failure.code().to_owned()),
                ),
                Err(failure) => (
                    IpcHostSessionState::Failed,
                    None,
                    Some(failure.code().to_owned()),
                ),
                Ok(_) => (
                    IpcHostSessionState::Failed,
                    None,
                    Some("bridge.host_session.response_invalid".to_owned()),
                ),
            },
            SessionPhase::Failed(failure) => (
                if failure.retryable() {
                    IpcHostSessionState::Starting
                } else {
                    IpcHostSessionState::Failed
                },
                None,
                Some(failure.code().to_owned()),
            ),
            SessionPhase::Closed => (IpcHostSessionState::Closed, None, None),
        };
        IpcHostSessionSummary {
            session_id: self.id.clone(),
            state,
            agent_id,
            error_code,
        }
    }

    async fn execute(&self, method: IpcMethod) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        if self.closing.load(Ordering::Acquire) {
            return Err(session_failure("bridge.host_session.closed", false));
        }
        let _call = self.calls.read().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(session_failure("bridge.host_session.closed", false));
        }
        *self.activity.lock().await = Instant::now();
        if self.closing.load(Ordering::Acquire) {
            return Err(session_failure("bridge.host_session.closed", false));
        }
        let handler = match &*self.phase.read().await {
            SessionPhase::Running(handler) => handler.clone(),
            SessionPhase::Starting => {
                return Err(session_failure("bridge.host_session.starting", true));
            }
            SessionPhase::Failed(failure) => return Err(*failure),
            SessionPhase::Closed => {
                return Err(session_failure("bridge.host_session.closed", false));
            }
        };
        handler.dispatch(method).await
    }

    async fn stop(&self) -> Result<(), BridgeIpcDispatchFailure> {
        self.closing.store(true, Ordering::Release);
        let _drained = self.calls.write().await;
        // 先排空调用并撤销快照引用，再通知运行时退出并等待资源释放。
        *self.phase.write().await = SessionPhase::Closed;
        self.shutdown.send_replace(true);
        let mut worker = self.worker.lock().await;
        if let Some(task) = worker.as_mut() {
            // 关闭请求可能被宿主取消；保留句柄，重试仍必须等待同一个任务释放资源。
            let result = task.await;
            worker.take();
            result.map_err(|_| session_failure("bridge.host_session.worker_failed", false))?;
        }
        Ok(())
    }
}

pub(crate) struct HostSessionRegistry {
    factory: Arc<dyn HostSessionFactory>,
    sessions: Mutex<BTreeMap<String, Arc<HostSession>>>,
    closing: AtomicBool,
}

impl HostSessionRegistry {
    pub(crate) fn new(factory: Arc<dyn HostSessionFactory>) -> Self {
        Self {
            factory,
            sessions: Mutex::new(BTreeMap::new()),
            closing: AtomicBool::new(false),
        }
    }

    pub(crate) async fn open(
        &self,
        request: IpcOpenHostSessionRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        IpcMethod::OpenHostSession(request.clone())
            .validate()
            .map_err(|_| session_failure("bridge.host_session.request_invalid", false))?;
        let mut sessions = self.sessions.lock().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(session_failure("bridge.host_session.shutting_down", false));
        }
        if let Some(session) = sessions.get(&request.session_key).cloned() {
            if session.request != request || session.closing.load(Ordering::Acquire) {
                return Err(BridgeIpcDispatchFailure::new(
                    "bridge.host_session.binding_conflict",
                    IpcErrorCategory::Conflict,
                    false,
                ));
            }
            drop(sessions);
            let _call = session.calls.read().await;
            if session.closing.load(Ordering::Acquire) {
                return Err(session_failure("bridge.host_session.closed", false));
            }
            *session.activity.lock().await = Instant::now();
            if session.closing.load(Ordering::Acquire) {
                return Err(session_failure("bridge.host_session.closed", false));
            }
            return Ok(IpcResponse::HostSession {
                session: session.summary().await,
            });
        }
        if sessions.len() >= MAX_HOST_SESSIONS {
            return Err(BridgeIpcDispatchFailure::new(
                "bridge.host_session.limit_reached",
                IpcErrorCategory::Conflict,
                false,
            ));
        }
        let (shutdown, receiver) = watch::channel(false);
        let session = Arc::new(HostSession {
            id: Uuid::now_v7().to_string(),
            request: request.clone(),
            phase: RwLock::new(SessionPhase::Starting),
            closing: AtomicBool::new(false),
            activity: Mutex::new(Instant::now()),
            calls: RwLock::new(()),
            shutdown,
            worker: Mutex::new(None),
        });
        let worker_session = session.clone();
        let factory = self.factory.clone();
        let worker = tokio::spawn(async move {
            run_host_session(factory, worker_session, receiver).await;
        });
        *session.worker.lock().await = Some(worker);
        sessions.insert(request.session_key, session.clone());
        drop(sessions);
        Ok(IpcResponse::HostSession {
            session: IpcHostSessionSummary {
                session_id: session.id.clone(),
                state: IpcHostSessionState::Starting,
                agent_id: None,
                error_code: None,
            },
        })
    }

    pub(crate) async fn execute(
        &self,
        session_id: &str,
        method: IpcMethod,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        IpcMethod::WithSession {
            session_id: session_id.to_owned(),
            method: Box::new(method.clone()),
        }
        .validate()
        .map_err(|_| session_failure("bridge.host_session.request_invalid", false))?;
        self.find(session_id).await?.execute(method).await
    }

    async fn find(&self, session_id: &str) -> Result<Arc<HostSession>, BridgeIpcDispatchFailure> {
        self.sessions
            .lock()
            .await
            .values()
            .find(|session| session.id == session_id)
            .cloned()
            .ok_or_else(|| session_failure("bridge.host_session.not_found", false))
    }

    pub(crate) async fn close(
        &self,
        session_id: &str,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        IpcMethod::CloseHostSession(agent_room_bridge_ipc::IpcCloseHostSessionRequest {
            session_id: session_id.to_owned(),
        })
        .validate()
        .map_err(|_| session_failure("bridge.host_session.request_invalid", false))?;
        let Ok(session) = self.find(session_id).await else {
            // 关闭只保证句柄不再可调用；重复请求不需要保留无限增长的墓碑记录。
            return Ok(IpcResponse::HostSession {
                session: IpcHostSessionSummary {
                    session_id: session_id.to_owned(),
                    state: IpcHostSessionState::Closed,
                    agent_id: None,
                    error_code: None,
                },
            });
        };
        session.stop().await?;
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(&session.request.session_key)
            .is_some_and(|current| Arc::ptr_eq(current, &session))
        {
            sessions.remove(&session.request.session_key);
        }
        Ok(IpcResponse::HostSession {
            session: session.summary().await,
        })
    }

    pub(crate) async fn expire_idle(&self) {
        let entries: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        for session in entries {
            let expired = {
                let activity = session.activity.lock().await;
                let expired = activity.elapsed() >= IDLE_LIFETIME;
                if expired {
                    session.closing.store(true, Ordering::Release);
                }
                expired
            };
            if expired && let Err(failure) = self.close(&session.id).await {
                tracing::warn!(error_code = failure.code(), "空闲 Agent 会话关闭失败");
            }
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        let entries: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        // 先拒绝全部新调用，随后逐一排空；其他 Agent 不受单个会话状态污染。
        for session in &entries {
            session.closing.store(true, Ordering::Release);
        }
        for session in entries {
            if let Err(failure) = self.close(&session.id).await {
                tracing::warn!(
                    error_code = failure.code(),
                    "Bridge 退出时 Agent 会话关闭失败"
                );
            }
        }
    }
}

async fn run_host_session(
    factory: Arc<dyn HostSessionFactory>,
    session: Arc<HostSession>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut retry_delay = Duration::from_secs(1);
    let prepared = loop {
        if *shutdown.borrow() {
            return;
        }
        // 初始化可能正在刷新共享设备令牌，不能中途取消并留下结果未知的刷新事务。
        let result = factory
            .prepare(session.request.clone(), shutdown.clone())
            .await;
        match result {
            Err(failure) if failure.retryable() && !session.closing.load(Ordering::Acquire) => {
                *session.phase.write().await = SessionPhase::Failed(failure);
                tokio::select! {
                    _ = shutdown.changed() => return,
                    () = tokio::time::sleep(retry_delay) => {},
                }
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
            }
            result => break result,
        }
    };
    match prepared {
        Ok(runtime) => {
            {
                let mut phase = session.phase.write().await;
                if !session.closing.load(Ordering::Acquire) {
                    *phase = SessionPhase::Running(runtime.handler);
                }
            }
            runtime.run.await;
            if !session.closing.load(Ordering::Acquire) {
                *session.phase.write().await =
                    SessionPhase::Failed(session_failure("bridge.host_session.stopped", false));
            }
        }
        Err(failure) => {
            let mut phase = session.phase.write().await;
            if !session.closing.load(Ordering::Acquire) {
                *phase = SessionPhase::Failed(failure);
            }
        }
    }
}

fn session_failure(code: &'static str, retryable: bool) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(code, IpcErrorCategory::DependencyUnavailable, retryable)
}

pub(crate) struct SessionAwareIpcHandler {
    pub(crate) default: Arc<dyn BridgeIpcRequestHandler>,
    pub(crate) sessions: Arc<HostSessionRegistry>,
}

impl BridgeIpcRequestHandler for SessionAwareIpcHandler {
    fn dispatch(&self, method: IpcMethod) -> BridgeIpcDispatchFuture<'_> {
        Box::pin(async move {
            match method {
                IpcMethod::OpenHostSession(request) => self.sessions.open(request).await,
                IpcMethod::CloseHostSession(request) => {
                    self.sessions.close(&request.session_id).await
                }
                IpcMethod::WithSession { session_id, method } => {
                    self.sessions.execute(&session_id, *method).await
                }
                method => self.default.dispatch(method).await,
            }
        })
    }
}

#[cfg(test)]
mod tests;
