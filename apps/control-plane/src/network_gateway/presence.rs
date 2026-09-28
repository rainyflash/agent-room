//! 网络 Agent 的在线状态：用本机 Bridge 同一个状态发布服务写房间状态。长轮询开始等待时宣布
//! “等待消息”，等待期间跟着连接租约续；一次长轮询结束后 10 秒内没开始下一次就清除等待。
//! 停止轮询后按租约转为离线；停用时先发“已离线”。
//! 每个网络 Agent 一个发布服务，续租节奏记在进程里：控制面重启后第一次发布就是新的开始。

use std::{collections::HashMap, sync::Arc};

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        Clock, MatrixEventId, MatrixResult, MatrixRoomId, MatrixStateEvent,
        NetworkAgentMatrixGateway, PortFuture, SecretValue,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    ports::{AgentStatusStatePublisher, StatusEventIdentifierFactory},
    status::{
        AgentStatusIntent, AgentStatusLeasePolicy, AgentStatusPublicationDependencies,
        AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState, WAIT_IDLE_TIMEOUT,
    },
};
use agent_room_domain::{
    agent_status::AgentStatusVisibility,
    ids::NetworkAgentId,
    time::{DurationMillis, UtcMillis},
};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::speaking::SessionSigner;

/// 与本机 Bridge 相同：租约 5 分钟，约 2 分钟续一次。
const LEASE_LIFETIME_MILLIS: u64 = 300_000;
const RENEWAL_INTERVAL_MILLIS: u64 = 120_000;
const RENEWAL_JITTER_MILLIS: u64 = 15_000;

/// 以 Agent 自己的 Matrix 会话写状态事件。
struct SessionStatePublisher {
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    access_token: SecretValue,
}

impl AgentStatusStatePublisher for SessionStatePublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        self.matrix
            .send_state_event(&self.access_token, room_id, event)
    }
}

struct UuidV7Identifiers;

impl StatusEventIdentifierFactory for UuidV7Identifiers {
    fn event_id(&self) -> Uuid {
        Uuid::now_v7()
    }

    fn correlation_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

struct AgentPresence {
    service: AgentStatusPublicationService,
    /// 每宣布一次在等待就加一。长轮询结束时记下它，过了 [`WAIT_IDLE_TIMEOUT`] 还没变，就是没再等。
    wait_generation: u64,
    /// 最近一次宣布在等待的时间，清除等待时当作最近读取时间。
    last_waited_at: Option<UtcMillis>,
}

type SharedPresence = Arc<Mutex<AgentPresence>>;

#[derive(Default)]
pub(super) struct Presence {
    agents: Mutex<HashMap<NetworkAgentId, SharedPresence>>,
}

impl Presence {
    /// 在这个 Agent 所在的每个房间按需发布状态；失败只记日志，不影响收发。
    pub(super) async fn publish(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
        intent: &AgentStatusIntent,
    ) {
        let Some(agent) = self.agent(matrix, clock, session).await else {
            return;
        };
        publish_in_rooms(&mut agent.lock().await.service, session, intent).await;
    }

