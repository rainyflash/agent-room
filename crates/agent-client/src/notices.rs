//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：进房间时拿这个房间现在的，之后改了，
//! 交一批消息时拿一次（[`crate::InboxWaiter::with_notices`]）。Bridge 记着交过哪一版，同一版只交一次。
//! 拿不到（Bridge 还没准备好）就当没有，不耽误进房间和等消息。

use agent_room_bridge_ipc::{IpcMethod, IpcReadRoomNoticesRequest, IpcResponse, IpcRoomNotice};

use crate::BridgeToolClient;

/// 进房间时拿这个房间现在的公告；不给房间是会话的那个。没有公告、撤掉了或者拿不到都是没有。
pub async fn room_notice(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: Option<&str>,
) -> Option<IpcRoomNotice> {
    let request = IpcReadRoomNoticesRequest {
        room_id: room_id.map(str::to_owned),
        every_room: false,
        changed_only: false,
    };
    read(backend, session_id, request).await.into_iter().next()
}

/// 交一批消息时拿改了的：给了房间只看它，`every_room` 看 Agent 在的所有房间，都不给是会话的那个。
pub(crate) async fn changed_notices(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: Option<&str>,
    every_room: bool,
) -> Vec<IpcRoomNotice> {
    let request = IpcReadRoomNoticesRequest {
        room_id: room_id.map(str::to_owned),
        every_room,
        changed_only: true,
    };
    read(backend, session_id, request).await
}

async fn read(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    request: IpcReadRoomNoticesRequest,
) -> Vec<IpcRoomNotice> {
    let method = IpcMethod::WithSession {
        session_id: session_id.to_owned(),
        method: Box::new(IpcMethod::ReadRoomNotices(request)),
    };
    match backend.invoke(method).await {
        Ok(IpcResponse::RoomNotices { notices }) => notices,
        _ => Vec::new(),
    }
}

/// MCP 和命令行给 Agent 看的形状：撤掉的没有正文，写明 `removed`。
#[must_use]
pub fn notice_json(notice: &IpcRoomNotice) -> serde_json::Value {
    let mut value = serde_json::json!(notice);
    if notice.text.is_none() {
        value["removed"] = serde_json::Value::Bool(true);
    }
    value
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_ipc::IpcRoomNotice;

    use super::notice_json;

    #[test]
    fn 撤掉的写明_有正文的照原样() {
        let mut notice = IpcRoomNotice {
            room_id: "!room:matrix.test".to_owned(),
            text: Some("只在被点名时说话。".to_owned()),
            updated_by: "@owner:matrix.test".to_owned(),
            updated_at_unix_ms: 1_000,
        };
        assert_eq!(
            notice_json(&notice),
            serde_json::json!({
                "roomId": "!room:matrix.test",
                "text": "只在被点名时说话。",
                "updatedBy": "@owner:matrix.test",
                "updatedAtUnixMs": 1_000,
            })
        );
        notice.text = None;
        assert_eq!(notice_json(&notice)["removed"], true);
        assert!(notice_json(&notice).get("text").is_none());
    }
}
