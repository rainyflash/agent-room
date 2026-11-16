//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：房主和管理员写的这间房的规矩。网络
//! Agent 进房间时拿这个房间现在的，之后改了，交一批消息时拿一次。现在的一版和交过的是哪一版记在
//! 收件箱的房间上（`NetworkAgentInboxStore`），同一版只交一次。
//!
//! 公告只写在私人房间和公开房间里，网络 Agent 进这些房间之前都先切到了加密客户端，它的同步不筛房间
//! 状态，所以同步时看得到公告。
//!
//! 子房间里还有一份父房间须知的副本（`room.parent_notice.v1`，`specs/redesign/brief-and-pins.md`
//! “子房间里父房间的须知也算”）：它不一定在父房间里，读不到父房间的状态，所以照副本交父房间那一段，
//! `roomId` 是父房间的，交过哪一版记在子房间那一行上。

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        MatrixEventType, MatrixRoomId, MatrixStateKey, MatrixSyncBatch, NetworkAgentRoomNotice,
    },
};
use agent_room_bridge_core::messages::{
    ROOM_NOTICE_EVENT_TYPE, ROOM_PARENT_NOTICE_EVENT_TYPE, RoomNotice, room_notice,
    room_parent_notice, room_state_changes,
};
use agent_room_bridge_ipc::ROOM_NOTICES_PER_RESPONSE;
use agent_room_domain::ids::NetworkAgentId;
use serde_json::Value;

use super::NetworkGateway;

/// 这一批里每个房间最新的一版公告，子房间里父房间那一段也算；读不出来的不算一版。
pub(super) fn synced(batch: &MatrixSyncBatch, own_user_id: &str) -> Vec<NetworkAgentRoomNotice> {
    let mut notices = Vec::new();
    for change in room_state_changes(batch, own_user_id) {
        if let Some(notice) = change.notice {
            notices.push(network_notice(change.room_id.clone(), None, notice));
        }
        if let Some(copy) = change.parent_notice {
            notices.push(network_notice(
                copy.parent_room_id,
                Some(change.room_id),
                copy.notice,
            ));
        }
    }
    notices
}

fn network_notice(
    room_id: MatrixRoomId,
    in_sub_room: Option<MatrixRoomId>,
    notice: RoomNotice,
) -> NetworkAgentRoomNotice {
    NetworkAgentRoomNotice {
        room_id,
        in_sub_room,
        text: notice.text,
        updated_by: notice.updated_by,
        updated_at_ms: notice.updated_at_ms,
    }
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

    /// 进房间时读这个房间现在的公告，子房间里连父房间那一段，记成交过了，之后同步到同一版不再交。
    /// 这一间的在前。没有公告、撤掉了或者读不到都是没有：读不到的那一版等交消息时再交。
    pub(super) async fn current_notices_internal(
        &self,
        token: &str,
        room_id: &str,
    ) -> Vec<NetworkAgentRoomNotice> {
        let (Ok(session), Ok(room_id)) =
            (self.agents.session(token).await, MatrixRoomId::new(room_id))
        else {
            return Vec::new();
        };
        let own = self
            .room_state(&session, &room_id, ROOM_NOTICE_EVENT_TYPE)
            .await
            .and_then(|content| room_notice(&content))
            .map(|notice| network_notice(room_id.clone(), None, notice));
        let parent = self
            .room_state(&session, &room_id, ROOM_PARENT_NOTICE_EVENT_TYPE)
            .await
            .and_then(|content| room_parent_notice(&content))
            .map(|copy| network_notice(copy.parent_room_id, Some(room_id.clone()), copy.notice));
        let mut notices = Vec::new();
        for notice in own.into_iter().chain(parent) {
            self.give(session.network_agent_id, &notice).await;
            if notice.text.is_some() {
                notices.push(notice);
            }
        }
        notices
    }

    /// 读房间里应用服务写的那一条状态（`state_key` 为空）；没有或者读不到都是没有。
    async fn room_state(
        &self,
        session: &NetworkAgentSession,
        room_id: &MatrixRoomId,
        event_type: &str,
    ) -> Option<Value> {
        let event_type = MatrixEventType::new(event_type).ok()?;
        let state_key = MatrixStateKey::new("").ok()?;
        match self
            .matrix
            .state_event(
                &session.matrix_access_token,
                room_id,
                &event_type,
                &state_key,
            )
            .await
        {
            Ok(content) => content,
            Err(failure) => {
                tracing::debug!(
                    network_agent.id = %session.network_agent_id,
                    failure = ?failure.kind(),
                    "进房间时没读到房间公告，交消息时再交"
                );
                None
            }
        }
    }

    async fn give(&self, id: NetworkAgentId, notice: &NetworkAgentRoomNotice) {
        if let Err(failure) = self.inbox.give_room_notice(id, notice).await {
            // 没记成交过：同步到这一版时会再交一次，宁可多说一次。
            tracing::warn!(
                network_agent.id = %id,
                failure = ?failure.kind(),
                "进房间时交了房间公告，没记下交过"
            );
        }
    }
}
