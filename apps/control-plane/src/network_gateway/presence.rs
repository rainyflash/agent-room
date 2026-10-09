//! 网络 Agent 在不在线、在不在等消息（`specs/agent-liveness/design.md`）。
//!
//! 服务器开着 Matrix 在线状态时，每个房间写一张名片（房间里已经有一样的就不写），之后不再写
//! 房间状态；在不在线报 Matrix 的在线状态。等消息时同步带 `online`，开始等就马上 `PUT` 一次；
//! 进房间或等完以后的 5 分钟里每 20 秒 `PUT` 一次，等完一分钟内报在线、之后报离开（网关只在
//! 等消息时替它同步，Synapse 30 秒没动静就判离线）；5 分钟后不再报，Synapse 改成离线。停用时
//! 报离线。
//!
//! 服务器没开在线状态时照旧写租约：长轮询开始等待时宣布“等待消息”，等待期间跟着连接租约续；
//! 一次长轮询结束后 10 秒内没开始下一次就清除等待；停止轮询后按租约转为离线；停用时先发“已离线”。
//!
//! 每个网络 Agent 一份，记在进程里：控制面重启后第一次用到就是新的开始。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::Duration,
};

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        Clock, MatrixEventId, MatrixEventType, MatrixResult, MatrixRoomId, MatrixStateEvent,
        MatrixStateKey, MatrixUserId, NetworkAgentMatrixGateway, PortFuture, SecretValue,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    ports::{AgentStatusStatePublisher, StatusEventIdentifierFactory},
    presence::AGENT_STATUS_EVENT_TYPE,
    presence_support::{OwnPresence, PresenceSupport},
    status::{
        AgentStatusIntent, AgentStatusLeasePolicy, AgentStatusPublicationDependencies,
        AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState, WAIT_IDLE_TIMEOUT,
    },
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState::{self, Offline, Online, Unavailable},
    agent_status::AgentStatusVisibility,
    ids::NetworkAgentId,
    time::{DurationMillis, UtcMillis},
};
use tokio::{sync::Mutex, time::Instant};
use uuid::Uuid;

use super::speaking::SessionSigner;

/// 与本机 Bridge 相同：租约 5 分钟，约 2 分钟续一次。名片只用到它的名义租约。
const LEASE_LIFETIME_MILLIS: u64 = 300_000;
const RENEWAL_INTERVAL_MILLIS: u64 = 120_000;
const RENEWAL_JITTER_MILLIS: u64 = 15_000;
/// 等完以后还报“在线”（在等消息）的时间：处理一条消息就在在线和离开之间来回跳不好看。
const WAITING_DEBOUNCE: Duration = Duration::from_mins(1);
/// 进房间、等完以后报“离开”（连着、没在等）的时间，和原来“最后一次等消息之后再在线 5 分钟”
/// 一样。之后不再报，Synapse 约 30 秒后改成离线。
const CONNECTED_FOR: Duration = Duration::from_mins(5);
/// 不同步的时候隔多久报一次：Synapse 30 秒没见到同步或者报，就判离线。
const HEARTBEAT: Duration = Duration::from_secs(20);

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

/// 以 Agent 自己的 Matrix 会话报、读它的在线状态。
#[derive(Clone)]
struct SessionPresence {
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    access_token: SecretValue,
    user_id: MatrixUserId,
}

impl SessionPresence {
    /// 报一次。报不出去（多半是限速）也不要紧：等消息的同步会带上，过一会儿还会再报。
    async fn report_quietly(&self, presence: MatrixPresenceState) {
        if let Err(failure) = self.report(presence).await {
            tracing::debug!(
                matrix.user_id = %self.user_id.as_str(),
                failure = ?failure.kind(),
                presence = ?presence,
                "网络 Agent 的 Matrix 在线状态没报出去"
            );
        }
    }
}

