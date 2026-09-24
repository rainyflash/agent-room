//! 网络 Agent 的网关（ADR 0010，2-收发）：服务器用 Agent 自己的 Matrix 会话替它同步房间，
//! 用本机 Bridge 同一套解析与验签把消息整理成预览，放进它的收件箱。Agent 凭令牌长轮询取，
//! 只有显式确认才往前走。一个 Agent 同时只有一次长轮询，新来的会让旧的立刻空手返回。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agent_room_application::{
    network_agents::{NetworkAgentFailure, NetworkAgentSession, NetworkAgentUseCases},
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, MatrixEventId,
        MatrixSyncBatch, NetworkAgentAckOutcome, NetworkAgentInboxAppend, NetworkAgentInboxChange,
        NetworkAgentInboxPage, NetworkAgentInboxStore, NetworkAgentMatrixGateway,
        NetworkAgentSyncRequest, PortFuture,
    },
};
use agent_room_bridge_core::{
    agent_verification::{
        AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
    },
    messages::{MessageSyncDependencies, MessageSyncService},
};
use agent_room_domain::ids::NetworkAgentId;
use serde_json::Value;
use tokio::{sync::Notify, time::Instant};

mod projection;
#[cfg(test)]
mod tests;

/// 长轮询最多等这么久。
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(30);
/// 一次最多取这么多条。
pub(crate) const MAX_PAGE: u16 = 50;
/// 第一次同步只带每个房间最近几条，给 Agent 一点上下文。
const FIRST_SYNC_TIMELINE_LIMIT: u16 = 20;
const SYNC_TIMELINE_LIMIT: u16 = 50;
/// 最多留这么多条没确认的；再多就丢掉最早的，并在下次取消息时告诉 Agent 丢了几条。
const INBOX_CAPACITY: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkGatewayFailure {
    /// 令牌、总开关这类身份问题，按网络 Agent 的错误码回答。
    Agent(NetworkAgentFailure),
    /// Matrix 或数据库暂时不可用，可以原样重试。
    Unavailable,
    /// 确认的事件 ID 格式不对。
    InvalidEvent,
}

/// 一次取到的消息：最早的在前。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NetworkAgentMessages {
    pub(crate) messages: Vec<Value>,
    /// 还没确认的总数，可能多于这一次取到的。
    pub(crate) pending: u64,
    /// 收件箱满了丢掉的条数，确认之后清零。
    pub(crate) dropped: u64,
}

/// 网络 Agent 收消息的接口；HTTP 层只认这个，便于单独测试。
pub(crate) trait NetworkAgentMessaging: Send + Sync {
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        wait: Duration,
        limit: u16,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>>;

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>>;
}

pub(crate) struct NetworkGatewayDependencies {
    pub(crate) agents: Arc<dyn NetworkAgentUseCases>,
    pub(crate) inbox: Arc<dyn NetworkAgentInboxStore>,
    pub(crate) matrix: Arc<dyn NetworkAgentMatrixGateway>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) clock: Arc<dyn Clock>,
}

pub(crate) struct NetworkGateway {
    agents: Arc<dyn NetworkAgentUseCases>,
    inbox: Arc<dyn NetworkAgentInboxStore>,
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    verification: Arc<dyn AgentInstanceVerificationRepository>,
    signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    clock: Arc<dyn Clock>,
    polls: LongPolls,
}

impl NetworkGateway {
    pub(crate) fn new(dependencies: NetworkGatewayDependencies) -> Self {
        Self {
            agents: dependencies.agents,
            inbox: dependencies.inbox,
            matrix: dependencies.matrix,
            verification: dependencies.verification,
            signatures: dependencies.signatures,
            clock: dependencies.clock,
            polls: LongPolls::default(),
        }
    }

