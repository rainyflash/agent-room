//! 主人交给 Agent 管的房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：交一批消息时，
//! 这一批里有管理授权的房间各带一条提示（[`crate::InboxWaiter::with_moderation`]），Agent 好记得照规则
//! 管。规则本身不带，要读用 `ModerateRoom` 的 `rules`。问不到（旧控制面、Bridge 还没准备好）就当没有，
//! 不耽误等消息。

use agent_room_bridge_ipc::{
    IpcMessagePreviewSummary, IpcMethod, IpcModerateRoomRequest, IpcModerationAction, IpcResponse,
};
use serde::Serialize;

use crate::BridgeToolClient;

/// 这一批里一个交给 Agent 管的房间：能做哪几样、到什么时候。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomModeration {
    /// Matrix 房间 ID，和消息里的 `roomId` 一样。
    pub room_id: String,
    pub actions: Vec<String>,
    pub expires_at_unix_ms: i64,
}

/// 这一批消息所在的房间里，有授权的那几个；按消息里先出现的顺序，每个房间问一次。
pub(crate) async fn granted_rooms(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    previews: &[IpcMessagePreviewSummary],
) -> Vec<RoomModeration> {
    let mut asked: Vec<&str> = Vec::new();
    let mut granted = Vec::new();
    for preview in previews {
        let room_id = preview.room_id.as_str();
        if asked.contains(&room_id) {
            continue;
        }
        asked.push(room_id);
        if let Some(moderation) = grant(backend, session_id, room_id).await {
            granted.push(moderation);
        }
    }
    granted
}

async fn grant(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: &str,
) -> Option<RoomModeration> {
    let method = IpcMethod::WithSession {
        session_id: session_id.to_owned(),
        method: Box::new(IpcMethod::ModerateRoom(IpcModerateRoomRequest {
            action: IpcModerationAction::Rules,
            catalog_id: None,
            room_id: Some(room_id.to_owned()),
            target: None,
            reason: None,
            note: None,
            duration_seconds: None,
        })),
    };
    match backend.invoke(method).await {
        Ok(IpcResponse::RoomModerationRules {
            grant: Some(grant), ..
        }) => Some(RoomModeration {
            room_id: room_id.to_owned(),
            actions: grant.actions,
            expires_at_unix_ms: grant.expires_at_unix_ms,
        }),
        _ => None,
    }
}
