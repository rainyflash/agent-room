//! 收件箱的确认位置（`specs/agent-reading/design.md` 第 4 步）：Agent 处理完一批，确认到最后一条，
//! 下次不带位置读收件箱、等消息时就从它之后开始。位置按房间记，只往前走。

use agent_room_bridge_ipc::{IpcAckInboxRequest, IpcResponse};

use super::{
    AgentRuntimeIpcFacade, BridgeIpcDispatchFailure, map_preview_query_failure,
    viewing::{lookup_id, message_not_found},
};

impl AgentRuntimeIpcFacade {
    /// 确认到某一条（含）：事件 ID 或消息 ID，不用给房间；只认这个 Agent 所在房间里的。
    pub(in crate::ipc) async fn ack_inbox(
        &self,
        request: IpcAckInboxRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let id = lookup_id(&request.id)?;
        let Some((room_id, event_id)) = self
            .previews
            .find_inbox_event(&id)
            .await
            .map_err(map_preview_query_failure)?
        else {
            return Err(message_not_found());
        };
        // 不在的房间（离开了、被移出了）里的消息，和找不到一样。
        match runtime
            .message_room(Some(room_id.as_str().to_owned()))
            .await
        {
            Ok(_) => {}
            Err(failure) if failure.code == "bridge.room_not_joined" => {
                return Err(message_not_found());
            }
            Err(failure) => return Err(failure),
        }
        let acknowledgement = self
            .previews
            .acknowledge_inbox(
                &room_id,
                &event_id,
                runtime.identity.matrix_user_id(),
                self.clock.now(),
            )
            .await
            .map_err(map_preview_query_failure)?;
        Ok(IpcResponse::InboxAcknowledged {
            room_id: room_id.as_str().to_owned(),
            event_id: event_id.as_str().to_owned(),
            acknowledged: acknowledgement.acknowledged,
            pending: acknowledgement.pending,
        })
    }
}