    /// 宣布这个 Agent 正在长轮询里等消息，返回这次宣布的编号。重复宣布由发布服务去重，
    /// 只有开始等待和续租时才真的写房间状态。
    async fn announce_waiting(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Option<u64> {
        let agent = self.agent(matrix, clock, session).await?;
        let mut agent = agent.lock().await;
        let now = clock.now();
        agent.wait_generation += 1;
        agent.last_waited_at = Some(now);
        let intent = AgentStatusIntent::new(HostAgentState::Available, None)
            .with_last_polled_at(Some(now))
            .with_waiting(true);
        publish_in_rooms(&mut agent.service, session, &intent).await;
        Some(agent.wait_generation)
    }

    /// 编号为 `generation` 的宣布之后没再宣布过，就是没开始新的等待：清除等待。已停用的不管。
    async fn clear_wait_if_idle(&self, session: &NetworkAgentSession, generation: u64) {
        let Some(agent) = self
            .agents
            .lock()
            .await
            .get(&session.network_agent_id)
            .cloned()
        else {
            return;
        };
        let mut agent = agent.lock().await;
        if agent.wait_generation != generation {
            return;
        }
        let intent = AgentStatusIntent::new(HostAgentState::Available, None)
            .with_last_polled_at(agent.last_waited_at)
            .with_waiting(false);
        publish_in_rooms(&mut agent.service, session, &intent).await;
    }

    /// 停用后不再续租。
    pub(super) async fn forget(&self, id: NetworkAgentId) {
        self.agents.lock().await.remove(&id);
    }

    async fn agent(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Option<SharedPresence> {
        let agent = self.agent_for(matrix, clock, session).await;
        if agent.is_none() {
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                "网络 Agent 的在线状态服务建不起来，这次不发布"
            );
        }
        agent
    }

    async fn agent_for(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Option<SharedPresence> {
        let mut agents = self.agents.lock().await;
        if let Some(agent) = agents.get(&session.network_agent_id) {
            return Some(agent.clone());
        }
        let identity = BridgeAgentIdentity::new(
            session.agent_id,
            session.display_name.clone(),
            session.agent_matrix_user_id.clone(),
            session.agent_instance_id,
        )
        .ok()?;
        let policy = AgentStatusLeasePolicy::new(
            DurationMillis::new(LEASE_LIFETIME_MILLIS).ok()?,
            DurationMillis::new(RENEWAL_INTERVAL_MILLIS).ok()?,
            DurationMillis::new(RENEWAL_JITTER_MILLIS).ok()?,
        )
        .ok()?;
        let agent = Arc::new(Mutex::new(AgentPresence {
            service: AgentStatusPublicationService::new(
                AgentStatusPublicationDependencies {
                    identity,
                    signer: Arc::new(SessionSigner::from_seed(&session.instance_signing_seed)?),
                    publisher: Arc::new(SessionStatePublisher {
                        matrix: matrix.clone(),
                        access_token: session.matrix_access_token.clone(),
                    }),
                    identifiers: Arc::new(UuidV7Identifiers),
                    clock: clock.clone(),
                },
                policy,
            ),
            wait_generation: 0,
            last_waited_at: None,
        }));
        agents.insert(session.network_agent_id, agent.clone());
        Some(agent)
    }
}

async fn publish_in_rooms(
    service: &mut AgentStatusPublicationService,
    session: &NetworkAgentSession,
    intent: &AgentStatusIntent,
) {
    for room in &session.rooms {
        let Ok(room_id) = MatrixRoomId::new(room.matrix_room_id.as_str()) else {
            continue;
        };
        let target = AgentStatusRoomTarget::new(room_id, AgentStatusVisibility::Coarse);
        if let Err(failure) = service.publish_if_due(&target, intent, entropy()).await {
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                failure = ?failure.kind(),
                "网络 Agent 的在线状态没发出去"
            );
        }
    }
}

/// 一次长轮询里的“等待消息”。长轮询不管怎么结束（返回、出错、连接被断开）都会丢掉它，
/// 这时交给看门狗：[`WAIT_IDLE_TIMEOUT`] 内没开始下一次等待，就清除等待。
pub(super) struct WaitAnnouncement {
    presence: Arc<Presence>,
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    clock: Arc<dyn Clock>,
    session: NetworkAgentSession,
    generation: Option<u64>,
}

impl WaitAnnouncement {
    pub(super) fn new(
        presence: &Arc<Presence>,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Self {
        Self {
            presence: presence.clone(),
            matrix: matrix.clone(),
            clock: clock.clone(),
            session: session.clone(),
            generation: None,
        }
    }

    /// 要等了：告诉房间里的人它在等消息。
    pub(super) async fn announce(&mut self) {
        if let Some(generation) = self
            .presence
            .announce_waiting(&self.matrix, &self.clock, &self.session)
            .await
        {
            self.generation = Some(generation);
        }
    }
}

impl Drop for WaitAnnouncement {
    fn drop(&mut self) {
        let Some(generation) = self.generation else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let presence = self.presence.clone();
        let session = self.session.clone();
        runtime.spawn(async move {
            tokio::time::sleep(WAIT_IDLE_TIMEOUT).await;
            presence.clear_wait_if_idle(&session, generation).await;
        });
    }
}

/// 续租时间的随机抖动，免得大家同时续租。
fn entropy() -> u64 {
    let bytes = Uuid::new_v4().into_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
