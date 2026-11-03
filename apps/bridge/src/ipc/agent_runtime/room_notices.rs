//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：同步时记在本地消息库里，Agent 进房间
//! 时拿现在的，之后改了，交一批消息时再拿一次。交出去就记下交过了这一版。

use agent_room_bridge_ipc::{
    IpcReadRoomNoticesRequest, IpcResponse, IpcRoomNotice, ROOM_NOTICES_PER_RESPONSE,
};

use super::{
    AgentRuntimeIpcFacade, BridgeIpcDispatchFailure, every_room::rooms_of,
    map_preview_query_failure,
};

impl AgentRuntimeIpcFacade {
    /// 默认看会话的那个房间，给了房间看它（要在里面），`everyRoom` 看 Agent 在的所有房间、会话的
    /// 那个在前。一次最多交 [`ROOM_NOTICES_PER_RESPONSE`] 个，剩下的没记成交过，下次再交。
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
            let Some(notice) = self
                .previews
                .take_room_notice(&room, request.changed_only)
                .await
                .map_err(map_preview_query_failure)?
            else {
                continue;
            };
            notices.push(IpcRoomNotice {
                room_id: room.as_str().to_owned(),
                text: notice.text,
                updated_by: notice.updated_by,
                updated_at_unix_ms: notice.updated_at_ms,
            });
        }
        Ok(IpcResponse::RoomNotices { notices })
    }
}
