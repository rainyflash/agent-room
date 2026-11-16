//! 房间的置顶写在 Matrix 标准的 `m.room.pinned_events` 里（`specs/redesign/brief-and-pins.md`“置顶”）。
//! 私人房间的 `state_default` 是管理员那一级，只有应用服务够得着，所以由控制面查过“管理”这一位以后
//! 以应用服务写，谁、什么时候置顶的另记在我们自己的字段里。

use agent_room_domain::{
    room_pins::{RoomPinEventId, RoomPins},
    time::UtcMillis,
};

use super::{MatrixFailure, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture};

/// 要置顶的这一句是不是这间房里的一条消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinTarget {
    /// 这间房里的一条消息（加密的也算：服务器看不到里面，只看外面的类型）。
    Message,
    /// 不在这间房里、读不到、是状态事件、不是消息，或者已经删了。
    NotMessage,
}

/// 改一次置顶：哪一句、置顶还是取消、谁、什么时候。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPinsChange {
    pub event_id: RoomPinEventId,
    pub pinned: bool,
    /// 改的人在 Matrix 上的 ID。状态事件是应用服务发的，读的一边按它说“某某置顶了一句”。
    pub actor: MatrixUserId,
    pub at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPinsWriteFailure {
    /// 已经满了，要先取消一条。
    Full,
    Matrix(MatrixFailure),
}

impl From<MatrixFailure> for RoomPinsWriteFailure {
    fn from(failure: MatrixFailure) -> Self {
        Self::Matrix(failure)
    }
}

pub trait PrivateRoomPinsGateway: Send + Sync {
    /// 以应用服务读这一条事件，看它是不是这间房里的一条消息。
    fn pin_target<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a RoomPinEventId,
    ) -> PortFuture<'a, MatrixResult<PinTarget>>;

    /// 读出整份置顶、改一项、变了才整份写回，回改完的列表。Matrix 没有按版本写的办法，同一个房间
    /// 一次只改一个，不然两个人同时置顶，后写的会盖掉先置顶的那条。
    fn change_pins<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        change: &'a RoomPinsChange,
    ) -> PortFuture<'a, Result<RoomPins, RoomPinsWriteFailure>>;
}
