//! 定时清理（ADR 0010 的治理）：停用 30 天没活动的与卡在创建中的网络 Agent，再替已停用、
//! 还没离开房间的网络 Agent 离开。后者包括运维停用的、闲置停用的，以及自己停用时没离开成的。
//! 离开了所有房间的，删掉服务器替它存的加密存储和钥匙（停用不能撤回，以后用不上了）。
//! 顺带关掉闲置太久的加密客户端。另有一项单独按时做：删掉过了房间保留期的消息副本。

use agent_room_application::{
    network_agents::{NetworkAgentFailure, NetworkAgentPendingExit},
    persistence::RepositoryResult,
    ports::NetworkAgentMessageRetention,
};
use agent_room_domain::ids::NetworkAgentId;

use super::NetworkGateway;

/// 每轮最多替这么多个网络 Agent 离开房间。
const EXIT_BATCH: u32 = 20;

/// 消息副本留多久：房间目录上没设保留期的，按聊天服务器的默认保留期（30 天）；到期后多留一天，
/// 等聊天服务器每天那次清理先删掉原来的事件，免得往回补时又补回来。和正文对象一样。
const MESSAGE_RETENTION: NetworkAgentMessageRetention = NetworkAgentMessageRetention {
    default_days: 30,
    grace_days: 1,
};

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
    /// 删掉了加密存储和钥匙的。
    pub(crate) keys_deleted: usize,
    /// 关掉的闲置加密客户端。
    pub(crate) closed: usize,
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
                    self.close_encrypted(id).await;
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
                    self.close_encrypted(id).await;
                    self.agents.mark_rooms_left(id).await?;
                    outcome.abandoned += 1;
                }
            }
        }
        for id in self.agents.pending_key_deletions(EXIT_BATCH).await? {
            // 先删存储再删钥匙：存储没删掉时钥匙留着，下一轮还会再来删。
            if self.remove_store(id).await {
                self.agents.delete_keys(id).await?;
                outcome.keys_deleted += 1;
            }
        }
        if let Some(encrypted) = &self.encrypted {
            outcome.closed = encrypted.evict_idle().await;
        }
        Ok(outcome)
    }

    /// 删掉所有网络 Agent 过了房间保留期的收件箱和消息记录，返回删了几条。
    pub(crate) async fn prune_expired_messages(&self) -> RepositoryResult<u64> {
        self.inbox
            .prune_expired(self.clock.now(), MESSAGE_RETENTION)
            .await
    }

    /// 删掉服务器上替它存的加密存储；没配加密客户端的就没有存储，当作删好了。
    async fn remove_store(&self, id: NetworkAgentId) -> bool {
        match &self.encrypted {
            Some(encrypted) => encrypted.remove_store(id).await.is_ok(),
            None => true,
        }
    }
}
