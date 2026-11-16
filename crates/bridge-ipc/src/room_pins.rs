//! 房间的置顶（`specs/redesign/brief-and-pins.md`“Agent 读置顶”，IPC 4.8 起）：Agent 进房间时拿现在的，
//! 之后改了，交一批消息时再拿一次整份。Bridge 记着交过哪一版，同一版不交第二次。
//!
//! 单独一个方法、回答的格式新定，不往公告的回答里塞：回答对多出来的字段严格校验，升级以后宿主里
//! 还开着的旧版 MCP 进程会整个读不出来。

use serde::{Deserialize, Serialize};

use crate::IpcReplyExcerpt;
use crate::limits;
use crate::tools::{IpcMethodValidationFailure, failure, validate_optional_bounded};

/// 一次最多交几个房间的置顶。一间房最多 5 条，每条只带开头一小段；剩下的下次再交。
pub const ROOM_PINS_PER_RESPONSE: usize = 4;

/// 读置顶（`ReadRoomPins`），包在 `WithSession` 里用。和读公告一样的三个参数。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcReadRoomPinsRequest {
    /// 看哪个房间；不给时是会话的那一个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// 看 Agent 在的所有房间，会话的那个在前；这时不给 `roomId`。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub every_room: bool,
    /// 只要这个 Agent 还没拿到过的那一版（交一批消息时用，全取消了也算一次改动）；不设时要现在的
    /// （进房间时用，没有置顶就不给）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed_only: bool,
}

impl IpcReadRoomPinsRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if self.every_room && self.room_id.is_some() {
            return Err(failure("bridge.ipc.room_id_invalid"));
        }
        validate_optional_bounded(
            self.room_id.as_deref(),
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )
    }
}

/// 一个房间此刻的置顶，整份，新的在最前面；全取消了是空的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomPins {
    /// Matrix 房间 ID。
    pub room_id: String,
    pub pinned: Vec<IpcRoomPin>,
    /// 最近一次改置顶的人在 Matrix 上的 ID。
    pub updated_by: String,
    pub updated_at_unix_ms: i64,
}

/// 置顶的一句。服务器上只记事件 ID：那句是谁说的、开头写了什么，照读的人手边已经有、解得开的那一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomPin {
    pub event_id: String,
    /// 置顶的人在 Matrix 上的 ID。
    pub pinned_by: String,
    pub pinned_at_unix_ms: i64,
    /// 那一句：消息 ID、谁说的、开头；全文按消息 ID 取。手边没有时不给，写明为什么。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<IpcReplyExcerpt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<IpcPinUnreadable>,
}

/// 置顶的那一句为什么没有内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcPinUnreadable {
    /// 这台电脑上还没有这一句（多半是加入以前的、还没往前翻到），往前翻也许能翻到。
    NotLoaded,
    /// 进来以前的加密消息，读不到（网络 Agent）。
    BeforeJoin,
}

#[cfg(test)]
mod tests {
    use super::{IpcPinUnreadable, IpcReadRoomPinsRequest, IpcRoomPin};
    use crate::{IpcMethod, IpcReplyExcerpt};

    #[test]
    fn 不设的字段不写出去_房间号要像样() {
        let request = IpcReadRoomPinsRequest::default();
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({})
        );
        assert!(IpcMethod::ReadRoomPins(request).validate().is_ok());
        let bad = IpcReadRoomPinsRequest {
            room_id: Some("!room:matrix.test".to_owned()),
            every_room: true,
            changed_only: false,
        };
        assert_eq!(
            IpcMethod::ReadRoomPins(bad)
                .validate()
                .map_err(crate::tools::IpcMethodValidationFailure::code),
            Err("bridge.ipc.room_id_invalid")
        );
    }

    #[test]
    fn 有内容的带上那一句_没有的写明为什么() {
        let read = IpcRoomPin {
            event_id: "$a:matrix.test".to_owned(),
            pinned_by: "@owner:matrix.test".to_owned(),
            pinned_at_unix_ms: 1_000,
            message: Some(IpcReplyExcerpt {
                message_id: "0193a1b2-0000-7000-8000-000000000001".to_owned(),
                actor_name: "Ada".to_owned(),
                excerpt: "周五发版。".to_owned(),
            }),
            unreadable: None,
        };
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::json!({
                "eventId": "$a:matrix.test",
                "pinnedBy": "@owner:matrix.test",
                "pinnedAtUnixMs": 1_000,
                "message": {
                    "messageId": "0193a1b2-0000-7000-8000-000000000001",
                    "actorName": "Ada",
                    "excerpt": "周五发版。",
                },
            })
        );
        let unread = IpcRoomPin {
            message: None,
            unreadable: Some(IpcPinUnreadable::BeforeJoin),
            ..read
        };
        assert_eq!(
            serde_json::to_value(&unread).unwrap()["unreadable"],
            "before_join"
        );
    }
}
