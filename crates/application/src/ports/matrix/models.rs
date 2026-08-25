use std::{fmt, num::NonZeroU16, sync::Arc};

use agent_room_domain::{DomainError, DomainResult, time::DurationMillis};
use serde_json::Value;

use crate::ports::SecretValue;

use super::{
    MatrixBackfillToken, MatrixDeviceId, MatrixEventId, MatrixEventType, MatrixGateway,
    MatrixRoomAliasLocalpart, MatrixRoomAuthorityGateway, MatrixRoomId, MatrixStateKey,
    MatrixSyncToken, MatrixTransactionId, MatrixUserId,
};

const MAX_LOGIN_ID_LENGTH: usize = 512;
const MAX_DISPLAY_NAME_LENGTH: usize = 128;
const MAX_ROOM_NAME_LENGTH: usize = 255;
const MAX_ROOM_TOPIC_LENGTH: usize = 2_048;
const MAX_EVENT_PAYLOAD_BYTES: usize = 65_536;
const MAX_ROOM_INVITES: usize = 250;
const MAX_SYNC_TIMEOUT_MILLIS: u64 = 60_000;
const MAX_BACKFILL_EVENTS: u16 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixAgentUserRegistration {
    localpart: super::MatrixAgentLocalpart,
}

impl MatrixAgentUserRegistration {
    pub const fn new(localpart: super::MatrixAgentLocalpart) -> Self {
        Self { localpart }
    }

