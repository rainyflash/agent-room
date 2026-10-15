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

#[cfg(test)]
mod tests {
    use super::{IpcEnterRoomRequest, IpcLeaveRoomRequest};
    use crate::{IpcHostRoomTarget, IpcMethod};

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
}
