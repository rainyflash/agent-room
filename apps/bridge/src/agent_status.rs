use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use agent_room_application::ports::{
    MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixOperation,
    MatrixResult, MatrixRoomId, MatrixStateKey, PortFuture,
};
use agent_room_bridge_core::{
    presence::AGENT_STATUS_EVENT_TYPE,
    presence_support::{OwnPresence, PresenceSupport},
    status::{
        AgentStatusIntent, AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState,
        StatusPublicationOutcome, StatusPublicationResult, WAIT_IDLE_TIMEOUT,
    },
};
use agent_room_domain::{agent_lifecycle::MatrixPresenceState, time::UtcMillis};
use tokio::{sync::Mutex, time::Instant};
use uuid::Uuid;

/// 等完以后还报“在线”（在等消息）的时间：处理一条消息就在在线和离开之间来回跳不好看
/// （`specs/agent-liveness/design.md`）。
const WAITING_DEBOUNCE: Duration = Duration::from_mins(1);

pub(crate) struct AgentStatusPublicationHandle {
    target: AgentStatusRoomTarget,
    /// 报 Matrix 在线状态用。没有就一直照旧写租约。
    matrix: Option<Arc<dyn MatrixGateway>>,
    state: Mutex<AgentStatusPublicationState>,
}

struct AgentStatusPublicationState {
    service: AgentStatusPublicationService,
    intent: AgentStatusIntent,
    /// 最近一次空手而归的 `WaitInbox`。等待的进程被杀或取消后就不会再来，看门狗据此清除等待。
    last_wait_at: Option<Instant>,
    watchdog_running: bool,
    liveness: Liveness,
}

/// 在不在线怎么告诉别人。
enum Liveness {
    /// 照旧写租约：没有 Matrix 网关，或者服务器没开在线状态。
    Lease,
    /// 进房间时写一张名片，在不在线、在不在等消息报 Matrix 的在线状态。
    Presence(PresenceReporting),
}

#[derive(Default)]
struct PresenceReporting {
    /// 这次连上以后，房间里已经有一样的名片（看到的或者自己写的）。
    card_in_place: bool,
    /// 服务器开没开在线状态。确认开着才写名片。
    support: PresenceSupport,
    /// 最近一次在等消息，哪个房间都算：Matrix 的在线状态按 Agent 算。
    waited_at: Option<Instant>,
    /// 上一次报出去（`PUT` 或同步带）的在线状态。
    reported: Option<MatrixPresenceState>,
}

impl PresenceReporting {
    /// 此刻该报的：在等消息、或者刚等完不到一分钟报在线，别的时候报离开。
    fn wanted(&self, now: Instant) -> MatrixPresenceState {
        if self
            .waited_at
            .is_some_and(|at| now.duration_since(at) < WAITING_DEBOUNCE)
        {
            MatrixPresenceState::Online
        } else {
            MatrixPresenceState::Unavailable
        }
    }
}

impl AgentStatusPublicationHandle {
    /// 一直写租约。产品里都用 [`Self::with_presence`]，租约只在服务器没开在线状态时退回去用。
    #[cfg(test)]
    pub(crate) fn new(
        service: AgentStatusPublicationService,
        target: AgentStatusRoomTarget,
        initial_state: HostAgentState,
    ) -> Self {
        Self::build(service, target, initial_state, None)
    }

    /// 写名片、报 Matrix 在线状态；读回发现服务器没开在线状态，就改回写租约。
    pub(crate) fn with_presence(
        service: AgentStatusPublicationService,
        target: AgentStatusRoomTarget,
        initial_state: HostAgentState,
        matrix: Arc<dyn MatrixGateway>,
    ) -> Self {
        Self::build(service, target, initial_state, Some(matrix))
    }

    fn build(
        service: AgentStatusPublicationService,
        target: AgentStatusRoomTarget,
        initial_state: HostAgentState,
        matrix: Option<Arc<dyn MatrixGateway>>,
    ) -> Self {
        let liveness = if matrix.is_some() {
            Liveness::Presence(PresenceReporting::default())
        } else {
            Liveness::Lease
        };
        Self {
            target,
            matrix,
            state: Mutex::new(AgentStatusPublicationState {
                service,
                intent: AgentStatusIntent::new(initial_state, None),
                last_wait_at: None,
                watchdog_running: false,
                liveness,
            }),
        }
    }

    /// 这次同步顺带报的在线状态。写租约时照旧报在线，和 Matrix SDK 不设时一样。
    pub(crate) async fn sync_presence(&self) -> MatrixPresenceState {
        let mut state = self.state.lock().await;
        match &mut state.liveness {
            Liveness::Lease => MatrixPresenceState::Online,
            Liveness::Presence(reporting) => {
                let wanted = reporting.wanted(Instant::now());
                reporting.reported = Some(wanted);
                wanted
            }
        }
    }