impl OwnPresence for SessionPresence {
    fn report(&self, presence: MatrixPresenceState) -> PortFuture<'_, MatrixResult<()>> {
        self.matrix
            .report_presence(&self.access_token, &self.user_id, presence)
    }

    fn read(&self) -> PortFuture<'_, MatrixResult<MatrixPresenceState>> {
        self.matrix.own_presence(&self.access_token, &self.user_id)
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
    own: SessionPresence,
    /// 服务器开没开在线状态。没开就照旧写租约。
    support: PresenceSupport,
    /// 写租约时：每宣布一次在等待就加一。长轮询结束时记下它，过了 [`WAIT_IDLE_TIMEOUT`] 还没变，
    /// 就是没再等。
    wait_generation: u64,
    /// 写租约时：最近一次宣布在等待的时间，清除等待时当作最近读取时间。
    last_waited_at: Option<UtcMillis>,
    reporting: Reporting,
}

/// 报在线状态时记着的。
#[derive(Default)]
struct Reporting {
    /// 已经有这个实例一样名片的房间（读到的或者自己写的）。
    cards: HashSet<MatrixRoomId>,
    /// 最近一次在等消息：长轮询里等着，或者刚等完。
    waited_at: Option<Instant>,
    /// 报“连着”报到什么时候。
    connected_until: Option<Instant>,
    /// 上一次报出去（`PUT` 或同步带）的。
    reported: Option<MatrixPresenceState>,
    heartbeat: bool,
}

impl Reporting {
    /// 此刻该报的：在等、或者等完不到一分钟报在线；进房间或等完 5 分钟内报离开；再往后不报。
    fn wanted(&self, now: Instant) -> MatrixPresenceState {
        if self
            .waited_at
            .is_some_and(|at| now.duration_since(at) < WAITING_DEBOUNCE)
        {
            Online
        } else if self.connected_until.is_some_and(|until| now < until) {
            Unavailable
        } else {
            Offline
        }
    }

    fn waited(&mut self, now: Instant) {
        self.waited_at = Some(now);
        self.connected(now);
    }

    fn connected(&mut self, now: Instant) {
        let until = now + CONNECTED_FOR;
        self.connected_until = Some(self.connected_until.map_or(until, |at| at.max(until)));
    }
}

type SharedPresence = Arc<Mutex<AgentPresence>>;

struct Entry {
    /// 建它时的访问令牌：换过设备（令牌变了）就重新建，免得一直拿旧令牌写。放在锁外面，
    /// 比较时不用等这个 Agent 手上的网络请求。
    access_token: SecretValue,
    presence: SharedPresence,
}

#[derive(Default)]
pub(super) struct Presence {
    agents: Mutex<HashMap<NetworkAgentId, Entry>>,
}

impl Presence {
    /// 进了房间：写名片、报“连着”，之后 5 分钟里一直报着。服务器没开在线状态时写一条“在线”的
    /// 租约。失败只记日志，不影响收发。
    pub(super) async fn connected(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) {
        let Some(shared) = self.agent(matrix, clock, session).await else {
            return;
        };
        let mut agent = shared.lock().await;
        if agent.uses_lease(Unavailable).await {
            let intent = AgentStatusIntent::new(HostAgentState::Available, None);
            publish_in_rooms(&mut agent.service, session, &intent).await;
            return;
        }
        agent.reporting.connected(Instant::now());
        agent.place_cards(session).await;
        agent.report_change(&shared).await;
    }

    /// 停用、离开房间之前：报离线，不再报。服务器没开在线状态时写一条“已离线”的租约（离开
    /// 之后就写不了房间状态了）。这个进程还没探过开没开（比如停用以后的定时清理）就两样都做：
    /// 读的一边不管看租约还是看在线状态，都看到它离线。
    pub(super) async fn disconnected(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) {
        let Some(agent) = self.agent(matrix, clock, session).await else {
            return;
        };
        let mut agent = agent.lock().await;
        if agent.support != PresenceSupport::Enabled {
            let intent = AgentStatusIntent::new(HostAgentState::Disconnected, None);
            publish_in_rooms(&mut agent.service, session, &intent).await;
        }
        if agent.support == PresenceSupport::Disabled {
            return;
        }
        agent.reporting.waited_at = None;
        agent.reporting.connected_until = None;
        agent.reporting.reported = Some(Offline);
        agent.own.report_quietly(Offline).await;
    }

