//! 定时清理（ADR 0010 的治理）：停用 30 天没活动的与卡在创建中的网络 Agent，再替已停用、
//! 还没离开房间的网络 Agent 离开。后者包括运维停用的、闲置停用的，以及自己停用时没离开成的。
//! 离开之前先关掉它当房主的私人房间，关不成就先不离开，下一轮再来。
//! 离开了所有房间的，删掉服务器替它存的加密存储和钥匙（停用不能撤回，以后用不上了）。
//! 一小时没调接口的网络 Agent 先请出公开大厅，它下次调接口时服务器替它回来
//! （`specs/launch-capacity/design.md` 第 2 步）。
//! 顺带关掉闲置太久的加密客户端。另有一项单独按时做：删掉过了房间保留期的消息副本。

use agent_room_application::{
    network_agents::{NetworkAgentFailure, NetworkAgentLobbyExit, NetworkAgentPendingExit},
    persistence::RepositoryResult,
    ports::{MatrixFailureKind, MatrixRoomId, NetworkAgentMessageRetention},
};
use agent_room_domain::ids::NetworkAgentId;

use super::NetworkGateway;

/// 每轮最多替这么多个网络 Agent 离开房间。
const EXIT_BATCH: u32 = 20;

/// 每轮最多请出这么多个闲置网络 Agent 所在的大厅分片。
const LOBBY_IDLE_BATCH: u32 = 50;

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
    /// 替停用的网络 Agent 关掉的、它当房主的私人房间。
    pub(crate) archived: usize,
    /// 替它们离开了所有房间的。
    pub(crate) left: usize,
    /// 会话打不开、没法替它离开，只好记为已离开的。
    pub(crate) abandoned: usize,
    /// 有房间没离开成、下一轮再试的（停用的和闲置的都算）。
    pub(crate) retrying: usize,
    /// 一小时没动静、请出了公开大厅的（每个大厅分片算一次）。
    pub(crate) away: usize,
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
                    let left = match self.archive_owned_rooms(id, session.principal_id).await {
                        Some(archived) => {
                            outcome.archived += archived;
                            self.leave_rooms(&session).await
                        }
                        None => false,
                    };
                    self.presence.forget(id).await;
                    self.close_encrypted(id).await;
                    if left {
                        self.agents.mark_rooms_left(id).await?;
                        outcome.left += 1;
                    } else {
                        outcome.retrying += 1;
                    }
                }
                NetworkAgentPendingExit::Unopenable { id, principal_id } => {
                    // 关房间用不着它的会话，照样关；关不成就下一轮再来。
                    let Some(archived) = self.archive_owned_rooms(id, principal_id).await else {
                        outcome.retrying += 1;
                        continue;
                    };
                    outcome.archived += archived;
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
        self.leave_idle_lobbies(&mut outcome).await?;
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

    /// 用它自己的会话离开这个大厅分片，再记一笔、放回位置；它下次调接口时服务器替它回来。
    /// 先离开再记：库里记的照 Matrix 的事实来，刚好在这时调了接口的那一次可能扑空，下一次就回来了。
    async fn leave_idle_lobbies(
        &self,
        outcome: &mut NetworkAgentCleanupOutcome,
    ) -> Result<(), NetworkAgentFailure> {
        for exit in self.agents.idle_lobby_exits(LOBBY_IDLE_BATCH).await? {
            if self.leave_idle_lobby(&exit).await {
                self.agents.mark_lobby_away(exit.id, &exit.room).await?;
                outcome.away += 1;
            } else {
                outcome.retrying += 1;
            }
        }
        Ok(())
    }

    /// 离开了（或者没法替它离开、只好放弃）返回 true；聊天服务器一时不通返回 false，下一轮再试。
    /// 会话打不开、令牌被拒这类再试也没用的放弃，免得最久没动静的那几个每轮都占着名额。
    async fn leave_idle_lobby(&self, exit: &NetworkAgentLobbyExit) -> bool {
        let Some(token) = &exit.matrix_access_token else {
            tracing::warn!(
                network_agent.id = %exit.id,
                "闲置的网络 Agent 会话打不开，没法替它离开公开大厅；只放回位置"
            );
            return true;
        };
        let Ok(room_id) = MatrixRoomId::new(exit.room.matrix_room_id.as_str()) else {
            return true;
        };
        match self.matrix.leave(token, &room_id).await {
            Ok(()) => true,
            Err(failure)
                if matches!(
                    failure.kind(),
                    MatrixFailureKind::RateLimited
                        | MatrixFailureKind::Timeout
                        | MatrixFailureKind::DependencyUnavailable
                        | MatrixFailureKind::UnknownCommit
                ) =>
            {
                tracing::warn!(
                    network_agent.id = %exit.id,
                    kind = ?failure.kind(),
                    "没能替闲置的网络 Agent 离开公开大厅，下一轮再试"
                );
                false
            }
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %exit.id,
                    kind = ?failure.kind(),
                    "替闲置的网络 Agent 离开公开大厅被拒，不再重试；只放回位置"
                );
                true
            }
        }
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
