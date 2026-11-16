//! 房间的置顶（`specs/redesign/brief-and-pins.md`“Agent 读置顶”）：网络 Agent 进房间时拿这个房间现在的，
//! 之后改了，交一批消息时拿一次整份。现在的一版和交过的是哪一版记在收件箱的房间上
//! （`NetworkAgentInboxStore`），同一版只交一次；置顶改了不叫醒它。
//!
//! 置顶状态里只有事件 ID。那一句是谁说的、开头写了什么，照它自己的消息记录（它进来以后收到的，每个
//! 房间最近 500 条）；记录里没有的多半是它进来以前的加密消息，它读不到，写明 `before_join`、不给内容。
//! 消息记录读不出来时写 `not_loaded`：说不清是哪一种。

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        MatrixRoomId, MatrixSyncBatch, NetworkAgentMessageRef, NetworkAgentPin,
        NetworkAgentRoomPins, NetworkAgentStoredMessage,
    },
};
use agent_room_bridge_core::messages::{
    ROOM_PINS_EVENT_TYPE, RoomPinsState, room_pins, room_state_changes,
};
use agent_room_bridge_ipc::{
    IpcMessagePreviewSummary, IpcPinUnreadable, IpcRoomPin, IpcRoomPins, ROOM_PINS_PER_RESPONSE,
    previews::stored_excerpt,
};
use agent_room_domain::ids::NetworkAgentId;

use super::NetworkGateway;

/// 这一批里每个房间最新的一版置顶；读不出来的不算一版。
pub(super) fn synced(batch: &MatrixSyncBatch, own_user_id: &str) -> Vec<NetworkAgentRoomPins> {
    room_state_changes(batch, own_user_id)
        .into_iter()
        .filter_map(|change| Some(network_pins(change.room_id, change.pins?)))
        .collect()
}

fn network_pins(room_id: MatrixRoomId, pins: RoomPinsState) -> NetworkAgentRoomPins {
    NetworkAgentRoomPins {
        room_id,
        entries: pins
            .entries
            .into_iter()
            .map(|entry| NetworkAgentPin {
                event_id: entry.event_id,
                pinned_by: entry.pinned_by,
                pinned_at_ms: entry.pinned_at_ms,
            })
            .collect(),
        updated_by: pins.updated_by,
        updated_at_ms: pins.updated_at_ms,
    }
}

impl NetworkGateway {
    /// 交一批消息时拿置顶改了的房间，记成交过了；给了房间只看它。拿不到就当没有，不耽误交消息。
    pub(super) async fn changed_pins(
        &self,
        session: &NetworkAgentSession,
        room: Option<&MatrixRoomId>,
    ) -> Vec<IpcRoomPins> {
        let limit = u16::try_from(ROOM_PINS_PER_RESPONSE).unwrap_or(u16::MAX);
        match self
            .inbox
            .take_room_pins(session.network_agent_id, room, limit)
            .await
        {
            Ok(rooms) => self.resolve(session.network_agent_id, rooms).await,
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    failure = ?failure.kind(),
                    "没读到改了的置顶，下次交消息时再看"
                );
                Vec::new()
            }
        }
    }

    /// 进房间时读这个房间现在的置顶，记成交过了，之后同步到同一版不再交。没有置顶、全取消了或者读
    /// 不到都是没有：读不到的那一版等交消息时再交。
    pub(super) async fn current_pins_internal(
        &self,
        token: &str,
        room_id: &str,
    ) -> Option<IpcRoomPins> {
        let session = self.agents.session(token).await.ok()?;
        let room_id = MatrixRoomId::new(room_id).ok()?;
        let content = self
            .room_state(&session, &room_id, ROOM_PINS_EVENT_TYPE)
            .await?;
        let pins = network_pins(room_id, room_pins(&content)?);
        if let Err(failure) = self
            .inbox
            .give_room_pins(session.network_agent_id, &pins)
            .await
        {
            // 没记成交过：同步到这一版时会再交一次，宁可多说一次。
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                failure = ?failure.kind(),
                "进房间时交了置顶，没记下交过"
            );
        }
        if pins.entries.is_empty() {
            return None;
        }
        self.resolve(session.network_agent_id, vec![pins])
            .await
            .pop()
    }

    /// 置顶的那几句一次去消息记录里找。
    async fn resolve(
        &self,
        id: NetworkAgentId,
        rooms: Vec<NetworkAgentRoomPins>,
    ) -> Vec<IpcRoomPins> {
        let refs: Vec<NetworkAgentMessageRef> = rooms
            .iter()
            .flat_map(|room| {
                room.entries
                    .iter()
                    .map(|pin| NetworkAgentMessageRef::Event(pin.event_id.clone()))
            })
            .collect();
        let found = if refs.is_empty() {
            Some(Vec::new())
        } else {
            match self.history.messages_by_id(id, &refs).await {
                Ok(found) => Some(found),
                Err(error) => {
                    tracing::warn!(
                        network_agent.id = %id,
                        error = ?error,
                        "网络 Agent 的消息记录读不出来，置顶的那几句只能先不写"
                    );
                    None
                }
            }
        };
        rooms
            .into_iter()
            .map(|room| ipc_pins(room, found.as_deref()))
            .collect()
    }
}

fn ipc_pins(
    room: NetworkAgentRoomPins,
    found: Option<&[NetworkAgentStoredMessage]>,
) -> IpcRoomPins {
    let pinned = room
        .entries
        .into_iter()
        .map(|pin| {
            let message = found.and_then(|found| {
                let stored = found.iter().find(|message| {
                    message.event_id == pin.event_id && message.room_id == room.room_id
                })?;
                let summary: IpcMessagePreviewSummary =
                    serde_json::from_value(stored.preview.clone()).ok()?;
                Some(stored_excerpt(&summary))
            });
            let unreadable = match (&message, found) {
                (Some(_), _) => None,
                (None, Some(_)) => Some(IpcPinUnreadable::BeforeJoin),
                (None, None) => Some(IpcPinUnreadable::NotLoaded),
            };
            IpcRoomPin {
                event_id: pin.event_id.as_str().to_owned(),
                pinned_by: pin.pinned_by,
                pinned_at_unix_ms: pin.pinned_at_ms,
                message,
                unreadable,
            }
        })
        .collect();
    IpcRoomPins {
        room_id: room.room_id.as_str().to_owned(),
        pinned,
        updated_by: room.updated_by,
        updated_at_unix_ms: room.updated_at_ms,
    }
}