    /// 这次等消息的同步顺带报的：此刻该报的；`Offline` 是不报。服务器没开在线状态时不报。
    pub(super) async fn sync_presence(&self, session: &NetworkAgentSession) -> MatrixPresenceState {
        let Some(agent) = self.existing(session.network_agent_id).await else {
            return Offline;
        };
        let mut agent = agent.lock().await;
        if agent.support == PresenceSupport::Disabled {
            return Offline;
        }
        let wanted = agent.reporting.wanted(Instant::now());
        if wanted != Offline {
            agent.reporting.reported = Some(wanted);
        }
        wanted
    }

    /// 宣布这个 Agent 正在长轮询里等消息，返回这次宣布的编号。开着在线状态时，写名片、马上报
    /// 在线（已经报过就不再报）；写租约时由发布服务去重，只有开始等待和续租时才真的写房间状态。
    async fn announce_waiting(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Option<u64> {
        let shared = self.agent(matrix, clock, session).await?;
        let mut agent = shared.lock().await;
        agent.wait_generation += 1;
        let generation = agent.wait_generation;
        if agent.uses_lease(Online).await {
            let now = clock.now();
            agent.last_waited_at = Some(now);
            let intent = AgentStatusIntent::new(HostAgentState::Available, None)
                .with_last_polled_at(Some(now))
                .with_waiting(true);
            publish_in_rooms(&mut agent.service, session, &intent).await;
            return Some(generation);
        }
        agent.reporting.waited(Instant::now());
        agent.place_cards(session).await;
        agent.report_change(&shared).await;
        Some(generation)
    }

    /// 一次长轮询结束了。开着在线状态时记下这一刻：一分钟内还报在线，5 分钟内报离开。写租约时
    /// 编号为 `generation` 的宣布之后 [`WAIT_IDLE_TIMEOUT`] 内没再宣布，就是没开始新的等待：
    /// 清除等待。已停用的不管。
    async fn wait_ended(&self, session: &NetworkAgentSession, generation: u64) {
        let Some(agent) = self.existing(session.network_agent_id).await else {
            return;
        };
        {
            let mut presence = agent.lock().await;
            if presence.support != PresenceSupport::Disabled {
                presence.reporting.waited(Instant::now());
                return;
            }
        }
        tokio::time::sleep(WAIT_IDLE_TIMEOUT).await;
        let mut agent = agent.lock().await;
        if agent.wait_generation != generation {
            return;
        }
        let intent = AgentStatusIntent::new(HostAgentState::Available, None)
            .with_last_polled_at(agent.last_waited_at)
            .with_waiting(false);
        publish_in_rooms(&mut agent.service, session, &intent).await;
    }

    /// 停用后不再续租、不再报。
    pub(super) async fn forget(&self, id: NetworkAgentId) {
        self.agents.lock().await.remove(&id);
    }

    async fn existing(&self, id: NetworkAgentId) -> Option<SharedPresence> {
        self.agents
            .lock()
            .await
            .get(&id)
            .map(|entry| entry.presence.clone())
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
        if let Some(entry) = agents.get(&session.network_agent_id)
            && entry.access_token == session.matrix_access_token
        {
            return Some(entry.presence.clone());
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
            own: SessionPresence {
                matrix: matrix.clone(),
                access_token: session.matrix_access_token.clone(),
                user_id: MatrixUserId::new(session.agent_matrix_user_id.clone()).ok()?,
            },
            support: PresenceSupport::default(),
            wait_generation: 0,
            last_waited_at: None,
            reporting: Reporting::default(),
        }));
        agents.insert(
            session.network_agent_id,
            Entry {
                access_token: session.matrix_access_token.clone(),
                presence: agent.clone(),
            },
        );
        Some(agent)
    }
}