    /// 取还没确认的消息；没有时最多等 `wait`，期间一有新消息就返回。
    async fn wait_internal(
        &self,
        token: &str,
        wait: Duration,
        limit: u16,
    ) -> Result<NetworkAgentMessages, NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let poll = self.polls.begin(session.network_agent_id);
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        let limit = limit.clamp(1, MAX_PAGE);
        loop {
            let page = self
                .inbox
                .pending(session.network_agent_id, limit)
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            // 第一次同步不等：先把房间里最近的几条拿来当上下文。
            let first = page.sync_token.is_none();
            if !page.entries.is_empty() || (remaining.is_zero() && !first) {
                return Ok(messages(page));
            }
            let request = NetworkAgentSyncRequest {
                since: page.sync_token.clone(),
                timeout_millis: if first {
                    0
                } else {
                    u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX)
                },
                timeline_limit: if first {
                    FIRST_SYNC_TIMELINE_LIMIT
                } else {
                    SYNC_TIMELINE_LIMIT
                },
            };
            let batch = tokio::select! {
                result = self.matrix.sync(&session.matrix_access_token, &request) => {
                    result.map_err(|_| NetworkGatewayFailure::Unavailable)?
                }
                () = poll.superseded() => return Ok(messages(page)),
            };
            let changes = self.changes(&session, &batch).await?;
            // 位置对不上说明另一次同步已经写过（例如被取代的旧长轮询），下一轮从新位置接着来。
            self.inbox
                .append(&NetworkAgentInboxAppend {
                    id: session.network_agent_id,
                    expected_sync_token: page.sync_token,
                    next_sync_token: batch.next_batch().clone(),
                    changes,
                    received_at: self.clock.now(),
                    capacity: INBOX_CAPACITY,
                })
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        }
    }

    /// 确认处理到这一条（含）为止。
    async fn acknowledge_internal(
        &self,
        token: &str,
        event_id: &str,
    ) -> Result<NetworkAgentAckOutcome, NetworkGatewayFailure> {
        let event_id =
            MatrixEventId::new(event_id).map_err(|_| NetworkGatewayFailure::InvalidEvent)?;
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.inbox
            .acknowledge(session.network_agent_id, &event_id)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)
    }

    /// 用本机 Bridge 的同步服务解析、验签这一批，只截下结果，不落它的投影。
    async fn changes(
        &self,
        session: &NetworkAgentSession,
        batch: &MatrixSyncBatch,
    ) -> Result<Vec<NetworkAgentInboxChange>, NetworkGatewayFailure> {
        let captured = Arc::new(projection::CapturedProjection::default());
        let sync = MessageSyncService::new(MessageSyncDependencies {
            authenticator: Arc::new(AgentInstanceMessageAuthenticator::new(
                AgentInstanceMessageAuthenticatorDependencies {
                    verification: Arc::new(projection::InstanceVerification::new(
                        self.verification.clone(),
                    )),
                    signatures: self.signatures.clone(),
                },
            )),
            projections: captured.clone(),
            submissions: Arc::new(projection::NoSubmissions),
        });
        sync.process(batch)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        Ok(projection::inbox_changes(captured.take(), session.agent_id))
    }
}

impl NetworkAgentMessaging for NetworkGateway {
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        wait: Duration,
        limit: u16,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>> {
        Box::pin(self.wait_internal(token, wait, limit))
    }

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>> {
        Box::pin(self.acknowledge_internal(token, event_id))
    }
}

fn messages(page: NetworkAgentInboxPage) -> NetworkAgentMessages {
    NetworkAgentMessages {
        messages: page
            .entries
            .into_iter()
            .map(|entry| entry.preview)
            .collect(),
        pending: page.pending,
        dropped: page.dropped,
    }
}

/// 每个网络 Agent 同时只有一次长轮询：新来的登记后通知旧的，旧的立刻空手返回。
#[derive(Default)]
struct LongPolls {
    next: AtomicU64,
    active: Mutex<HashMap<NetworkAgentId, (u64, Arc<Notify>)>>,
}

impl LongPolls {
    fn begin(&self, agent: NetworkAgentId) -> PollTicket<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let notify = Arc::new(Notify::new());
        let previous = self
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(agent, (id, notify.clone()));
        if let Some((_, previous)) = previous {
            // 旧的哪怕此刻不在等，也会在下一次等之前收到这个通知。
            previous.notify_one();
        }
        PollTicket {
            polls: self,
            agent,
            id,
            notify,
        }
    }
}

struct PollTicket<'a> {
    polls: &'a LongPolls,
    agent: NetworkAgentId,
    id: u64,
    notify: Arc<Notify>,
}

impl PollTicket<'_> {
    async fn superseded(&self) {
        self.notify.notified().await;
    }
}

impl Drop for PollTicket<'_> {
    fn drop(&mut self) {
        let mut active = self
            .polls
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if active
            .get(&self.agent)
            .is_some_and(|(current, _)| *current == self.id)
        {
            active.remove(&self.agent);
        }
    }
}
