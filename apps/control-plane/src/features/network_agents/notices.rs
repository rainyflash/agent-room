//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：进房间的回答带这个房间现在的
//! （`notice`），之后改了，等消息的回答带一次（`notices`）。HTTP 和远程 MCP 是同一个形状。
//!
//! 子房间里父房间的须知也算（`specs/redesign/brief-and-pins.md`“子房间里父房间的须知也算”）：进子房间
//! 的回答另带 `parentNotice`，等消息的 `notices` 里父房间那一段的 `roomId` 是父房间的，形状不变。
//!
//! 置顶（“Agent 读置顶”）也在这里一起拿：进房间的回答带这个房间现在的（`pins`），之后改了，等消息的
//! 回答带一次（`pins`，每个房间整份）。形状和本机 MCP 一样。

use agent_room_application::ports::NetworkAgentRoomNotice;
use agent_room_bridge_ipc::IpcRoomPins;
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

/// 进房间时给的：这一间的公告，子房间里还有父房间那一段，再加上这一间现在的置顶。
#[derive(Debug, Default)]
pub(super) struct EnteredNotices {
    pub(super) notice: Option<NoticeResponse>,
    pub(super) parent_notice: Option<NoticeResponse>,
    pub(super) pins: Option<IpcRoomPins>,
}

impl EnteredNotices {
    /// 有没有公告（置顶不算）：有公告时远程 MCP 先说一句公告是房间的规矩。
    pub(super) const fn any(&self) -> bool {
        self.notice.is_some() || self.parent_notice.is_some()
    }
}

/// 进了的房间现在的公告；只敲了门、没有公告或者读不到时没有。
pub(super) async fn entered(
    state: &NetworkAgentHttpState,
    token: &str,
    room: Option<&RoomResponse>,
) -> EnteredNotices {
    let mut entered = EnteredNotices::default();
    let Some(room) = room else {
        return entered;
    };
    for notice in state
        .messaging
        .current_notices(token, &room.matrix_room_id)
        .await
    {
        let slot = if notice.in_sub_room.is_some() {
            &mut entered.parent_notice
        } else {
            &mut entered.notice
        };
        if slot.is_none() {
            *slot = Some(NoticeResponse::from(notice));
        }
    }
    entered.pins = state
        .messaging
        .current_pins(token, &room.matrix_room_id)
        .await;
    entered
}

pub(super) fn changed(notices: Vec<NetworkAgentRoomNotice>) -> Vec<NoticeResponse> {
    notices.into_iter().map(NoticeResponse::from).collect()
}
