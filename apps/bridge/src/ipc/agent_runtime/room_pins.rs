//! 房间的置顶（`specs/redesign/brief-and-pins.md`“Agent 读置顶”）：同步时记在本地消息库里，Agent 进房间
//! 时拿现在的，之后改了，交一批消息时再拿一次整份。交出去就记下交过了这一版。
//!
//! 服务器上只记事件 ID：置顶的那一句是谁说的、开头写了什么，照本地消息库里已经有的那一条；还没有的
//! 写明 `not_loaded`（往前翻也许翻得到），全文照旧按 ID 取。

use agent_room_application::ports::MatrixRoomId;
use agent_room_bridge_core::messages::{MessageLookupId, ProjectedMessagePreview, RoomPinsState};
use agent_room_bridge_ipc::previews::reply_excerpt;
use agent_room_bridge_ipc::{
    IpcPinUnreadable, IpcReadRoomPinsRequest, IpcResponse, IpcRoomPin, IpcRoomPins,
    ROOM_PINS_PER_RESPONSE,
};

use super::{
    AgentRuntimeIpcFacade, BridgeIpcDispatchFailure, every_room::rooms_of,
    map_preview_query_failure,
};

impl AgentRuntimeIpcFacade {
    /// 看哪些房间和读公告一样。一次最多交 [`ROOM_PINS_PER_RESPONSE`] 个房间，剩下的没记成交过，
    /// 下次再交。
    pub(in crate::ipc) async fn read_room_pins(
        &self,
        request: IpcReadRoomPinsRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let rooms = if request.every_room {
            rooms_of(&runtime).await?
        } else {
            vec![runtime.message_room(request.room_id).await?.0]
        };
        let mut taken = Vec::new();
        for room in rooms {
            if taken.len() == ROOM_PINS_PER_RESPONSE {
                break;
            }
            if let Some(pins) = self
                .previews
                .take_room_pins(&room, request.changed_only)
                .await
                .map_err(map_preview_query_failure)?
            {
                taken.push((room, pins));
            }
        }
        // 置顶的那几句一次查完。
        let wanted: Vec<MessageLookupId> = taken
            .iter()
            .flat_map(|(_, pins)| {
                pins.entries
                    .iter()
                    .map(|pin| MessageLookupId::Event(pin.event_id.clone()))
            })
            .collect();
        let found = if wanted.is_empty() {
            Vec::new()
        } else {
            self.previews
                .lookup_messages(&wanted)
                .await
                .map_err(map_preview_query_failure)?
        };
        let rooms = taken
            .into_iter()
            .map(|(room, pins)| room_pins(&room, pins, &found))
            .collect();
        Ok(IpcResponse::RoomPins { rooms })
    }
}

fn room_pins(
    room: &MatrixRoomId,
    pins: RoomPinsState,
    found: &[ProjectedMessagePreview],
) -> IpcRoomPins {
    let pinned = pins
        .entries
        .into_iter()
        .map(|pin| {
            let message = found
                .iter()
                .find(|message| message.event_id == pin.event_id && message.room_id == *room);
            IpcRoomPin {
                event_id: pin.event_id.as_str().to_owned(),
                pinned_by: pin.pinned_by,
                pinned_at_unix_ms: pin.pinned_at_ms,
                message: message.map(reply_excerpt),
                unreadable: message.is_none().then_some(IpcPinUnreadable::NotLoaded),
            }
        })
        .collect();
    IpcRoomPins {
        room_id: room.as_str().to_owned(),
        pinned,
        updated_by: pins.updated_by,
        updated_at_unix_ms: pins.updated_at_ms,
    }
}
