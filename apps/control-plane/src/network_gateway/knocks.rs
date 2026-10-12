//! 有人敲网络 Agent 建的房间的门：它正在等消息就叫醒它（`specs/agent-rooms/design.md` 的“敲门叫醒
//! 房主 Agent”）。能放行的只有它自己，不叫醒它，敲门放一小时没人回答就作废了。
//!
//! 敲门记在库里，同步 Matrix 看不到，所以等消息时隔一会儿（`RECHECK`）去库里看一眼。这个进程里有人
//! 敲门就打一下铃，有房间的房主听到马上去看；别的进程里敲的，下一次看时也看得到。
//!
//! 同一次敲门只叫醒一次：交给它的回答里列过的就记下，只记在这个进程里，重启以后会再叫醒一次。再敲
//! 一次算新的一次。被新的等待顶掉的那次回答多半没人看，不列敲门，也不算告诉过它。

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, PoisonError},
    time::Duration,
};

use agent_room_application::network_agents::{
    NetworkAgentIncomingKnock, NetworkAgentIncomingKnocks, NetworkAgentRoomUseCases,
};
use agent_room_bridge_ipc::wake::WakeReason;
use agent_room_domain::ids::{AgentId, NetworkAgentId, RoomCatalogId};
use tokio::{sync::watch, time::Instant};

use super::NetworkAgentMessages;

/// 没听到铃时隔多久去库里看一次：别的进程里有人敲门，最晚这么久以后（再加一段同步）叫醒它。
const RECHECK: Duration = Duration::from_secs(10);

/// 一次敲门：哪个房间、谁敲的、什么时候敲的。
type KnockKey = (RoomCatalogId, AgentId, i64);

pub(super) struct KnockWatch {
    bell: watch::Sender<u64>,
    /// 交给每个网络 Agent 的回答里列过的敲门。
    shown: Mutex<HashMap<NetworkAgentId, HashSet<KnockKey>>>,
}

impl Default for KnockWatch {
    fn default() -> Self {
        Self {
            bell: watch::channel(0).0,
            shown: Mutex::default(),
        }
    }
}

impl KnockWatch {
    /// 这个进程里有人敲了门：有房间、正在等的房主马上去看一眼。
    pub(super) fn ring(&self) {
        self.bell
            .send_modify(|rings| *rings = rings.wrapping_add(1));
    }

    /// 一次等消息里看敲门：第一次看马上就看。
    pub(super) fn listen(&self, agent: NetworkAgentId) -> KnockListener<'_> {
        KnockListener {
            watch: self,
            agent,
            bell: self.bell.subscribe(),
            rang: false,
            next_check: None,
            current: None,
        }
    }

    fn unseen(&self, agent: NetworkAgentId, knocks: &[NetworkAgentIncomingKnock]) -> bool {
        let shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let seen = shown.get(&agent);
        knocks
            .iter()
            .any(|knock| !seen.is_some_and(|seen| seen.contains(&key(knock))))
    }

    /// 回答里列了这些敲门：记下，同一次敲门不再叫醒它。不在等的顺手忘掉。
    fn remember(&self, agent: NetworkAgentId, knocks: &[NetworkAgentIncomingKnock]) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if knocks.is_empty() {
            shown.remove(&agent);
        } else {
            shown.insert(agent, knocks.iter().map(key).collect());
        }
    }
}

pub(super) struct KnockListener<'a> {
    watch: &'a KnockWatch,
    agent: NetworkAgentId,
    bell: watch::Receiver<u64>,
    /// 等铃的时候听到过：下一次马上去看。
    rang: bool,
    /// 下一次去库里看的时刻；还没看过时为空。
    next_check: Option<Instant>,
    /// 最近一次看到的；一次都没看成时为空。
    current: Option<NetworkAgentIncomingKnocks>,
}

impl KnockListener<'_> {
    /// 到了该看的时候（第一次、听到铃、隔了 `RECHECK`）去库里看一眼。没看成就照旧用上一次看到的：
    /// 只是晚一点叫醒它，不耽误交消息。一间房都没有的不听铃，和 `rung` 一样。
    pub(super) async fn refresh(&mut self, rooms: &dyn NetworkAgentRoomUseCases, token: &str) {
        let rang = self.rang || (self.listening() && self.bell.has_changed().unwrap_or(false));
        if !rang && self.next_check.is_some_and(|at| Instant::now() < at) {
            return;
        }
        self.rang = false;
        self.bell.borrow_and_update();
        self.next_check = Some(Instant::now() + RECHECK);
        match rooms.incoming_knocks(token).await {
            Ok(knocks) => self.current = Some(knocks),
            Err(failure) => tracing::warn!(
                network_agent.id = %self.agent,
                kind = ?failure.kind(),
                "等消息时没看成有没有人敲它建的房间的门，过一会儿再看"
            ),
        }
    }

    /// 有还没告诉过它的敲门。
    pub(super) fn fresh(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| self.watch.unseen(self.agent, &current.knocks))
    }

    /// 把在等的敲门列进交给它的回答，记为告诉过它。没有消息、是敲门叫醒它的，原因写成敲门。
    pub(super) fn attach(&self, mut messages: NetworkAgentMessages) -> NetworkAgentMessages {
        let Some(current) = &self.current else {
            return messages;
        };
        if messages.messages.is_empty() && self.fresh() {
            messages.wake.reason = WakeReason::Knocks;
        }
        self.watch.remember(self.agent, &current.knocks);
        messages.incoming_knocks.clone_from(&current.knocks);
        messages
    }

    /// 等到铃响。一间没关的房间都没有的不听：谁敲门都跟它无关，不必打断它的同步。
    pub(super) async fn rung(&mut self) {
        if self.listening() && self.bell.changed().await.is_ok() {
            self.rang = true;
            return;
        }
        // 网关一直拿着铃，`changed` 不会出错；真出错也当没响，别空转。
        std::future::pending::<()>().await;
    }

    /// 听不听铃：上一次看到它有没关的房间才听。
    fn listening(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.owns_rooms)
    }
}

fn key(incoming: &NetworkAgentIncomingKnock) -> KnockKey {
    (
        incoming.room.catalog_id,
        incoming.knock.agent_id,
        incoming.knock.knocked_at.value(),
    )
}
