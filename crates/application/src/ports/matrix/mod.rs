mod failure;
mod models;
mod values;

use super::PortFuture;

pub use failure::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixRecoveryAction, MatrixRetryPolicy,
};
pub use models::{
    MatrixAcceptedEvent, MatrixAgentDeviceSessionRequest, MatrixAgentUserRegistration,
    MatrixBackfillPage, MatrixBackfillRequest, MatrixConnection, MatrixCreateRoom, MatrixEvent,
    MatrixLogin, MatrixReceipt, MatrixReceiptKind, MatrixRoomPreset, MatrixRoomSync,
    MatrixRoomSyncKind, MatrixRoomVisibility, MatrixSession, MatrixSessionMetadata,
    MatrixStateEvent, MatrixSyncBatch, MatrixSyncRequest, MatrixTimelineEvent,
};
pub use values::{
    MatrixAgentLocalpart, MatrixBackfillToken, MatrixDeviceId, MatrixEventId, MatrixEventType,
    MatrixRoomId, MatrixStateKey, MatrixSyncToken, MatrixTransactionId, MatrixUserId,
    MatrixValueError,
};

pub type MatrixResult<T> = Result<T, MatrixFailure>;

/// 通过受控 Matrix Application Service 命名空间管理 Agent 用户和设备会话。
///
/// 实现不得把 Application Service Token 下发给 Bridge 或任何前端。
pub trait MatrixAgentIdentityProvisioner: Send + Sync {
    fn ensure_user<'a>(
        &'a self,
        registration: &'a MatrixAgentUserRegistration,
    ) -> PortFuture<'a, MatrixResult<MatrixUserId>>;

    fn issue_device_session<'a>(
        &'a self,
        request: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>>;
}

/// 创建或恢复一个与单个 Matrix 设备绑定的客户端。
pub trait MatrixClientFactory: Send + Sync {
    fn login<'a>(
        &'a self,
        login: &'a MatrixLogin,
    ) -> PortFuture<'a, MatrixResult<MatrixConnection>>;

    fn restore<'a>(
        &'a self,
        session: &'a MatrixSession,
    ) -> PortFuture<'a, MatrixResult<MatrixConnection>>;
}

/// 已认证 Matrix 会话的协议无关能力端口。
///
/// 实现必须只调用标准 Matrix API，不得读取 Homeserver 内部数据库。
pub trait MatrixGateway: Send + Sync {
    fn metadata(&self) -> &MatrixSessionMetadata;

    fn sync_once<'a>(
        &'a self,
        request: &'a MatrixSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>>;

    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>>;

    fn invite<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn join<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>>;

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>>;

    fn send_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>>;

    fn send_state_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>>;

    fn send_receipt<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        receipt: &'a MatrixReceipt,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn backfill<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>>;
}
