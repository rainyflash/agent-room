//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：房主和管理员写的这间房的规矩。网络
//! Agent 进房间时拿这个房间现在的，之后改了，交一批消息时拿一次。现在的一版和交过的是哪一版记在
//! 收件箱的房间上（`NetworkAgentInboxStore`），同一版只交一次。
//!
//! 公告只写在私人房间和公开房间里，网络 Agent 进这些房间之前都先切到了加密客户端，它的同步不筛房间
//! 状态，所以同步时看得到公告。

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        MatrixEventType, MatrixRoomId, MatrixStateKey, MatrixSyncBatch, NetworkAgentRoomNotice,
    },
};
use agent_room_bridge_core::messages::{ROOM_NOTICE_EVENT_TYPE, room_notice, room_state_changes};
use agent_room_bridge_ipc::ROOM_NOTICES_PER_RESPONSE;

use super::NetworkGateway;

/// 这一批里每个房间最新的一版公告；读不出来的不算一版。
pub(super) fn synced(batch: &MatrixSyncBatch, own_user_id: &str) -> Vec<NetworkAgentRoomNotice> {
    room_state_changes(batch, own_user_id)
        .into_iter()
        .filter_map(|change| {
            let notice = change.notice?;
            Some(NetworkAgentRoomNotice {
                room_id: change.room_id,
                text: notice.text,
                updated_by: notice.updated_by,
                updated_at_ms: notice.updated_at_ms,
            })
        })
        .collect()
}

impl NetworkGateway {
    /// 交一批消息时拿改了的公告，记成交过了；给了房间只看它。拿不到就当没有，不耽误交消息。
    pub(super) async fn changed_notices(
        &self,
        session: &NetworkAgentSession,
        room: Option<&MatrixRoomId>,
    ) -> Vec<NetworkAgentRoomNotice> {
        let limit = u16::try_from(ROOM_NOTICES_PER_RESPONSE).unwrap_or(u16::MAX);
        match self
            .inbox
            .take_room_notices(session.network_agent_id, room, limit)
            .await
        {
            Ok(notices) => notices,
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    failure = ?failure.kind(),
                    "没读到改了的房间公告，下次交消息时再看"
                );
                Vec::new()
            }
        }
    }

    /// 进房间时读这个房间现在的公告，记成交过了，之后同步到同一版不再交。没有公告、撤掉了或者
    /// 读不到都是没有：读不到的那一版等交消息时再交。
    pub(super) async fn current_notice_internal(
        &self,
        token: &str,
        room_id: &str,
    ) -> Option<NetworkAgentRoomNotice> {
        let session = self.agents.session(token).await.ok()?;
        let room_id = MatrixRoomId::new(room_id).ok()?;
        let event_type = MatrixEventType::new(ROOM_NOTICE_EVENT_TYPE).ok()?;
        let state_key = MatrixStateKey::new("").ok()?;
        let content = match self
            .matrix
            .state_event(
                &session.matrix_access_token,
                &room_id,
                &event_type,
                &state_key,
            )
            .await
        {
            Ok(content) => content?,
            Err(failure) => {
                tracing::debug!(
                    network_agent.id = %session.network_agent_id,
                    failure = ?failure.kind(),
                    "进房间时没读到房间公告，交消息时再交"
                );
                return None;
            }
        };
        let notice = room_notice(&content)?;
        let notice = NetworkAgentRoomNotice {
            room_id,
            text: notice.text,
            updated_by: notice.updated_by,
            updated_at_ms: notice.updated_at_ms,
        };
        if let Err(failure) = self
            .inbox
            .give_room_notice(session.network_agent_id, &notice)
            .await
        {
            // 没记成交过：同步到这一版时会再交一次，宁可多说一次。
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                failure = ?failure.kind(),
                "进房间时交了房间公告，没记下交过"
            );
        }
        notice.text.is_some().then_some(notice)
    }
}
