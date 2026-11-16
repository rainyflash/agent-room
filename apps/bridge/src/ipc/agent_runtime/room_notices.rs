//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：同步时记在本地消息库里，Agent 进房间
//! 时拿现在的，之后改了，交一批消息时再拿一次。交出去就记下交过了这一版。
//!
//! 子房间里父房间须知的副本（`specs/redesign/brief-and-pins.md`“子房间里父房间的须知也算”）当成父房间
//! 的那一段，`roomId` 是父房间的。回答的形状不变：IPC 回答不认多出来的字段，加字段会让还开着的旧版
//! MCP 和命令行整个读不出来。父房间那一段排在这一间自己的后面：旧版进房间时只拿第一段，照旧是这一间的。

use agent_room_bridge_core::messages::RoomNotice;
use agent_room_bridge_ipc::{
    IpcReadRoomNoticesRequest, IpcResponse, IpcRoomNotice, ROOM_NOTICES_PER_RESPONSE,
};

use super::{
    AgentRuntimeIpcFacade, BridgeIpcDispatchFailure, every_room::rooms_of,
    map_preview_query_failure,
};

impl AgentRuntimeIpcFacade {
    /// 默认看会话的那个房间，给了房间看它（要在里面），`everyRoom` 看 Agent 在的所有房间、会话的
    /// 那个在前。一次最多交 [`ROOM_NOTICES_PER_RESPONSE`] 段，剩下的没记成交过，下次再交。
    pub(in crate::ipc) async fn read_room_notices(
        &self,
        request: IpcReadRoomNoticesRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let rooms = if request.every_room {
            rooms_of(&runtime).await?
        } else {
            vec![runtime.message_room(request.room_id).await?.0]
        };
        let mut notices = Vec::new();
        for room in rooms {
            if notices.len() == ROOM_NOTICES_PER_RESPONSE {
                break;
            }
            if let Some(notice) = self
                .previews
                .take_room_notice(&room, request.changed_only)
                .await
                .map_err(map_preview_query_failure)?
            {
                notices.push(ipc_notice(room.as_str(), notice));
            }
            if notices.len() == ROOM_NOTICES_PER_RESPONSE {
                break;
            }
            if let Some(copy) = self
                .previews
                .take_parent_notice(&room, request.changed_only)
                .await
                .map_err(map_preview_query_failure)?
            {
                notices.push(ipc_notice(copy.parent_room_id.as_str(), copy.notice));
            }
        }
        Ok(IpcResponse::RoomNotices { notices })
    }
}

fn ipc_notice(room_id: &str, notice: RoomNotice) -> IpcRoomNotice {
    IpcRoomNotice {
        room_id: room_id.to_owned(),
        text: notice.text,
        updated_by: notice.updated_by,
        updated_at_unix_ms: notice.updated_at_ms,
    }
}
