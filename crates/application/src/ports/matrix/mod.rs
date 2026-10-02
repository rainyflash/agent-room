mod failure;
mod models;
mod values;

use super::PortFuture;

pub use failure::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixRecoveryAction, MatrixRetryPolicy,
};
pub use models::{
    MatrixAcceptedEvent, MatrixAgentDeviceSessionRequest, MatrixAgentDeviceSessionTarget,
    MatrixAgentUserRegistration, MatrixBackfillPage, MatrixBackfillRequest, MatrixConnection,
    MatrixCreateRoom, MatrixEvent, MatrixLogin, MatrixPowerLevel, MatrixReceipt, MatrixReceiptKind,
    MatrixRoomAccess, MatrixRoomAuthority, MatrixRoomEncryption, MatrixRoomKind,
    MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomStatePosition, MatrixRoomSync,
    MatrixRoomSyncKind, MatrixRoomVisibility, MatrixSession, MatrixSessionMetadata,
    MatrixStateEvent, MatrixSyncBatch, MatrixSyncRequest, MatrixTimelineEncryption,
    MatrixTimelineEvent,
};
pub use values::{
    MatrixAgentLocalpart, MatrixBackfillToken, MatrixDeviceId, MatrixEventId, MatrixEventType,
    MatrixRoomAliasLocalpart, MatrixRoomId, MatrixStateKey, MatrixSyncToken, MatrixTransactionId,
    MatrixUserId, MatrixValueError,
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

/// 撤销 Agent 实例专属的 Matrix 设备会话。
///
/// 实现必须让重复撤销保持幂等，并且不得接受受控 Agent 命名空间之外的用户。
pub trait MatrixAgentDeviceSessionRevoker: Send + Sync {
    fn revoke_device_session<'a>(
        &'a self,
        target: &'a MatrixAgentDeviceSessionTarget,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

/// 为稳定 Agent 实例轮换唯一有效的 Matrix 设备会话。
///
/// 实现必须先幂等撤销目标设备，再签发同一用户和设备标识的新会话。这样同一注册
/// 请求在结果未知后重放时，不会留下两套可同时使用的设备凭据或加密钥匙。
pub trait MatrixAgentDeviceSessionRotator: Send + Sync {
    fn rotate_device_session<'a>(
        &'a self,
        request: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>>;

    /// 换一台设备：幂等撤销 `previous`（连同它上传过的密钥），再为同一用户的新设备 `next`
    /// 签发会话。Synapse 删设备时不删别人给这台设备的交叉签名，同一设备 ID 重新签发后新签名
    /// 会被当成已有而跳过，所以丢了加密存储时要换设备 ID。
    fn replace_device_session<'a>(
        &'a self,
        previous: &'a MatrixAgentDeviceSessionTarget,
        next: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>>;
}

/// 由受限后台工作流擦除一个本地 Matrix 人类账户。
///
/// 实现必须验证目标属于本 Homeserver，并把“已经停用”视为幂等成功。管理员凭据不得暴露给前端。
pub trait MatrixAccountLifecycleGateway: Send + Sync {
    fn deactivate_and_erase<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

/// 替这个本地账户上传新的签名公钥（设备上的自动签名重建签名身份时用，ADR 0011）。
///
/// 已有签名身份时换身份，Synapse 要交互认证；没接 MAS 的部署里只有应用服务的请求能免
/// （MSC4190），所以由控制面以应用服务的身份代传。私钥只在人的设备上，这里只有公钥和签名。
pub trait MatrixCrossSigningResetGateway: Send + Sync {
    fn replace_cross_signing_keys<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keys: &'a MatrixCrossSigningKeys,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

/// 新的签名公钥：Matrix `keys/device_signing/upload` 的正文，不带交互认证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCrossSigningKeys(serde_json::Map<String, serde_json::Value>);

/// 签名公钥不对：不是这几把钥匙、缺主密钥，或者有一把不属于这个账户。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCrossSigningKeys;

impl MatrixCrossSigningKeys {
    const KEYS: [&'static str; 3] = ["master_key", "self_signing_key", "user_signing_key"];

    /// 只收主密钥、自签密钥和用户签名密钥，必须有主密钥，每把都要属于 `owner`。
    ///
    /// # Errors
    ///
    /// 不符合上面任何一条时返回 [`InvalidCrossSigningKeys`]。
    pub fn new(
        body: serde_json::Value,
        owner: &MatrixUserId,
    ) -> Result<Self, InvalidCrossSigningKeys> {
        let serde_json::Value::Object(keys) = body else {
            return Err(InvalidCrossSigningKeys);
        };
        let belongs_to_owner = |value: &serde_json::Value| {
            value
                .as_object()
                .and_then(|key| key.get("user_id"))
                .and_then(serde_json::Value::as_str)
                == Some(owner.as_str())
        };
        if !keys.contains_key("master_key")
            || keys.iter().any(|(name, value)| {
                !Self::KEYS.contains(&name.as_str()) || !belongs_to_owner(value)
            })
        {
            return Err(InvalidCrossSigningKeys);
        }
        Ok(Self(keys))
    }

    pub const fn as_json(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
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

/// 从 Homeserver 当前房间状态读取成员资格和 Power Level。
///
/// 实现不得用本地同步缓存或控制平面投影替代权威状态查询。唯一例外是“房间已加密”：
/// Matrix 不允许关掉加密，见过一次就可以记住。
pub trait MatrixRoomAuthorityGateway: Send + Sync {
    fn inspect_room_authority<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomAuthority>>;

    /// 只查成员资格与加密状态，给读消息、发消息之前的检查用。
    ///
    /// 等消息的 Agent 每秒都会问一次，所以实现可以只发必需的请求；默认实现走完整的
    /// [`Self::inspect_room_authority`]，结果相同。
    fn inspect_room_access<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomAccess>> {
        Box::pin(async move {
            self.inspect_room_authority(room_id, user_id)
                .await
                .map(MatrixRoomAccess::from)
        })
    }
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

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
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

    /// 让这个会话进行中的长轮询同步马上返回。
    ///
    /// 退出时用：同步里有加密存储的写入，不能半途取消，但也不必干等它超时（最长 30 秒）。
    /// 默认什么也不做，调用方照旧等同步自己返回。
    fn wake_sync(&self) -> PortFuture<'_, MatrixResult<()>> {
        Box::pin(async { Ok(()) })
    }

    /// 按事件 ID 重读一条时间线事件，读的时候先试着解密。
    ///
    /// Bridge 用它重读当初解不开、后来拿到房间密钥的消息。默认不支持。
    fn fetch_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<MatrixTimelineEvent>> {
        let _ = (room_id, event_id);
        Box::pin(async {
            Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::NotFound,
            ))
        })
    }

    /// 自上次调用以来导入的、别人重发的会话（房间与会话 ID）；Bridge 据此重读用这些会话加密、
    /// 当初解不开而隔离的消息。
    fn take_recovered_sessions(&self) -> Vec<(MatrixRoomId, String)> {
        Vec::new()
    }

    /// 请发送者那台设备（没点名时是它的每台设备）重发这些会话的房间密钥。
    ///
    /// Bridge 重启后，对存储里仍隔离着的“解不开”的消息重新请求；同一会话一小时内只请求一次。
    /// 默认什么也不做。
    fn request_room_keys(
        &self,
        room_id: &MatrixRoomId,
        sender: &MatrixUserId,
        sender_device: Option<&str>,
        session_ids: &[String],
    ) {
        let _ = (room_id, sender, sender_device, session_ids);
    }
}
