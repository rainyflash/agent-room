//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”，IPC 4.6 起）：Agent 进房间时拿现在的，
//! 之后改了，交一批消息时再拿一次。Bridge 记着交过哪一版，同一版不交第二次。
//!
//! 单独一个方法，不加进等消息的回答：回答对多出来的字段严格校验，升级以后宿主里还开着的旧版
//! MCP 进程会整个读不出来。

use serde::{Deserialize, Serialize};

use crate::limits;
use crate::tools::{IpcMethodValidationFailure, failure, validate_optional_bounded};

/// 一次最多交几个房间的公告。一条最多 2000 个字，四条放得进一帧；剩下的下次再交。
pub const ROOM_NOTICES_PER_RESPONSE: usize = 4;

/// 读公告（`ReadRoomNotices`），包在 `WithSession` 里用。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcReadRoomNoticesRequest {
    /// 看哪个房间；不给时是会话的那一个，和读收件箱一样。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// 看 Agent 在的所有房间，会话的那个在前；这时不给 `roomId`。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub every_room: bool,
    /// 只要这个 Agent 还没拿到过的那一版（交一批消息时用，撤掉也算一次改动）；不设时要现在的
    /// （进房间时用，没有公告或撤掉了就不给）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed_only: bool,
}

impl IpcReadRoomNoticesRequest {
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

/// 一个房间的公告。房主和管理员写的这间房的规矩，不加密，服务器读得到。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomNotice {
    /// Matrix 房间 ID。
    pub room_id: String,
    /// 公告正文；撤掉了没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 改的人在 Matrix 上的 ID。
    pub updated_by: String,
    pub updated_at_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::IpcReadRoomNoticesRequest;
    use crate::IpcMethod;

    #[test]
    fn 不设的字段不写出去_房间号要像样() {
        let request = IpcReadRoomNoticesRequest::default();
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({})
        );
        assert!(IpcMethod::ReadRoomNotices(request).validate().is_ok());
        let changed = IpcReadRoomNoticesRequest {
            every_room: true,
            changed_only: true,
            ..IpcReadRoomNoticesRequest::default()
        };
        assert_eq!(
            serde_json::to_value(&changed).unwrap(),
            serde_json::json!({"everyRoom": true, "changedOnly": true})
        );
        for bad in [
            IpcReadRoomNoticesRequest {
                room_id: Some("坏\n的".to_owned()),
                ..IpcReadRoomNoticesRequest::default()
            },
            // 看所有房间时不给房间。
            IpcReadRoomNoticesRequest {
                room_id: Some("!room:matrix.test".to_owned()),
                every_room: true,
                changed_only: false,
            },
        ] {
            assert_eq!(
                IpcMethod::ReadRoomNotices(bad)
                    .validate()
                    .map_err(crate::tools::IpcMethodValidationFailure::code),
                Err("bridge.ipc.room_id_invalid")
            );
        }
    }
}