    pub const fn localpart(&self) -> &super::MatrixAgentLocalpart {
        &self.localpart
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixAgentDeviceSessionRequest {
    user_id: MatrixUserId,
    device_id: MatrixDeviceId,
    initial_device_display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixAgentDeviceSessionTarget {
    user_id: MatrixUserId,
    device_id: MatrixDeviceId,
}

impl MatrixAgentDeviceSessionTarget {
    pub const fn new(user_id: MatrixUserId, device_id: MatrixDeviceId) -> Self {
        Self { user_id, device_id }
    }

    pub const fn user_id(&self) -> &MatrixUserId {
        &self.user_id
    }

    pub const fn device_id(&self) -> &MatrixDeviceId {
        &self.device_id
    }
}

impl MatrixAgentDeviceSessionRequest {
    /// 创建 Agent 实例的 Matrix 设备会话请求。
    ///
    /// # Errors
    ///
    /// 设备显示名为空、超长或包含控制字符时失败。
    pub fn new(
        user_id: MatrixUserId,
        device_id: MatrixDeviceId,
        initial_device_display_name: String,
    ) -> DomainResult<Self> {
        validate_text(
            "matrix_device_display_name",
            &initial_device_display_name,
            MAX_DISPLAY_NAME_LENGTH,
            false,
        )?;
        Ok(Self {
            user_id,
            device_id,
            initial_device_display_name,
        })
    }

    pub const fn user_id(&self) -> &MatrixUserId {
        &self.user_id
    }

    pub const fn device_id(&self) -> &MatrixDeviceId {
        &self.device_id
    }

    pub fn initial_device_display_name(&self) -> &str {
        &self.initial_device_display_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixLogin {
    login_id: String,
    password: SecretValue,
    device_id: Option<MatrixDeviceId>,
    initial_device_display_name: Option<String>,
}

impl MatrixLogin {
    /// 创建密码登录请求。密码只允许通过敏感值容器进入边界。
    ///
    /// # Errors
    ///
    /// 登录标识或设备显示名包含控制字符、为空或超长时返回校验错误。
    pub fn new(
        login_id: impl Into<String>,
        password: SecretValue,
        device_id: Option<MatrixDeviceId>,
        initial_device_display_name: Option<String>,
    ) -> DomainResult<Self> {
        let login_id = login_id.into();
        validate_text("matrix_login_id", &login_id, MAX_LOGIN_ID_LENGTH, false)?;
        if let Some(display_name) = initial_device_display_name.as_deref() {
            validate_text(
                "matrix_device_display_name",
                display_name,
                MAX_DISPLAY_NAME_LENGTH,
                false,
            )?;
        }
        Ok(Self {
            login_id,
            password,
            device_id,
            initial_device_display_name,
        })
    }

    pub fn login_id(&self) -> &str {
        &self.login_id
    }

    pub const fn password(&self) -> &SecretValue {
        &self.password
    }

    pub const fn device_id(&self) -> Option<&MatrixDeviceId> {
        self.device_id.as_ref()
    }

    pub fn initial_device_display_name(&self) -> Option<&str> {
        self.initial_device_display_name.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSessionMetadata {
    user_id: MatrixUserId,
    device_id: MatrixDeviceId,
}

impl MatrixSessionMetadata {
    pub const fn new(user_id: MatrixUserId, device_id: MatrixDeviceId) -> Self {
        Self { user_id, device_id }
    }

    pub const fn user_id(&self) -> &MatrixUserId {
        &self.user_id
    }

    pub const fn device_id(&self) -> &MatrixDeviceId {
        &self.device_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSession {
    metadata: MatrixSessionMetadata,
    access_token: SecretValue,
    refresh_token: Option<SecretValue>,
}

impl MatrixSession {
    pub const fn new(
        metadata: MatrixSessionMetadata,
        access_token: SecretValue,
        refresh_token: Option<SecretValue>,
    ) -> Self {
        Self {
            metadata,
            access_token,
            refresh_token,
        }
    }

    pub const fn metadata(&self) -> &MatrixSessionMetadata {
        &self.metadata
    }

    pub const fn access_token(&self) -> &SecretValue {
        &self.access_token
    }

    pub const fn refresh_token(&self) -> Option<&SecretValue> {
        self.refresh_token.as_ref()
    }
}

pub struct MatrixConnection {
    session: MatrixSession,
    gateway: Arc<dyn MatrixGateway>,
    room_authority_gateway: Arc<dyn MatrixRoomAuthorityGateway>,
}

impl MatrixConnection {
    pub const fn from_parts(
        session: MatrixSession,
        gateway: Arc<dyn MatrixGateway>,
        room_authority_gateway: Arc<dyn MatrixRoomAuthorityGateway>,
    ) -> Self {
        Self {
            session,
            gateway,
            room_authority_gateway,
        }
    }

    pub const fn session(&self) -> &MatrixSession {
        &self.session
    }

    pub fn gateway(&self) -> &dyn MatrixGateway {
        self.gateway.as_ref()
    }

    pub fn gateway_handle(&self) -> Arc<dyn MatrixGateway> {
        Arc::clone(&self.gateway)
    }

    pub fn room_authority_gateway(&self) -> &dyn MatrixRoomAuthorityGateway {
        self.room_authority_gateway.as_ref()
    }

    pub fn room_authority_gateway_handle(&self) -> Arc<dyn MatrixRoomAuthorityGateway> {
        Arc::clone(&self.room_authority_gateway)
    }

    pub fn into_parts(
        self,
    ) -> (
        MatrixSession,
        Arc<dyn MatrixGateway>,
        Arc<dyn MatrixRoomAuthorityGateway>,
    ) {
        (self.session, self.gateway, self.room_authority_gateway)
    }
}

impl fmt::Debug for MatrixConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixConnection")
            .field("metadata", self.session.metadata())
            .finish_non_exhaustive()
    }
}

/// Matrix 用户在房间中的有效 Power Level。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixPowerLevel {
    Finite(i64),
    Infinite,
}

impl MatrixPowerLevel {
    pub const fn finite(value: i64) -> Self {
        Self::Finite(value)
    }

    pub const fn is_at_least(self, minimum: i64) -> bool {
        match self {
            Self::Finite(value) => value >= minimum,
            Self::Infinite => true,
        }
    }

    pub const fn satisfies(self, required: Self) -> bool {
        match (self, required) {
            (Self::Infinite, _) => true,
            (Self::Finite(_), Self::Infinite) => false,
            (Self::Finite(actual), Self::Finite(minimum)) => actual >= minimum,
        }
    }
}

/// 单次权威状态查询得到的内容读取授权证据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixRoomAuthority {
    joined: bool,
    power_level: MatrixPowerLevel,
    message_send_power_level: MatrixPowerLevel,
}

impl MatrixRoomAuthority {
    pub const fn joined(power_level: MatrixPowerLevel) -> Self {
        Self {
            joined: true,
            power_level,
            message_send_power_level: MatrixPowerLevel::Finite(0),
        }
    }

    pub const fn joined_with_message_threshold(
        power_level: MatrixPowerLevel,
        message_send_power_level: MatrixPowerLevel,
    ) -> Self {
        Self {
            joined: true,
            power_level,
            message_send_power_level,
        }
    }

    pub const fn not_joined() -> Self {
        Self {
            joined: false,
            power_level: MatrixPowerLevel::Finite(0),
            message_send_power_level: MatrixPowerLevel::Finite(0),
        }
    }

    pub const fn is_joined(self) -> bool {
        self.joined
    }

    pub const fn power_level(self) -> MatrixPowerLevel {
        self.power_level
    }

    pub const fn message_send_power_level(self) -> MatrixPowerLevel {
        self.message_send_power_level
    }

    pub const fn can_send_messages(self) -> bool {
        self.joined && self.power_level.satisfies(self.message_send_power_level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRoomVisibility {
    Private,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRoomPreset {
    PrivateChat,
    PublicChat,
    TrustedPrivateChat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRoomKind {
    Conversation,
    Space,
}

/// Matrix 房间正文的传输保密策略。
///
/// 公共大厅允许服务端治理，因此默认保持明文；私人房间和直接会话必须显式选择端到端加密。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatrixRoomEncryption {
    #[default]
    Unencrypted,
    EndToEnd,
}

/// Matrix 建房时采用的协议权限基线。
///
/// 产品权限仍由 Agent Room 自己持久化；该配置只负责把 Matrix 收紧为不可绕过的硬边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatrixRoomPowerProfile {
    #[default]
    Standard,
    ManagedPrivate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCreateRoom {
    name: Option<String>,
    topic: Option<String>,
    visibility: MatrixRoomVisibility,
    preset: MatrixRoomPreset,
    direct: bool,
    invite: Vec<MatrixUserId>,
    kind: MatrixRoomKind,
    encryption: MatrixRoomEncryption,
    power_profile: MatrixRoomPowerProfile,
    alias_localpart: Option<MatrixRoomAliasLocalpart>,
    member_writable_state_event_types: Vec<MatrixEventType>,
}

impl MatrixCreateRoom {
    /// 创建标准 Matrix 房间参数。
    ///
    /// # Errors
    ///
    /// 文本字段或初始邀请人数超出边界时返回校验错误。
    pub fn new(
        name: Option<String>,
        topic: Option<String>,
        visibility: MatrixRoomVisibility,
        preset: MatrixRoomPreset,
        direct: bool,
        invite: Vec<MatrixUserId>,
    ) -> DomainResult<Self> {
        if let Some(name) = name.as_deref() {
            validate_text("matrix_room_name", name, MAX_ROOM_NAME_LENGTH, false)?;
        }
        if let Some(topic) = topic.as_deref() {
            validate_text("matrix_room_topic", topic, MAX_ROOM_TOPIC_LENGTH, true)?;
        }
        if invite.len() > MAX_ROOM_INVITES {
            return Err(DomainError::Validation {
                field: "matrix_room_invites",
                reason: "初始邀请人数不能超过 250",
            });
        }
        Ok(Self {
            name,
            topic,
            visibility,
            preset,
            direct,
            invite,
            kind: MatrixRoomKind::Conversation,
            encryption: MatrixRoomEncryption::Unencrypted,
            power_profile: MatrixRoomPowerProfile::Standard,
            alias_localpart: None,
            member_writable_state_event_types: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_kind(mut self, kind: MatrixRoomKind) -> Self {
        self.kind = kind;
        self
    }

    #[must_use]
    pub fn with_end_to_end_encryption(mut self) -> Self {
        self.encryption = MatrixRoomEncryption::EndToEnd;
        self
    }

    #[must_use]
    pub fn with_power_profile(mut self, power_profile: MatrixRoomPowerProfile) -> Self {
        self.power_profile = power_profile;
        self
    }

    #[must_use]
    pub fn with_alias_localpart(mut self, alias_localpart: MatrixRoomAliasLocalpart) -> Self {
        self.alias_localpart = Some(alias_localpart);
        self
    }

    /// 允许普通已加入成员写入指定状态事件类型。
    #[must_use]
    pub fn with_member_writable_state_event_type(mut self, event_type: MatrixEventType) -> Self {
        if !self.member_writable_state_event_types.contains(&event_type) {
            self.member_writable_state_event_types.push(event_type);
        }
        self
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn topic(&self) -> Option<&str> {
        self.topic.as_deref()
    }

    pub const fn visibility(&self) -> MatrixRoomVisibility {
        self.visibility
    }

    pub const fn preset(&self) -> MatrixRoomPreset {
        self.preset
    }

    pub const fn direct(&self) -> bool {
        self.direct
    }

    pub fn invite(&self) -> &[MatrixUserId] {
        &self.invite
    }

    pub const fn kind(&self) -> MatrixRoomKind {
        self.kind
    }

    pub const fn encryption(&self) -> MatrixRoomEncryption {
        self.encryption
    }

    pub const fn power_profile(&self) -> MatrixRoomPowerProfile {
        self.power_profile
    }

    pub const fn alias_localpart(&self) -> Option<&MatrixRoomAliasLocalpart> {
        self.alias_localpart.as_ref()
    }

    pub fn member_writable_state_event_types(&self) -> &[MatrixEventType] {
        &self.member_writable_state_event_types
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixEvent {
    event_type: MatrixEventType,
    transaction_id: MatrixTransactionId,
    content: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixStateEvent {
    event_type: MatrixEventType,
    state_key: MatrixStateKey,
    content: Value,
}

impl MatrixStateEvent {
    /// 创建依赖状态键覆盖语义的 Matrix State Event。
    ///
    /// # Errors
    ///
    /// 内容不是对象或序列化后超过 64 KiB 时返回校验错误。
    pub fn new(
        event_type: MatrixEventType,
        state_key: MatrixStateKey,
        content: Value,
    ) -> DomainResult<Self> {
        validate_event_content(&content)?;
        Ok(Self {
            event_type,
            state_key,
            content,
        })
    }

    pub const fn event_type(&self) -> &MatrixEventType {
        &self.event_type
    }

    pub const fn state_key(&self) -> &MatrixStateKey {
        &self.state_key
    }

    pub const fn content(&self) -> &Value {
        &self.content
    }
}

impl MatrixEvent {
    /// 创建带稳定事务标识的消息型事件。
    ///
    /// # Errors
    ///
    /// 内容不是对象或序列化后超过 64 KiB 时返回校验错误。
    pub fn new(
        event_type: MatrixEventType,
        transaction_id: MatrixTransactionId,
        content: Value,
    ) -> DomainResult<Self> {
        validate_event_content(&content)?;
        Ok(Self {
            event_type,
            transaction_id,
            content,
        })
    }

    pub const fn event_type(&self) -> &MatrixEventType {
        &self.event_type
    }

    pub const fn transaction_id(&self) -> &MatrixTransactionId {
        &self.transaction_id
    }

    pub const fn content(&self) -> &Value {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixAcceptedEvent {
    transaction_id: MatrixTransactionId,
    event_id: MatrixEventId,
}

impl MatrixAcceptedEvent {
    pub const fn new(transaction_id: MatrixTransactionId, event_id: MatrixEventId) -> Self {
        Self {
            transaction_id,
            event_id,
        }
    }

    pub const fn transaction_id(&self) -> &MatrixTransactionId {
        &self.transaction_id
    }

    pub const fn event_id(&self) -> &MatrixEventId {
        &self.event_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixReceiptKind {
    Read,
    PrivateRead,
    FullyRead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixReceipt {
    kind: MatrixReceiptKind,
    event_id: MatrixEventId,
}

impl MatrixReceipt {
    pub const fn new(kind: MatrixReceiptKind, event_id: MatrixEventId) -> Self {
        Self { kind, event_id }
    }

    pub const fn kind(&self) -> MatrixReceiptKind {
        self.kind
    }

    pub const fn event_id(&self) -> &MatrixEventId {
        &self.event_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSyncRequest {
    since: Option<MatrixSyncToken>,
    timeout: DurationMillis,
    full_state: bool,
}

impl MatrixSyncRequest {
    /// 创建一次有界长轮询同步请求。
    ///
    /// # Errors
    ///
    /// 超时大于 60 秒时返回校验错误。
    pub fn new(
        since: Option<MatrixSyncToken>,
        timeout: DurationMillis,
        full_state: bool,
    ) -> DomainResult<Self> {
        if timeout.value() > MAX_SYNC_TIMEOUT_MILLIS {
            return Err(DomainError::Validation {
                field: "matrix_sync_timeout",
                reason: "不能超过 60 秒",
            });
        }
        Ok(Self {
            since,
            timeout,
            full_state,
        })
    }

    pub const fn since(&self) -> Option<&MatrixSyncToken> {
        self.since.as_ref()
    }

    pub const fn timeout(&self) -> DurationMillis {
        self.timeout
    }

    pub const fn full_state(&self) -> bool {
        self.full_state
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRoomSyncKind {
    Joined,
    Invited,
    Left,
    Knocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRoomStatePosition {
    BeforeTimeline,
    AfterTimeline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRoomSync {
    room_id: MatrixRoomId,
    kind: MatrixRoomSyncKind,
    timeline_limited: bool,
    previous_batch: Option<MatrixBackfillToken>,
    timeline: Vec<MatrixTimelineEvent>,
    state: Vec<MatrixTimelineEvent>,
    state_position: MatrixRoomStatePosition,
}

impl MatrixRoomSync {
    pub const fn new(
        room_id: MatrixRoomId,
        kind: MatrixRoomSyncKind,
        timeline_limited: bool,
        previous_batch: Option<MatrixBackfillToken>,
        timeline: Vec<MatrixTimelineEvent>,
        state: Vec<MatrixTimelineEvent>,
    ) -> Self {
        Self {
            room_id,
            kind,
            timeline_limited,
            previous_batch,
            timeline,
            state,
            state_position: MatrixRoomStatePosition::BeforeTimeline,
        }
    }

    #[must_use]
    pub const fn with_state_position(mut self, position: MatrixRoomStatePosition) -> Self {
        self.state_position = position;
        self
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn kind(&self) -> MatrixRoomSyncKind {
        self.kind
    }

    pub const fn timeline_limited(&self) -> bool {
        self.timeline_limited
    }

    pub const fn previous_batch(&self) -> Option<&MatrixBackfillToken> {
        self.previous_batch.as_ref()
    }

    pub fn timeline(&self) -> &[MatrixTimelineEvent] {
        &self.timeline
    }

    pub fn state(&self) -> &[MatrixTimelineEvent] {
        &self.state
    }

    pub const fn state_position(&self) -> MatrixRoomStatePosition {
        self.state_position
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixTimelineEvent {
    event_id: Option<MatrixEventId>,
    sender: Option<MatrixUserId>,
    event_type: MatrixEventType,
    state_key: Option<String>,
    transaction_id: Option<MatrixTransactionId>,
    origin_server_timestamp: Option<u64>,
    content: Value,
    encryption: MatrixTimelineEncryption,
}

/// Matrix 时间线事件在 SDK 解密后的传输信任状态。
///
/// 该类型只描述 Matrix 密码学层，不替代 Agent Room 自己的设备签名验证。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixTimelineEncryption {
    Plaintext,
    EndToEndUntrusted,
    EndToEndTrusted,
}

impl MatrixTimelineEvent {
    /// 创建来自同步或历史接口的已校验原始事件。
    ///
    /// # Errors
    ///
    /// 状态键含控制字符或事件内容越界时返回校验错误。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: Option<MatrixEventId>,
        sender: Option<MatrixUserId>,
        event_type: MatrixEventType,
        state_key: Option<String>,
        transaction_id: Option<MatrixTransactionId>,
        origin_server_timestamp: Option<u64>,
        content: Value,
    ) -> DomainResult<Self> {
        if let Some(state_key) = state_key.as_deref()
            && (state_key.len() > 512 || state_key.chars().any(char::is_control))
        {
            return Err(DomainError::Validation {
                field: "matrix_state_key",
                reason: "状态键超出允许范围",
            });
        }
        validate_event_content(&content)?;
        Ok(Self {
            event_id,
            sender,
            event_type,
            state_key,
            transaction_id,
            origin_server_timestamp,
            content,
            encryption: MatrixTimelineEncryption::Plaintext,
        })
    }

    #[must_use]
    pub const fn with_trusted_end_to_end_encryption(mut self) -> Self {
        self.encryption = MatrixTimelineEncryption::EndToEndTrusted;
        self
    }

    #[must_use]
    pub const fn with_untrusted_end_to_end_encryption(mut self) -> Self {
        self.encryption = MatrixTimelineEncryption::EndToEndUntrusted;
        self
    }

    pub const fn event_id(&self) -> Option<&MatrixEventId> {
        self.event_id.as_ref()
    }

    pub const fn sender(&self) -> Option<&MatrixUserId> {
        self.sender.as_ref()
    }

    pub const fn event_type(&self) -> &MatrixEventType {
        &self.event_type
    }

    pub fn state_key(&self) -> Option<&str> {
        self.state_key.as_deref()
    }

    pub const fn transaction_id(&self) -> Option<&MatrixTransactionId> {
        self.transaction_id.as_ref()
    }

    pub const fn origin_server_timestamp(&self) -> Option<u64> {
        self.origin_server_timestamp
    }

    pub const fn content(&self) -> &Value {
        &self.content
    }

    pub const fn end_to_end_encrypted(&self) -> bool {
        matches!(
            self.encryption,
            MatrixTimelineEncryption::EndToEndUntrusted | MatrixTimelineEncryption::EndToEndTrusted
        )
    }

    pub const fn end_to_end_sender_trusted(&self) -> bool {
        matches!(self.encryption, MatrixTimelineEncryption::EndToEndTrusted)
    }

    pub const fn encryption(&self) -> MatrixTimelineEncryption {
        self.encryption
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSyncBatch {
    next_batch: MatrixSyncToken,
    rooms: Vec<MatrixRoomSync>,
}

impl MatrixSyncBatch {
    pub const fn new(next_batch: MatrixSyncToken, rooms: Vec<MatrixRoomSync>) -> Self {
        Self { next_batch, rooms }
    }

    pub const fn next_batch(&self) -> &MatrixSyncToken {
        &self.next_batch
    }

    pub fn rooms(&self) -> &[MatrixRoomSync] {
        &self.rooms
    }

    /// 用发送设备可见的 `unsigned.transaction_id` 对账未知提交。
    ///
    /// # Errors
    ///
    /// 同一事务标识映射到不同事件 ID 时拒绝继续，防止隐藏协议损坏。
    pub fn reconcile_transaction(
        &self,
        transaction_id: &MatrixTransactionId,
    ) -> DomainResult<Option<MatrixEventId>> {
        let mut matched: Option<MatrixEventId> = None;
        for event in self.rooms.iter().flat_map(MatrixRoomSync::timeline) {
            if event.transaction_id() != Some(transaction_id) {
                continue;
            }
            let Some(event_id) = event.event_id() else {
                continue;
            };
            if matched.as_ref().is_some_and(|known| known != event_id) {
                return Err(DomainError::Validation {
                    field: "matrix_transaction_mapping",
                    reason: "同一事务标识映射到多个事件",
                });
            }
            matched = Some(event_id.clone());
        }
        Ok(matched)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixBackfillRequest {
    from: MatrixBackfillToken,
    limit: NonZeroU16,
}

impl MatrixBackfillRequest {
    /// 创建向历史方向读取的有界请求。
    ///
    /// # Errors
    ///
    /// 单页超过 1000 个事件时返回校验错误。
    pub fn new(from: MatrixBackfillToken, limit: NonZeroU16) -> DomainResult<Self> {
        if limit.get() > MAX_BACKFILL_EVENTS {
            return Err(DomainError::Validation {
                field: "matrix_backfill_limit",
                reason: "单页不能超过 1000 个事件",
            });
        }
        Ok(Self { from, limit })
    }

    pub const fn from(&self) -> &MatrixBackfillToken {
        &self.from
    }

    pub const fn limit(&self) -> NonZeroU16 {
        self.limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixBackfillPage {
    start: MatrixBackfillToken,
    end: Option<MatrixBackfillToken>,
    events: Vec<MatrixTimelineEvent>,
}

impl MatrixBackfillPage {
    pub const fn new(
        start: MatrixBackfillToken,
        end: Option<MatrixBackfillToken>,
        events: Vec<MatrixTimelineEvent>,
    ) -> Self {
        Self { start, end, events }
    }

    pub const fn start(&self) -> &MatrixBackfillToken {
        &self.start
    }

    pub const fn end(&self) -> Option<&MatrixBackfillToken> {
        self.end.as_ref()
    }

    pub fn events(&self) -> &[MatrixTimelineEvent] {
        &self.events
    }
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum: usize,
    allow_empty: bool,
) -> DomainResult<()> {
    if (!allow_empty && value.is_empty())
        || value.len() > maximum
        || value.chars().any(char::is_control)
    {
        return Err(DomainError::Validation {
            field,
            reason: "文本超出允许范围",
        });
    }
    Ok(())
}

fn validate_event_content(content: &Value) -> DomainResult<()> {
    if !content.is_object() {
        return Err(DomainError::Validation {
            field: "matrix_event_content",
            reason: "事件内容必须是 JSON 对象",
        });
    }
    let size = serde_json::to_vec(content)
        .map_err(|_| DomainError::Validation {
            field: "matrix_event_content",
            reason: "事件内容无法序列化",
        })?
        .len();
    if size > MAX_EVENT_PAYLOAD_BYTES {
        return Err(DomainError::Validation {
            field: "matrix_event_content",
            reason: "事件内容不能超过 64 KiB",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use agent_room_domain::time::DurationMillis;
    use serde_json::json;

    use super::{
        MatrixCreateRoom, MatrixEvent, MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomSync,
        MatrixRoomSyncKind, MatrixRoomVisibility, MatrixStateEvent, MatrixSyncBatch,
        MatrixSyncRequest, MatrixTimelineEvent,
    };

    #[test]
    fn 建房策略显式去重成员可写状态事件() {
        let event_type =
            MatrixEventType::new("org.agentroom.agent.status.v1").expect("事件类型有效");
        let request = MatrixCreateRoom::new(
            Some("Lobby".to_owned()),
            None,
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效")
        .with_member_writable_state_event_type(event_type.clone())
        .with_member_writable_state_event_type(event_type.clone());

        assert_eq!(request.member_writable_state_event_types(), &[event_type]);
    }

    #[test]
    fn 建房默认不会意外启用私人房间权限基线() {
        let standard = MatrixCreateRoom::new(
            Some("Lobby".to_owned()),
            None,
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效");
        let private = standard
            .clone()
            .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);

        assert_eq!(standard.power_profile(), MatrixRoomPowerProfile::Standard);
        assert_eq!(
            private.power_profile(),
            MatrixRoomPowerProfile::ManagedPrivate
        );
    }

    use crate::ports::{
        MatrixEventId, MatrixEventType, MatrixRoomId, MatrixStateKey, MatrixSyncToken,
        MatrixTransactionId,
    };

    #[test]
    fn 事件拒绝非对象和超大载荷() {
        let event_type = MatrixEventType::new("org.agentroom.test.v1").expect("事件类型有效");
        let transaction_id = MatrixTransactionId::new("txn-1").expect("事务标识有效");
        assert!(MatrixEvent::new(event_type.clone(), transaction_id.clone(), json!([])).is_err());
        assert!(
            MatrixEvent::new(
                event_type,
                transaction_id,
                json!({ "body": "x".repeat(65_536) }),
            )
            .is_err()
        );
        assert!(
            MatrixStateEvent::new(
                MatrixEventType::new("org.agentroom.agent.status.v1").expect("事件类型有效"),
                MatrixStateKey::new("instance-1").expect("状态键有效"),
                json!([]),
            )
            .is_err()
        );
    }

    #[test]
    fn 同步超时严格受长轮询预算约束() {
        assert!(
            MatrixSyncRequest::new(None, DurationMillis::new(60_000).expect("时长有效"), false,)
                .is_ok()
        );
        assert!(
            MatrixSyncRequest::new(None, DurationMillis::new(60_001).expect("时长有效"), false,)
                .is_err()
        );
    }

    #[test]
    fn 未知提交按事务标识对账且拒绝一对多() {
        let transaction_id = MatrixTransactionId::new("txn-stable").expect("事务标识有效");
        let first = timeline_event("$event-a:example.org", transaction_id.clone());
        let room = MatrixRoomSync::new(
            MatrixRoomId::new("!room:example.org").expect("房间标识有效"),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![first],
            Vec::new(),
        );
        let batch = MatrixSyncBatch::new(
            MatrixSyncToken::new("next-1").expect("同步游标有效"),
            vec![room],
        );
        assert_eq!(
            batch
                .reconcile_transaction(&transaction_id)
                .expect("映射无冲突")
                .expect("已找到事件")
                .as_str(),
            "$event-a:example.org"
        );

        let conflicting_room = MatrixRoomSync::new(
            MatrixRoomId::new("!room:example.org").expect("房间标识有效"),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![
                timeline_event("$event-a:example.org", transaction_id.clone()),
                timeline_event("$event-b:example.org", transaction_id.clone()),
            ],
            Vec::new(),
        );
        let conflicting = MatrixSyncBatch::new(
            MatrixSyncToken::new("next-2").expect("同步游标有效"),
            vec![conflicting_room],
        );
        assert!(conflicting.reconcile_transaction(&transaction_id).is_err());
    }

    fn timeline_event(event_id: &str, transaction_id: MatrixTransactionId) -> MatrixTimelineEvent {
        MatrixTimelineEvent::new(
            Some(MatrixEventId::new(event_id).expect("事件标识有效")),
            None,
            MatrixEventType::new("org.agentroom.test.v1").expect("事件类型有效"),
            None,
            Some(transaction_id),
            Some(1_000),
            json!({ "schemaVersion": "1.0" }),
        )
        .expect("时间线事件有效")
    }
}
