//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：进房间时拿这个房间现在的，之后改了，
//! 交一批消息时拿一次（[`crate::InboxWaiter::with_notices`]）。Bridge 记着交过哪一版，同一版只交一次。
//! 拿不到（Bridge 还没准备好）就当没有，不耽误进房间和等消息。
//!
//! 子房间里父房间的须知也算（`specs/redesign/brief-and-pins.md`“子房间里父房间的须知也算”）：Bridge 把
//! 子房间里的副本当成父房间那一段交过来，`roomId` 是父房间的，形状和这一间自己的一样。

use agent_room_bridge_ipc::{IpcMethod, IpcReadRoomNoticesRequest, IpcResponse, IpcRoomNotice};

use crate::BridgeToolClient;

/// 进房间时拿到的须知：这间房自己的（`notice`），子房间里还有父房间的那一段（`parentNotice`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnteredNotices {
    pub notice: Option<IpcRoomNotice>,
    pub parent_notice: Option<IpcRoomNotice>,
}

impl EnteredNotices {
    /// 写进进房间的回答：有的才写。
    pub fn add_to(&self, result: &mut serde_json::Value) {
        if let Some(notice) = &self.notice {
            result["notice"] = notice_json(notice);
        }
        if let Some(parent) = &self.parent_notice {
            result["parentNotice"] = notice_json(parent);
        }
    }
}

/// 进房间时拿这个房间现在的公告，子房间里连父房间的那一段一起。没有公告、撤掉了或者拿不到都是没有。
pub async fn room_notices(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: &str,
) -> EnteredNotices {
    let request = IpcReadRoomNoticesRequest {
        room_id: Some(room_id.to_owned()),
        every_room: false,
        changed_only: false,
    };
    let mut entered = EnteredNotices::default();
    // 按 `roomId` 分：是这间房的就是它自己的，别的是父房间那一段。
    for notice in read(backend, session_id, request).await {
        let slot = if notice.room_id == room_id {
            &mut entered.notice
        } else {
            &mut entered.parent_notice
        };
        if slot.is_none() {
            *slot = Some(notice);
        }
    }
    entered
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

    use super::{EnteredNotices, notice_json};

    #[test]
    fn 进子房间时两段分开写_没有的不写() {
        let notice = |room_id: &str, text: &str| IpcRoomNotice {
            room_id: room_id.to_owned(),
            text: Some(text.to_owned()),
            updated_by: "@owner:matrix.test".to_owned(),
            updated_at_unix_ms: 1_000,
        };
        let mut result = serde_json::json!({});
        EnteredNotices {
            notice: Some(notice("!sub:matrix.test", "只改前端。")),
            parent_notice: Some(notice("!project:matrix.test", "先说要做什么。")),
        }
        .add_to(&mut result);
        assert_eq!(result["notice"]["roomId"], "!sub:matrix.test");
        assert_eq!(result["parentNotice"]["roomId"], "!project:matrix.test");
        assert_eq!(result["parentNotice"]["text"], "先说要做什么。");

        let mut result = serde_json::json!({});
        EnteredNotices::default().add_to(&mut result);
        assert_eq!(result, serde_json::json!({}));
    }

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