    /// 旧版客户端（MCP 和命令行）还会发 `PublishStatus`：照收不发，交回现在的名片或租约。
    /// Agent 自己报的工作状态去掉了（`specs/agent-liveness/design.md` 第 3 步），写租约的也只到点
    /// 续租，不换状态。
    pub(crate) async fn acknowledge(&self) -> StatusPublicationResult<StatusPublicationOutcome> {
        let mut state = self.state.lock().await;
        let state = &mut *state;
        if matches!(state.liveness, Liveness::Presence(_)) {
            return state.service.card_outcome();
        }
        state
            .service
            .publish_if_due(&self.target, &state.intent, status_entropy())
            .await
    }

    /// 正常退出，同步已经停了。写名片的报一次离线，别人马上看到，不用等 Synapse 那 30 秒；
    /// 写租约的发一条离线。
    pub(crate) async fn disconnect(&self) -> StatusPublicationResult<()> {
        let presence = matches!(self.state.lock().await.liveness, Liveness::Presence(_));
        if presence {
            self.report(MatrixPresenceState::Offline).await;
            return Ok(());
        }
        let mut state = self.state.lock().await;
        let intent = AgentStatusIntent::new(HostAgentState::Disconnected, None);
        state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await?;
        state.intent = intent;
        Ok(())
    }

    /// 每次同步之后调用。写名片的：还没确认服务器开着在线状态就确认一次，确认了才写名片
    /// （没开的话写了名片，别人只会看到它离线）；房间里已经有一样的就不写，之后也不再写。
    /// 写租约的：到点续租。
    pub(crate) async fn renew(&self) -> StatusPublicationResult<()> {
        let mut state = self.state.lock().await;
        let state = &mut *state;
        if let Liveness::Presence(reporting) = &mut state.liveness
            && let Some(matrix) = &self.matrix
        {
            let wanted = reporting.wanted(Instant::now());
            let unknown = matches!(reporting.support, PresenceSupport::Unknown { .. });
            if let Some(failure) = reporting
                .support
                .probe(&OwnMatrixPresence(matrix.as_ref()), wanted)
                .await
            {
                tracing::debug!(
                    failure_kind = ?failure.kind(),
                    operation = ?failure.operation(),
                    "没确认服务器开没开 Matrix 在线状态，下次同步后再确认"
                );
            }
            // 探的时候已经报过了：紧接着开始等消息时不用再报（10 秒内再报会被限速）。
            if unknown && reporting.support == PresenceSupport::Enabled {
                reporting.reported = Some(wanted);
            }
            if reporting.support == PresenceSupport::Disabled {
                tracing::warn!("服务器没开 Matrix 在线状态，改回写租约");
                state.liveness = Liveness::Lease;
            }
        }
        match &mut state.liveness {
            Liveness::Presence(reporting)
                if reporting.support != PresenceSupport::Enabled || reporting.card_in_place =>
            {
                Ok(())
            }
            Liveness::Presence(reporting) => {
                let Some(matrix) = &self.matrix else {
                    return Ok(());
                };
                // 问服务器，不看本机投影：投影把同一个 Agent 离线的几个实例并成一张，刚连上、还没
                // 拿到自己的在线状态时，自己的名片可能就被并掉了。读不出来这次先不写，下次同步后
                // 再看：多写一张就多一条永久记录。
                match card_in_room(matrix.as_ref(), &self.target, &state.service).await {
                    Ok(true) => {}
                    Ok(false) => {
                        state.service.publish_card(&self.target).await?;
                    }
                    Err(failure) => {
                        tracing::debug!(
                            failure_kind = ?failure.kind(),
                            "没读到房间里的名片，下次同步后再看"
                        );
                        return Ok(());
                    }
                }
                reporting.card_in_place = true;
                Ok(())
            }
            Liveness::Lease => {
                let intent = state.intent.clone();
                state
                    .service
                    .publish_if_due(&self.target, &intent, status_entropy())
                    .await
                    .map(|_| ())
            }
        }
    }