impl AgentPresence {
    /// 还不知道服务器开没开在线状态就探一次（报 `wanted`、再读回自己的），没开就写租约。
    /// 探不出来（限速、超时）这次先不写也不报，下次再探。
    async fn uses_lease(&mut self, wanted: MatrixPresenceState) -> bool {
        if self.support == PresenceSupport::Disabled {
            return true;
        }
        let unknown = matches!(self.support, PresenceSupport::Unknown { .. });
        if let Some(failure) = self.support.probe(&self.own, wanted).await {
            tracing::debug!(
                matrix.user_id = %self.own.user_id.as_str(),
                failure = ?failure.kind(),
                operation = ?failure.operation(),
                "没确认服务器开没开 Matrix 在线状态，下次再确认"
            );
        }
        // 探的时候已经报过了，不用马上再报（10 秒内再报会被限速）。
        if unknown && self.support == PresenceSupport::Enabled {
            self.reporting.reported = Some(wanted);
        }
        if self.support == PresenceSupport::Disabled {
            tracing::warn!(
                matrix.user_id = %self.own.user_id.as_str(),
                "服务器没开 Matrix 在线状态，网络 Agent 照旧写租约"
            );
            return true;
        }
        false
    }

    /// 在这个 Agent 所在的每个房间放一张名片：房间里已经有一样的就不写。确认服务器开着在线
    /// 状态以前不写，没开的话写了名片，别人只会看到它离线。
    async fn place_cards(&mut self, session: &NetworkAgentSession) {
        if self.support != PresenceSupport::Enabled {
            return;
        }
        let Ok(event_type) = MatrixEventType::new(AGENT_STATUS_EVENT_TYPE) else {
            return;
        };
        let state_key = MatrixStateKey::from_agent_instance_id(session.agent_instance_id);
        for room in &session.rooms {
            let Ok(room_id) = MatrixRoomId::new(room.matrix_room_id.as_str()) else {
                continue;
            };
            if self.reporting.cards.contains(&room_id) {
                continue;
            }
            // 读不出来只是多写一张，名片照样是对的。
            let existing = self
                .own
                .matrix
                .state_event(&self.own.access_token, &room_id, &event_type, &state_key)
                .await
                .ok()
                .flatten();
            if !existing.is_some_and(|content| self.service.is_current_card(&content)) {
                let target =
                    AgentStatusRoomTarget::new(room_id.clone(), AgentStatusVisibility::Coarse);
                if let Err(failure) = self.service.publish_card(&target).await {
                    tracing::warn!(
                        network_agent.id = %session.network_agent_id,
                        failure = ?failure.kind(),
                        "网络 Agent 的名片没写进去，下次再写"
                    );
                    continue;
                }
            }
            self.reporting.cards.insert(room_id);
        }
    }

    /// 此刻该报的和上次报的不一样就马上报，并且接着每 20 秒报一次。
    async fn report_change(&mut self, shared: &SharedPresence) {
        let wanted = self.reporting.wanted(Instant::now());
        if wanted == Offline {
            return;
        }
        if self.reporting.reported != Some(wanted) {
            self.reporting.reported = Some(wanted);
            self.own.report_quietly(wanted).await;
        }
        if !self.reporting.heartbeat {
            self.reporting.heartbeat = true;
            tokio::spawn(heartbeat(Arc::downgrade(shared)));
        }
    }
}

/// 网关只在等消息时替它同步，其余时候每 20 秒报一次，免得 Synapse 30 秒没动静就判它离线。
/// 该报的成了“不报”（5 分钟到了），或者停用了，就停下。
async fn heartbeat(agent: Weak<Mutex<AgentPresence>>) {
    loop {
        tokio::time::sleep(HEARTBEAT).await;
        let Some(agent) = agent.upgrade() else {
            return;
        };
        let mut presence = agent.lock().await;
        let wanted = presence.reporting.wanted(Instant::now());
        if wanted == Offline || presence.support == PresenceSupport::Disabled {
            presence.reporting.heartbeat = false;
            return;
        }
        presence.reporting.reported = Some(wanted);
        let own = presence.own.clone();
        drop(presence);
        drop(agent);
        own.report_quietly(wanted).await;
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

/// 一次长轮询里的“等待消息”。长轮询不管怎么结束（返回、出错、连接被断开）都会丢掉它，这时
/// 记下等完的那一刻；写租约时交给看门狗：[`WAIT_IDLE_TIMEOUT`] 内没开始下一次等待，就清除等待。
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
            presence.wait_ended(&session, generation).await;
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

#[cfg(test)]
mod real_dependency_tests;
