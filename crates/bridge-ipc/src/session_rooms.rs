//! 会话的 Agent 再进一个房间、离开一个房间（`specs/agent-identity/design.md` 第 3 步）。两个方法
//! 都包在 `WithSession` 里用：进出的是这个会话的 Agent。

use serde::{Deserialize, Serialize};

use crate::{
    IpcHostRoomTarget,
    host_sessions::{validate_room, validate_session_id},
    tools::{IpcMethodValidationFailure, failure},
};

/// 和开会话时进房间同一套：私人房间带 Matrix 房间，公共大厅只给目录时由服务器分配分片。
/// 不离开已经在的房间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcEnterRoomRequest {
    pub room: IpcHostRoomTarget,
}

impl IpcEnterRoomRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_room(Some(&self.room))
    }
}

/// 离开一个房间：在 Matrix 上离开、私人房间里收回发言级别、服务器放掉它占的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcLeaveRoomRequest {
    pub catalog_id: String,
}

impl IpcLeaveRoomRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.catalog_id).map_err(|_| failure("bridge.ipc.room_invalid"))
    }
}

/// 会话的 Agent 在的一个房间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcSessionRoom {
    pub catalog_id: String,
    /// Matrix 房间 ID：读消息、说话时带的 `roomId`。
    pub room_id: String,
}

/// 看所有房间等消息时，一个房间从哪一条之后开始（第 4 步）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomCursor {
    pub room_id: String,
    pub after_event_id: String,
}

#[cfg(test)]
mod tests {
    use super::{IpcEnterRoomRequest, IpcLeaveRoomRequest, IpcRoomCursor};
    use crate::{IpcHostRoomTarget, IpcListPreviewsRequest, IpcMethod};

    const SESSION_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

    #[test]
    fn 进出房间只能包在会话里用_房间号要像样() {
        let enter = IpcMethod::EnterRoom(IpcEnterRoomRequest {
            room: IpcHostRoomTarget {
                catalog_id: CATALOG_ID.to_owned(),
                room_id: Some("!private:matrix.test".to_owned()),
            },
        });
        let wrapped = IpcMethod::WithSession {
            session_id: SESSION_ID.to_owned(),
            method: Box::new(enter.clone()),
        };
        wrapped.validate().expect("包在会话里的进房间有效");
        assert_eq!(wrapped.name(), "enter_room");
        assert_eq!(
            wrapped.required_scope(),
            agent_room_bridge_core::ipc::IpcScope::HostSessionsManage
        );
        assert_eq!(
            serde_json::to_value(&wrapped).expect("可以序列化"),
            serde_json::json!({
                "with_session": {
                    "sessionId": SESSION_ID,
                    "method": {"enter_room": {"room": {
                        "catalogId": CATALOG_ID, "roomId": "!private:matrix.test"
                    }}}
                }
            })
        );

        let leave = IpcMethod::LeaveRoom(IpcLeaveRoomRequest {
            catalog_id: "not-a-room".to_owned(),
        });
        assert_eq!(
            leave.validate().expect_err("房间号不像样").code(),
            "bridge.ipc.room_invalid"
        );
        let bad_room = IpcMethod::EnterRoom(IpcEnterRoomRequest {
            room: IpcHostRoomTarget {
                catalog_id: CATALOG_ID.to_owned(),
                room_id: Some("#alias:matrix.test".to_owned()),
            },
        });
        assert_eq!(
            bad_room.validate().expect_err("Matrix 房间不像样").code(),
            "bridge.ipc.room_invalid"
        );
    }

    fn cursor(room_id: &str, after_event_id: &str) -> IpcRoomCursor {
        IpcRoomCursor {
            room_id: room_id.to_owned(),
            after_event_id: after_event_id.to_owned(),
        }
    }

    #[test]
    fn 看所有房间时不给单个房间的位置_一个房间一条位置() {
        let every_room = IpcListPreviewsRequest {
            after_event_id: None,
            room_id: None,
            before_event_id: None,
            limit: 20,
            keep_waiting: false,
            wait_ms: None,
            from_ack: true,
            every_room: true,
            room_cursors: vec![cursor("!lobby:matrix.test", "$1:matrix.test")],
        };
        let code = |method: IpcMethod| {
            method
                .validate()
                .map_err(|failure| failure.code().to_owned())
        };
        assert!(code(IpcMethod::WaitInbox(every_room.clone())).is_ok());

        let mut single_room = every_room.clone();
        single_room.room_id = Some("!lobby:matrix.test".to_owned());
        let mut single_cursor = every_room.clone();
        single_cursor.room_cursors.clear();
        single_cursor.after_event_id = Some("$1:matrix.test".to_owned());
        let mut not_every_room = every_room.clone();
        not_every_room.every_room = false;
        let mut same_room_twice = every_room.clone();
        same_room_twice
            .room_cursors
            .push(cursor("!lobby:matrix.test", "$2:matrix.test"));
        let mut newest_with_cursor = every_room;
        newest_with_cursor.from_ack = false;
        for method in [
            IpcMethod::WaitInbox(single_room),
            IpcMethod::ReadInbox(single_cursor),
            IpcMethod::ReadInbox(not_every_room),
            IpcMethod::WaitInbox(same_room_twice),
            // 从新到旧只给最新的一页。
            IpcMethod::ListPreviews(newest_with_cursor),
        ] {
            assert_eq!(
                code(method),
                Err("bridge.ipc.event_cursor_invalid".to_owned())
            );
        }
    }
}
