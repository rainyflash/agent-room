//! 子房间里父房间须知的副本（`specs/redesign/brief-and-pins.md` 的“子房间里父房间的须知也算”）：进子房间
//! 的 Agent 不一定在父房间的聊天房间里，读不到父房间的须知，所以控制面以应用服务在每间子房间里写一份。
//!
//! 库里只记哪间子房间还要写（`private_room_parent_notice_sync`），不存须知本身：做的时候读父房间此刻
//! 在聊天服务器上的那一版照着写。

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use crate::persistence::RepositoryResult;

use super::{MatrixResult, MatrixRoomId, PortFuture, RoomNoticeWrite, SubRoomSyncScope};

/// 子房间里那份副本（`io.github.rainyflash.agentroom.room.parent_notice.v1`）：父房间那一版的正文、改的人、
/// 改的时间原样抄过来，再带上父房间的 Matrix 房间号。正文空的是父房间撤掉了须知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentNoticeCopy {
    pub notice: RoomNoticeWrite,
    pub parent_room_id: MatrixRoomId,
}

/// 一间子房间的副本还没照父房间写（`private_room_parent_notice_sync`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentNoticeSyncTask {
    /// 哪一间子房间。一间子房间只排一条。
    pub catalog_id: RoomCatalogId,
    /// 领到的是第几版。做完只删这一版：做的时候又排进来的留着。
    pub generation: i64,
    /// 连这一次，一共领过几次。
    pub attempt: u32,
}

/// 副本的待办。领、做完、没做成往后排，和 [`super::SubRoomStore`] 里子房间跟着父房间变的那一套一样。
pub trait ParentNoticeSyncStore: Send + Sync {
    /// 父房间的须知变了：它下面每间子房间（归档了的也算）排一条，已经排着的重排（版本加一、马上能领、
    /// 从头数）。回排了几间，没有子房间时是 0。
    fn queue_parent_notice_sync(
        &self,
        parent: RoomCatalogId,
        queued_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>>;

    /// 领一批：到了能领的时间的（没领过的、租约过了的、退避到点的），按排进来的先后，最多 `limit` 件。
    /// 领到的在 `lease_until` 之前别人领不到。
    fn claim_parent_notice_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ParentNoticeSyncTask>>>;

    /// 写好了：删掉领到的那一版。做的时候又排进来过的（版本变了，或者已经被别处做完删掉了）再排一次：
    /// 这一处照的可能是旧的那一版，它之后得还有一遍照新的写的。
    fn complete_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 没写成：记下原因，到 `retry_at` 再试。做的时候又排进来过的不记，和做完时一样再排一次。
    fn retry_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}

/// 读写须知在聊天服务器上的那几条房间状态，都以应用服务。
pub trait ParentNoticeGateway: Send + Sync {
    /// 这间房此刻的须知（`room.notice.v1`）。没写过为空；内容读不懂（缺字段、太长、时间不对）的也当
    /// 没有。正文空的是撤掉了，照样回。
    fn room_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<RoomNoticeWrite>>>;

    /// 这间子房间里父房间须知的副本。没写过为空；读不懂的也当没有。
    fn parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<ParentNoticeCopy>>>;

    /// 写这间子房间里的副本，盖掉上一份。和当前一模一样时聊天服务器直接回旧的事件，重写是幂等的。
    fn set_parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        copy: &'a ParentNoticeCopy,
    ) -> PortFuture<'a, MatrixResult<()>>;
}