    /// 宿主收件后记下读取时间与是否仍在等待。写名片的：哪个房间里等着都算在等，刚开始等就
    /// 马上报在线。写租约的：只认大厅里的，开始、结束等待各发一次状态；等待期间不按轮询重发，
    /// 由看门狗在 [`WAIT_IDLE_TIMEOUT`] 内没再等时清除。
    pub(crate) async fn note_inbox_wait(
        self: &Arc<Self>,
        room_id: &MatrixRoomId,
        at: UtcMillis,
        waiting: bool,
    ) -> StatusPublicationResult<()> {
        let mut state = self.state.lock().await;
        let mut report_online = false;
        if let Liveness::Presence(reporting) = &mut state.liveness
            && waiting
        {
            reporting.waited_at = Some(Instant::now());
            if reporting.reported != Some(MatrixPresenceState::Online) {
                reporting.reported = Some(MatrixPresenceState::Online);
                report_online = true;
            }
        }
        // A private-room fetch cannot advertise reception in the public lobby.
        if room_id == self.target.room_id() {
            state.intent = state
                .intent
                .clone()
                .with_last_polled_at(Some(at))
                .with_waiting(waiting);
            state.last_wait_at = waiting.then(Instant::now);
            if waiting && !state.watchdog_running {
                state.watchdog_running = true;
                tokio::spawn(watch_abandoned_wait(Arc::downgrade(self)));
            }
        }
        if matches!(state.liveness, Liveness::Presence(_)) || room_id != self.target.room_id() {
            // 放开锁再报：报得慢也不挡同步。
            drop(state);
            if report_online {
                self.report(MatrixPresenceState::Online).await;
            }
            return Ok(());
        }
        let intent = state.intent.clone();
        state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await?;
        Ok(())
    }

    /// 马上报一次在线状态。报不出去（多半是限速）也不要紧，下一次同步会带上。
    async fn report(&self, presence: MatrixPresenceState) {
        let Some(matrix) = &self.matrix else {
            return;
        };
        if let Err(failure) = matrix.report_presence(presence).await {
            tracing::debug!(
                failure_kind = ?failure.kind(),
                presence = ?presence,
                "没报出去 Matrix 在线状态，下一次同步会带上"
            );
        }
    }
}

/// 用这个 Agent 自己的 Matrix 会话报、读它的在线状态。
struct OwnMatrixPresence<'a>(&'a dyn MatrixGateway);

impl OwnPresence for OwnMatrixPresence<'_> {
    fn report(&self, presence: MatrixPresenceState) -> PortFuture<'_, MatrixResult<()>> {
        self.0.report_presence(presence)
    }

    fn read(&self) -> PortFuture<'_, MatrixResult<MatrixPresenceState>> {
        Box::pin(async move {
            self.0
                .user_presence(self.0.metadata().user_id())
                .await
                .map(|presence| presence.state())
        })
    }
}

/// 房间里这个实例的那条状态就是它此刻的名片：内容里的身份都没变，用不着再写。
async fn card_in_room(
    matrix: &dyn MatrixGateway,
    target: &AgentStatusRoomTarget,
    service: &AgentStatusPublicationService,
) -> MatrixResult<bool> {
    let event_type = MatrixEventType::new(AGENT_STATUS_EVENT_TYPE).map_err(|_| {
        MatrixFailure::new(
            MatrixOperation::ReadRoomState,
            MatrixFailureKind::InvalidConfiguration,
        )
    })?;
    let state_key = MatrixStateKey::from_agent_instance_id(service.identity().agent_instance_id());
    let content = matrix
        .state_event(target.room_id(), &event_type, &state_key)
        .await?;
    Ok(content.is_some_and(|content| service.is_current_card(&content)))
}

/// 等待的进程被杀、被取消时不会告诉 Bridge，只是不再来 `WaitInbox`。隔了 [`WAIT_IDLE_TIMEOUT`]
/// 还没来就清除等待，不让“持续等待消息”一直挂到 `waitingUntil`。写名片时不用发什么：在线
/// 状态按最近一次等消息算，过了一分钟自己变“离开”。
async fn watch_abandoned_wait(handle: Weak<AgentStatusPublicationHandle>) {
    loop {
        let Some(handle) = handle.upgrade() else {
            return;
        };
        let mut state = handle.state.lock().await;
        let Some(last_wait_at) = state.last_wait_at.filter(|_| state.intent.waiting()) else {
            state.watchdog_running = false;
            return;
        };
        let deadline = last_wait_at + WAIT_IDLE_TIMEOUT;
        if Instant::now() < deadline {
            drop(state);
            drop(handle);
            tokio::time::sleep_until(deadline).await;
            continue;
        }
        state.intent = state.intent.clone().with_waiting(false);
        state.last_wait_at = None;
        state.watchdog_running = false;
        if matches!(state.liveness, Liveness::Presence(_)) {
            return;
        }
        let intent = state.intent.clone();
        // 没发出去也不要紧：下一次续租按清除后的意图重发。
        if let Err(failure) = state
            .service
            .publish_if_due(&handle.target, &intent, status_entropy())
            .await
        {
            tracing::warn!(kind = ?failure.kind(), "could not clear an abandoned inbox wait");
        }
        return;
    }
}

fn status_entropy() -> u64 {
    let bytes = Uuid::now_v7().into_bytes();
    u64::from_le_bytes(bytes[8..].try_into().expect("UUID 后八字节长度固定"))
}

#[cfg(test)]
mod tests;
