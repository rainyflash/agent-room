//! 找公开房间（`specs/public-rooms/design.md` 5c，IPC 4.6 起）：本机 MCP 和命令行经 Bridge 翻发现页的
//! 目录，不要会话。本机 Agent 跟着主人进，所以每间房带上这台电脑的账号是不是已经在里面了。

use serde::{Deserialize, Serialize};

use crate::tools::{IpcMethodValidationFailure, failure};

/// 搜的字最多这么多个，和发现页一样。
pub const PUBLIC_ROOM_SEARCH_CHARACTERS: usize = 100;
/// 翻页的记号是控制面给的一个数，最多这么多位。
const CURSOR_DIGITS: usize = 16;

/// 翻一页公开房间（`ListPublicRooms`）。`cursor` 照上一页的 `nextCursor` 原样带回。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcListPublicRoomsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl IpcListPublicRoomsRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if let Some(search) = &self.search
            && (search.chars().count() > PUBLIC_ROOM_SEARCH_CHARACTERS
                || search.chars().any(char::is_control))
        {
            return Err(failure("bridge.ipc.public_room_search_invalid"));
        }
        if let Some(cursor) = &self.cursor
            && (cursor.is_empty()
                || cursor.len() > CURSOR_DIGITS
                || !cursor.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(failure("bridge.ipc.public_room_cursor_invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcPublicRoomJoinMode {
    /// 账号点“加入”就进去了。
    Open,
    /// 要申请，房主或管理员批准。
    Request,
    /// 只能由房主邀请。
    InviteOnly,
}

/// 发现页上的一间公开房间。`joined` 时本机 Agent 拿 `name` 或分享链接就能进；没加入的要请主人
/// 先打开 `shareUrl` 加入。房间名、简介和房主的昵称是人写的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPublicRoom {
    pub catalog_id: String,
    pub name: String,
    pub description: String,
    pub owner_name: String,
    pub join_mode: IpcPublicRoomJoinMode,
    pub end_to_end_encrypted: bool,
    pub member_count: u32,
    pub agent_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at_unix_ms: Option<i64>,
    pub share_url: String,
    pub joined: bool,
}

#[cfg(test)]
mod tests {
    use super::IpcListPublicRoomsRequest;

    fn request(search: Option<&str>, cursor: Option<&str>) -> IpcListPublicRoomsRequest {
        IpcListPublicRoomsRequest {
            search: search.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
        }
    }

    #[test]
    fn 搜的字和翻页记号都有上限() {
        assert!(request(None, None).validate().is_ok());
        assert!(request(Some("读书"), Some("24")).validate().is_ok());
        assert!(request(Some(&"书".repeat(100)), None).validate().is_ok());
        for (search, cursor, code) in [
            (
                Some("书".repeat(101)),
                None,
                "bridge.ipc.public_room_search_invalid",
            ),
            (
                Some("a\nb".to_owned()),
                None,
                "bridge.ipc.public_room_search_invalid",
            ),
            (None, Some(""), "bridge.ipc.public_room_cursor_invalid"),
            (None, Some("-1"), "bridge.ipc.public_room_cursor_invalid"),
            (
                None,
                Some("12345678901234567"),
                "bridge.ipc.public_room_cursor_invalid",
            ),
        ] {
            let failure = request(search.as_deref(), cursor)
                .validate()
                .expect_err("不收");
            assert_eq!(failure.code(), code);
        }
    }

    #[test]
    fn 请求不收多出来的字段() {
        assert!(
            serde_json::from_str::<IpcListPublicRoomsRequest>(r#"{"search":"a","sort":"new"}"#)
                .is_err()
        );
    }
}
