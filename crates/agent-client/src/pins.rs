//! 房间的置顶（`specs/redesign/brief-and-pins.md`“Agent 读置顶”）：进房间时拿这个房间现在的，之后改了，
//! 交一批消息时拿一次整份（跟着 [`crate::InboxWaiter::with_notices`]）。置顶改了不叫醒 Agent。Bridge 记着
//! 交过哪一版，同一版只交一次。拿不到（Bridge 还没准备好）就当没有，不耽误进房间和等消息。

use agent_room_bridge_ipc::{IpcMethod, IpcReadRoomPinsRequest, IpcResponse, IpcRoomPins};

use crate::BridgeToolClient;

/// 进房间时拿这个房间现在的置顶。没有置顶、全取消了或者拿不到都是没有。
pub(crate) async fn room_pins(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: &str,
) -> Option<IpcRoomPins> {
    let request = IpcReadRoomPinsRequest {
        room_id: Some(room_id.to_owned()),
        every_room: false,
        changed_only: false,
    };
    read(backend, session_id, request)
        .await
        .into_iter()
        .find(|pins| pins.room_id == room_id)
}

/// 交一批消息时拿改了的（全取消了也算）：看的房间和读改了的公告一样。
pub(crate) async fn changed_pins(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    room_id: Option<&str>,
    every_room: bool,
) -> Vec<IpcRoomPins> {
    let request = IpcReadRoomPinsRequest {
        room_id: room_id.map(str::to_owned),
        every_room,
        changed_only: true,
    };
    read(backend, session_id, request).await
}

async fn read(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    request: IpcReadRoomPinsRequest,
) -> Vec<IpcRoomPins> {
    let method = IpcMethod::WithSession {
        session_id: session_id.to_owned(),
        method: Box::new(IpcMethod::ReadRoomPins(request)),
    };
    match backend.invoke(method).await {
        Ok(IpcResponse::RoomPins { rooms }) => rooms,
        _ => Vec::new(),
    }
}
