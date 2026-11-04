//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：进房间的回答带这个房间现在的
//! （`notice`），之后改了，等消息的回答带一次（`notices`）。HTTP 和远程 MCP 是同一个形状。

use agent_room_application::ports::NetworkAgentRoomNotice;
use serde::Serialize;

use super::{NetworkAgentHttpState, RoomResponse};

/// 一个房间的公告：房主和管理员写的这间房的规矩。撤掉了没有正文，写明 `removed`。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NoticeResponse {
    room_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    removed: bool,
    /// 改的人在 Matrix 上的 ID。
    updated_by: String,
    updated_at_unix_ms: i64,
}

impl From<NetworkAgentRoomNotice> for NoticeResponse {
    fn from(notice: NetworkAgentRoomNotice) -> Self {
        Self {
            room_id: notice.room_id.as_str().to_owned(),
            removed: notice.text.is_none(),
            text: notice.text,
            updated_by: notice.updated_by,
            updated_at_unix_ms: notice.updated_at_ms,
        }
    }
}

/// 进了的房间现在的公告；只敲了门、没有公告或者读不到时没有。
pub(super) async fn entered(
    state: &NetworkAgentHttpState,
    token: &str,
    room: Option<&RoomResponse>,
) -> Option<NoticeResponse> {
    let room = room?;
    state
        .messaging
        .current_notice(token, &room.matrix_room_id)
        .await
        .map(NoticeResponse::from)
}

pub(super) fn changed(notices: Vec<NetworkAgentRoomNotice>) -> Vec<NoticeResponse> {
    notices.into_iter().map(NoticeResponse::from).collect()
}
