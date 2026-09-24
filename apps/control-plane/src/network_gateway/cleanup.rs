//! 定时清理（ADR 0010 的治理）：停用 30 天没活动的与卡在创建中的网络 Agent，再替已停用、
//! 还没离开房间的网络 Agent 离开。后者包括运维停用的、闲置停用的，以及自己停用时没离开成的。

use agent_room_application::network_agents::{NetworkAgentFailure, NetworkAgentPendingExit};

use super::NetworkGateway;

/// 每轮最多替这么多个网络 Agent 离开房间。
const EXIT_BATCH: u32 = 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NetworkAgentCleanupOutcome {
    /// 这一轮因闲置或卡在创建中而停用的。
    pub(crate) disabled: usize,
    /// 替它们离开了所有房间的。
    pub(crate) left: usize,
    /// 会话打不开、没法替它离开，只好记为已离开的。
    pub(crate) abandoned: usize,
    /// 有房间没离开成、下一轮再试的。
    pub(crate) retrying: usize,
}

impl NetworkGateway {
    /// 清理一轮。库不可用时整轮失败，下一轮再来；个别房间没离开成只影响那一个 Agent。
    pub(crate) async fn clean_up(&self) -> Result<NetworkAgentCleanupOutcome, NetworkAgentFailure> {
        let mut outcome = NetworkAgentCleanupOutcome {
            disabled: self.agents.disable_stale().await?,
            ..NetworkAgentCleanupOutcome::default()
        };
        for exit in self.agents.pending_exits(EXIT_BATCH).await? {
            match exit {
                NetworkAgentPendingExit::Session(session) => {
                    let id = session.network_agent_id;
                    let left = self.leave_rooms(&session).await;
                    self.presence.forget(id).await;
                    if left {
                        self.agents.mark_rooms_left(id).await?;
                        outcome.left += 1;
                    } else {
                        outcome.retrying += 1;
                    }
                }
                NetworkAgentPendingExit::Unopenable(id) => {
                    tracing::warn!(
                        network_agent.id = %id,
                        "停用的网络 Agent 会话打不开，没法替它离开房间；不再重试"
                    );
                    self.agents.mark_rooms_left(id).await?;
                    outcome.abandoned += 1;
                }
            }
        }
        Ok(outcome)
    }
}
