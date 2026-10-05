use agent_room_bridge_core::ipc::IpcScope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    IpcCloseHostSessionRequest, IpcHostSessionSummary, IpcInvitationOffer,
    IpcOpenHostSessionRequest, IpcPendingInvitation, IpcRedeemJoinCodeRequest,
    IpcResolveJoinCodeRequest, IpcWithdrawInvitationRequest, limits,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMethod {
    BridgeStatus,
    HostSessionDiagnostics,
    OpenHostSession(IpcOpenHostSessionRequest),
    CloseHostSession(IpcCloseHostSessionRequest),
    WithSession {
        #[serde(rename = "sessionId")]
        session_id: String,
        method: Box<IpcMethod>,
    },
    GetSelf,
    /// 这台设备的账号能进的房间，供 CLI/MCP 按名字解析房间；不需要先开会话。
    ListRooms,
    /// 桌面端接入面板把准备好的人物挂在 Bridge 上等 Agent 来接；同一时间只挂一份，后挂的替换先挂的。
    OfferInvitation(IpcInvitationOffer),
    /// 面板关闭时撤回自己挂的那份。
    WithdrawInvitation(IpcWithdrawInvitationRequest),
    /// 不带邀请的 join 查看正在等待的人物；用它开出会话时这份邀请才被用掉。
    ReadInvitation,
    /// 凭私人房间口令查看房间，供 CLI/MCP 决定用哪个人物；不让任何 Agent 加入，也不需要先开会话。
    ResolveJoinCode(IpcResolveJoinCodeRequest),
    /// 凭口令让这个会话键的人物加入私人房间；随后照常开会话进入返回的房间。
    RedeemJoinCode(IpcRedeemJoinCodeRequest),
    RegisterReception(crate::IpcRegisterReceptionRequest),
    ReceptionControl(crate::ReceptionRequest),
    SendReceptionMessage {
        run_id: Uuid,
        request: IpcSendMessageRequest,
    },
    MatrixSecurity(crate::IpcMatrixSecurityRequest),
    ListRecoverySessions,
    MatrixRecovery(crate::IpcMatrixRecoveryRequest),
    BootstrapDefaultAgent(IpcBootstrapDefaultAgentRequest),
    ListPreviews(IpcListPreviewsRequest),
    ReadInbox(IpcListPreviewsRequest),
    WaitInbox(IpcListPreviewsRequest),
    /// 按 ID 取消息：事件 ID 或消息 ID，最多 20 个，给全文；只给自己在的房间里的。不动收件箱的位置。
    GetMessages(IpcGetMessagesRequest),
    /// 一条消息和它前后各几条，早的在前。不动收件箱的位置。
    MessagesAround(IpcMessagesAroundRequest),
    /// 往前翻（或从某条往后翻），可以只看某个人、只看提到我的。不动收件箱的位置。
    RoomHistory(IpcRoomHistoryRequest),
    /// 确认收件箱处理到某一条（含）：它和它之前收到的都不再交。位置按房间记，只往前走。
    AckInbox(IpcAckInboxRequest),
    GetPresence(IpcGetPresenceRequest),
    OpenContent(IpcOpenContentRequest),
    PublishStatus(IpcPublishStatusRequest),
    SendMessage(IpcSendMessageRequest),
    ApproveHandoff(IpcApproveHandoffRequest),
    ListHandoffs(IpcListHandoffsRequest),
    ConsumeHandoff(IpcHandoffRequest),
    DeclineHandoff(IpcHandoffRequest),
}

impl IpcMethod {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::BridgeStatus => "bridge_status",
            Self::HostSessionDiagnostics => "host_session_diagnostics",
            Self::OpenHostSession(_) => "open_host_session",
            Self::CloseHostSession(_) => "close_host_session",
            Self::WithSession { method, .. } => method.name(),
            Self::GetSelf => "get_self",
            Self::ListRooms => "list_rooms",
            Self::OfferInvitation(_) => "offer_invitation",
            Self::WithdrawInvitation(_) => "withdraw_invitation",
            Self::ReadInvitation => "read_invitation",
            Self::ResolveJoinCode(_) => "resolve_join_code",
            Self::RedeemJoinCode(_) => "redeem_join_code",
            Self::RegisterReception(_) => "register_reception",
            Self::ReceptionControl(_) => "reception_control",
            Self::SendReceptionMessage { .. } | Self::SendMessage(_) => "send_message",
            Self::MatrixSecurity(_) => "matrix_security",
            Self::ListRecoverySessions => "list_recovery_sessions",
            Self::MatrixRecovery(_) => "matrix_recovery",
            Self::BootstrapDefaultAgent(_) => "bootstrap_default_agent",
            Self::ListPreviews(_) => "list_previews",
            Self::ReadInbox(_) => "read_inbox",
            Self::WaitInbox(_) => "wait_inbox",
            Self::GetMessages(_) => "get_messages",
            Self::MessagesAround(_) => "messages_around",
            Self::RoomHistory(_) => "room_history",
            Self::AckInbox(_) => "ack_inbox",
            Self::GetPresence(_) => "get_presence",
            Self::OpenContent(_) => "open_content",
            Self::PublishStatus(_) => "publish_status",
            Self::ApproveHandoff(_) => "approve_handoff",
            Self::ListHandoffs(_) => "list_handoffs",
            Self::ConsumeHandoff(_) => "consume_handoff",
            Self::DeclineHandoff(_) => "decline_handoff",
        }
    }

    pub const fn required_scope(&self) -> IpcScope {
        match self {
            Self::BridgeStatus | Self::HostSessionDiagnostics => IpcScope::BridgeStatusRead,
            // 凭口令进房间是开会话的一部分：CLI 与 MCP 在开会话之前调用。
            Self::OpenHostSession(_)
            | Self::CloseHostSession(_)
            | Self::ReadInvitation
            | Self::ResolveJoinCode(_)
            | Self::RedeemJoinCode(_)
            | Self::RegisterReception(_)
            | Self::ReceptionControl(_) => IpcScope::HostSessionsManage,
            Self::WithSession { method, .. } => method.required_scope(),
            Self::GetSelf => IpcScope::SelfRead,
            Self::MatrixSecurity(_) => IpcScope::MatrixSecurityManage,
            Self::ListRecoverySessions | Self::MatrixRecovery(_) => IpcScope::MatrixRecoveryManage,
            // 挂邀请等于替用户准备一个人物，与桌面端建默认人物同属一类权限。
            Self::BootstrapDefaultAgent(_)
            | Self::OfferInvitation(_)
            | Self::WithdrawInvitation(_) => IpcScope::AgentBootstrap,
            Self::ListPreviews(_)
            | Self::ReadInbox(_)
            | Self::WaitInbox(_)
            | Self::GetMessages(_)
            | Self::MessagesAround(_)
            | Self::RoomHistory(_)
            // 确认位置只是这个 Agent 自己读到哪了，和读收件箱同一类权限。
            | Self::AckInbox(_)
            | Self::ListRooms => IpcScope::PreviewsRead,
            Self::GetPresence(_) => IpcScope::PresenceRead,
            Self::OpenContent(_) => IpcScope::ContentRead,
            Self::PublishStatus(_) => IpcScope::StatusPublish,
            Self::SendMessage(_) | Self::SendReceptionMessage { .. } => IpcScope::MessageSend,
            Self::ApproveHandoff(_) => IpcScope::HandoffApprove,
            Self::ListHandoffs(_) => IpcScope::HandoffList,
            Self::ConsumeHandoff(_) => IpcScope::HandoffConsume,
            Self::DeclineHandoff(_) => IpcScope::HandoffDecline,
        }
    }

    /// 在进入 Bridge 用例前执行传输层硬上限校验。
    ///
    /// # Errors
    ///
    /// 任一标识、文本、集合或分页参数超出闭合协议边界时返回稳定错误。
    pub fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        match self {
            Self::BridgeStatus
            | Self::GetSelf
            | Self::ListRooms
            | Self::ReadInvitation
            | Self::ListRecoverySessions
            | Self::HostSessionDiagnostics => Ok(()),
            Self::WithdrawInvitation(request) => request.validate(),
            Self::ResolveJoinCode(request) => request.validate(),
            Self::RedeemJoinCode(request) => request.validate(),
            Self::MatrixRecovery(request) => request.command().map(|_| ()),
            Self::MatrixSecurity(request) => request.command().map(|_| ()),
            Self::OpenHostSession(request) => request.validate(),
            Self::OfferInvitation(offer) => offer.validate(),
            Self::CloseHostSession(request) => request.validate(),
            Self::RegisterReception(request) => request.validate(),
            Self::ReceptionControl(request) => {
                if request.valid() {
                    Ok(())
                } else {
                    Err(failure("bridge.ipc.reception_invalid"))
                }
            }
            Self::SendReceptionMessage { run_id, request } => {
                validate_uuid_v7(&run_id.to_string(), "bridge.ipc.reception_invalid")?;
                request.validate()
            }
            Self::WithSession { session_id, method } => {
                crate::host_sessions::validate_session_id(session_id)?;
                if matches!(
                    method.as_ref(),
                    Self::WithSession { .. }
                        | Self::OpenHostSession(_)
                        | Self::CloseHostSession(_)
                        | Self::BootstrapDefaultAgent(_)
                        | Self::BridgeStatus
                        | Self::HostSessionDiagnostics
                        | Self::ListRecoverySessions
                        | Self::ListRooms
                        | Self::OfferInvitation(_)
                        | Self::WithdrawInvitation(_)
                        | Self::ReadInvitation
                        | Self::ResolveJoinCode(_)
                        | Self::RedeemJoinCode(_)
                ) {
                    return Err(failure("bridge.ipc.session_method_invalid"));
                }
                method.validate()
            }
            Self::BootstrapDefaultAgent(request) => request.validate(),
            Self::ListPreviews(request) => {
                // 往前翻的页没有确认位置可言。
                if request.from_ack {
                    return Err(failure("bridge.ipc.event_cursor_invalid"));
                }
                request.validate()
            }
            Self::ReadInbox(request) | Self::WaitInbox(request) => {
                if request.before_event_id.is_some() {
                    return Err(failure("bridge.ipc.event_cursor_invalid"));
                }
                request.validate()
            }
            Self::GetMessages(request) => request.validate(),
            Self::MessagesAround(request) => request.validate(),
            Self::RoomHistory(request) => request.validate(),
            Self::AckInbox(request) => validate_message_reference(&request.id),
            Self::GetPresence(request) => request.validate(),
            Self::OpenContent(request) => request.validate(),
            Self::PublishStatus(request) => request.validate(),
            Self::SendMessage(request) => request.validate(),
            Self::ApproveHandoff(request) => request.validate(),
            Self::ListHandoffs(request) => request.validate(),
            Self::ConsumeHandoff(request) | Self::DeclineHandoff(request) => request.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcBootstrapDefaultAgentRequest {
    pub preferred_language: Option<String>,
}

impl IpcBootstrapDefaultAgentRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_optional_bounded(
            self.preferred_language.as_deref(),
            limits::LANGUAGE_BYTES,
            "bridge.ipc.language_invalid",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcListPreviewsRequest {
    #[serde(default)]
    pub after_event_id: Option<String>,
    pub room_id: Option<String>,
    pub before_event_id: Option<String>,
    pub limit: u16,
    /// 等消息的客户端读到了消息、但按规则先不交时，仍然算在等（房间里照样显示“等待中”）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_waiting: bool,
    /// 没有新消息时 Bridge 最多挂多久再空手返回（毫秒，最多 8000）；来了新消息立刻返回。
    /// 只对 `WaitInbox` 有用，省得客户端每秒新开一条连接来问。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
    /// 没给 `afterEventId` 时从这个房间的确认位置之后开始，没确认过就从最早一条开始。给了
    /// `afterEventId` 就从它之后，不看确认位置。只对 `ReadInbox`、`WaitInbox` 有用。
    #[serde(default, skip_serializing_if = "is_false")]
    pub from_ack: bool,
}

impl IpcListPreviewsRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if self.after_event_id.is_some() && self.before_event_id.is_some() {
            return Err(failure("bridge.ipc.event_cursor_invalid"));
        }
        if self
            .wait_ms
            .is_some_and(|wait| wait > limits::INBOX_BLOCK_MILLIS)
        {
            return Err(failure("bridge.ipc.inbox_wait_invalid"));
        }
        validate_optional_bounded(
            self.after_event_id.as_deref(),
            limits::EVENT_ID_BYTES,
            "bridge.ipc.event_id_invalid",
        )?;

        validate_optional_bounded(
            self.room_id.as_deref(),
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        validate_optional_bounded(
            self.before_event_id.as_deref(),
            limits::EVENT_ID_BYTES,
            "bridge.ipc.event_id_invalid",
        )?;
        if !(1..=limits::PREVIEW_PAGE_SIZE).contains(&self.limit) {
            return Err(failure("bridge.ipc.preview_limit_invalid"));
        }
        Ok(())
    }
}

/// 确认收件箱（`AckInbox`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcAckInboxRequest {
    /// 处理到的最后一条：事件 ID（`$` 开头）或消息 ID（UUIDv7），不用给房间。
    pub id: String,
}

/// 按 ID 取消息（`GetMessages`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcGetMessagesRequest {
    /// 事件 ID（`$` 开头）或消息 ID（UUIDv7），1 到 20 个。
    pub ids: Vec<String>,
}

impl IpcGetMessagesRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if self.ids.is_empty() || self.ids.len() > limits::MESSAGE_LOOKUP_IDS {
            return Err(failure("bridge.ipc.message_ids_invalid"));
        }
        self.ids
            .iter()
            .try_for_each(|id| validate_message_reference(id))
    }
}

/// 看前后（`MessagesAround`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMessagesAroundRequest {
    /// 哪个房间；不给就是会话所在的房间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// 中间那条：事件 ID 或消息 ID。
    pub id: String,
    /// 它前面几条，0 到 20。
    pub before: u16,
    /// 它后面几条，0 到 20。
    pub after: u16,
}

impl IpcMessagesAroundRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_optional_bounded(
            self.room_id.as_deref(),
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        validate_message_reference(&self.id)?;
        if self.before > limits::AROUND_MESSAGES || self.after > limits::AROUND_MESSAGES {
            return Err(failure("bridge.ipc.around_limit_invalid"));
        }
        Ok(())
    }
}

/// 往前翻（`RoomHistory`）：不给 `before`、`after` 时从最新的一条往前。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomHistoryRequest {
    /// 哪个房间；不给就是会话所在的房间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// 从这条往前翻（新的在前）：事件 ID 或消息 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// 从这条往后翻（旧的在前）：事件 ID 或消息 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// 最多几条，1 到 50。
    pub limit: u16,
    /// 只看某个人：Matrix 用户 ID（`@` 开头），或者名字（不分大小写）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// 只看提到我或回复我的。
    #[serde(default, skip_serializing_if = "is_false")]
    pub mentions_me: bool,
}

impl IpcRoomHistoryRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if self.before.is_some() && self.after.is_some() {
            return Err(failure("bridge.ipc.event_cursor_invalid"));
        }
        validate_optional_bounded(
            self.room_id.as_deref(),
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        if let Some(cursor) = self.before.as_deref().or(self.after.as_deref()) {
            validate_message_reference(cursor)?;
        }
        if !(1..=limits::PREVIEW_PAGE_SIZE).contains(&self.limit) {
            return Err(failure("bridge.ipc.preview_limit_invalid"));
        }
        if let Some(from) = &self.from {
            validate_bounded(
                from,
                limits::MESSAGE_FROM_BYTES,
                "bridge.ipc.message_from_invalid",
            )?;
            if from.trim().is_empty() {
                return Err(failure("bridge.ipc.message_from_invalid"));
            }
        }
        Ok(())
    }
}

/// 事件 ID（`$` 开头）或消息 ID（UUIDv7）。
fn validate_message_reference(id: &str) -> Result<(), IpcMethodValidationFailure> {
    if id.starts_with('$') {
        // 和 Matrix 事件 ID 的规则一致：至少 4 个字节，不含空白。
        if id.len() < 4 || id.chars().any(char::is_whitespace) {
            return Err(failure("bridge.ipc.message_id_invalid"));
        }
        validate_bounded(id, limits::EVENT_ID_BYTES, "bridge.ipc.message_id_invalid")
    } else {
        validate_uuid_v7(id, "bridge.ipc.message_id_invalid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcGetPresenceRequest {
    pub room_id: String,
    #[serde(default)]
    pub agent_ids: Vec<String>,
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub after_agent_id: Option<String>,
    #[serde(default = "default_presence_limit")]
    pub limit: u16,
}

const fn default_presence_limit() -> u16 {
    100
}

impl IpcGetPresenceRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_bounded(
            &self.room_id,
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        if self.agent_ids.len() > limits::PRESENCE_TARGETS {
            return Err(failure("bridge.ipc.presence_targets_invalid"));
        }
        if self.limit == 0 || self.limit > 100 {
            return Err(failure("bridge.ipc.presence_limit_invalid"));
        }
        if let Some(after) = &self.after_agent_id {
            validate_uuid_v7(after, "bridge.ipc.agent_id_invalid")?;
        }
        self.agent_ids
            .iter()
            .try_for_each(|value| validate_uuid(value, "bridge.ipc.agent_id_invalid"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenContentRequest {
    pub room_id: Option<String>,
    pub content_id: String,
}

impl IpcOpenContentRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if let Some(room_id) = &self.room_id {
            validate_bounded(room_id, limits::ROOM_ID_BYTES, "bridge.ipc.room_id_invalid")?;
        }
        validate_uuid(&self.content_id, "bridge.ipc.content_id_invalid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPublishStatusRequest {
    pub room_id: String,
    pub status: IpcWorkStatus,
    pub task_summary: Option<String>,
    pub progress_basis_points: Option<u16>,
}

impl IpcPublishStatusRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_bounded(
            &self.room_id,
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        if let Some(summary) = &self.task_summary {
            validate_human_text(
                summary,
                limits::TASK_SUMMARY_CHARACTERS,
                "bridge.ipc.task_summary_invalid",
            )?;
        }
        if self
            .progress_basis_points
            .is_some_and(|progress| progress > limits::PROGRESS_BASIS_POINTS)
        {
            return Err(failure("bridge.ipc.progress_invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcSendMessageRequest {
    #[serde(default)]
    pub chat: bool,
    #[serde(default)]
    pub mentions: Vec<String>,
    /// @所有人：只有聊天能带，只能发到私人房间（端到端加密的房间）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub mentions_everyone: bool,
    pub submission_id: Option<String>,
    pub automation_grant_id: Option<String>,
    pub room_id: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub media_type: String,
    pub language: Option<String>,
    pub sensitivity: IpcMessageSensitivity,
    #[serde(default)]
    pub risk_flags: Vec<String>,
    pub provenance: IpcMessageProvenance,
    pub reply_to_message_id: Option<String>,
}

impl IpcSendMessageRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if self.chat {
            agent_room_bridge_core::messages::validate_chat(&self.body, &self.mentions)
                .map_err(|_| failure("bridge.ipc.conversation_invalid"))?;
            if self.media_type != "text/plain" {
                return Err(failure("bridge.ipc.conversation_invalid"));
            }
        } else if !self.mentions.is_empty() || self.mentions_everyone {
            return Err(failure("bridge.ipc.conversation_invalid"));
        }

        if let Some(submission_id) = &self.submission_id {
            validate_uuid_v7(submission_id, "bridge.ipc.submission_id_invalid")?;
        }
        if let Some(grant_id) = &self.automation_grant_id {
            validate_uuid_v7(grant_id, "bridge.ipc.automation_grant_id_invalid")?;
        }
        let is_automated = self.provenance == IpcMessageProvenance::AutonomousAgent;
        if is_automated != self.automation_grant_id.is_some() {
            return Err(failure("bridge.ipc.automation_grant_required"));
        }
        validate_bounded(
            &self.room_id,
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        validate_human_text(
            &self.title,
            limits::TITLE_CHARACTERS,
            "bridge.ipc.message_title_invalid",
        )?;
        validate_human_text(
            &self.summary,
            limits::SUMMARY_CHARACTERS,
            "bridge.ipc.message_summary_invalid",
        )?;
        if self.body.is_empty() || self.body.len() > limits::INLINE_TEXT_BYTES {
            return Err(failure("bridge.ipc.message_body_invalid"));
        }
        validate_bounded(
            &self.media_type,
            limits::MEDIA_TYPE_BYTES,
            "bridge.ipc.media_type_invalid",
        )?;
        validate_optional_bounded(
            self.language.as_deref(),
            limits::LANGUAGE_BYTES,
            "bridge.ipc.language_invalid",
        )?;
        validate_risk_flags(&self.risk_flags)?;
        if let Some(message_id) = &self.reply_to_message_id {
            validate_uuid_v7(message_id, "bridge.ipc.message_id_invalid")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcHandoffRequest {
    pub handoff_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcListHandoffsRequest {
    pub limit: u16,
}

impl IpcListHandoffsRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if !(1..=limits::HANDOFF_PAGE_SIZE).contains(&self.limit) {
            return Err(failure("bridge.ipc.handoff_limit_invalid"));
        }
        Ok(())
    }
}

impl IpcHandoffRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.handoff_id, "bridge.ipc.handoff_id_invalid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcApproveHandoffRequest {
    pub handoff_id: String,
    pub principal_id: String,
    pub room_id: String,
    pub source_content_id: String,
    pub target_agent_id: String,
    pub target_instance_id: String,
    pub permissions: Vec<IpcHandoffPermission>,
    pub purpose: IpcHandoffPurpose,
    pub expires_at_unix_ms: i64,
}

impl IpcApproveHandoffRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.handoff_id, "bridge.ipc.handoff_id_invalid")?;
        validate_uuid_v7(&self.principal_id, "bridge.ipc.principal_id_invalid")?;
        validate_bounded(
            &self.room_id,
            limits::ROOM_ID_BYTES,
            "bridge.ipc.room_id_invalid",
        )?;
        validate_uuid_v7(&self.source_content_id, "bridge.ipc.content_id_invalid")?;
        validate_uuid_v7(&self.target_agent_id, "bridge.ipc.target_agent_id_invalid")?;
        validate_uuid_v7(
            &self.target_instance_id,
            "bridge.ipc.target_instance_id_invalid",
        )?;
        if self.permissions.is_empty() || self.permissions.len() > 3 {
            return Err(failure("bridge.ipc.handoff_permissions_invalid"));
        }
        let unique = self
            .permissions
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != self.permissions.len() {
            return Err(failure("bridge.ipc.handoff_permissions_invalid"));
        }
        if self.expires_at_unix_ms <= 0 {
            return Err(failure("bridge.ipc.handoff_expiry_invalid"));
        }
        Ok(())
    }
}

/// 时间线上少了的一段。`beforeEventId` 这条前面少了消息，`afterEventId` 是之前最后一条（房间里
/// 原来没消息时没有）。`reason` 是 `too_many`（同步时一次来得太多，往回补也没接上）；网络接入还有
/// `undecryptable_before_join`（凭口令进的私人房间里加入之前的消息解不开，没有 `afterEventId`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcTimelineGap {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_event_id: Option<String>,
    pub before_event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcResponse {
    Reception {
        record: crate::ReceptionRecord,
    },
    HostSessionDiagnostics {
        sessions: Vec<crate::IpcHostSessionDiagnostics>,
    },
    RecoverySessions {
        sessions: Vec<crate::IpcAgentRecoverySession>,
    },
    MatrixRecovery {
        recovery: crate::IpcMatrixRecoveryResult,
    },
    MatrixSecurity {
        security: crate::IpcMatrixSecurityResult,
    },
    HostSession {
        session: IpcHostSessionSummary,
    },
    BridgeStatus {
        state: IpcBridgeState,
        #[serde(rename = "startedAtUnixMs")]
        started_at_unix_ms: i64,
        /// 离线时 Bridge 自己判定的原因码，桌面端据此说明为什么停了。
        #[serde(
            rename = "failureCode",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        failure_code: Option<String>,
    },
    SelfSummary {
        summary: IpcSelfSummary,
    },
    Rooms {
        rooms: Vec<IpcRoomSummary>,
    },
    /// 挂上、撤回或查看等待接入的人物；没有等待中的邀请时为空。
    Invitation {
        invitation: Option<IpcPendingInvitation>,
    },
    /// 口令对应的私人房间：查看时还没有人加入，兑换后选定的人物已是它的 Agent 成员。
    JoinCodeRoom {
        room: IpcRoomSummary,
    },
    DefaultAgentBootstrap {
        bootstrap: IpcDefaultAgentBootstrap,
    },
    MessagePreviews {
        previews: Vec<IpcMessagePreviewSummary>,
        #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
        /// 等消息（`WaitInbox`）时，这个房间里此刻在打字的人：叫醒它的人还在打字就再等等。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        typing: Vec<crate::wake::IpcTyping>,
        /// 这一页里哪几条前面少了一段补不回来的消息（`specs/agent-reading/design.md` 第 4 步）。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        gaps: Vec<IpcTimelineGap>,
    },
    /// 按 ID 取到的消息，给全文，按要的顺序。`missing` 是找不到、或不在你所在房间里的；
    /// `more` 是这次放不下、要再取一次的。
    Messages {
        messages: Vec<IpcMessagePreviewSummary>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missing: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        more: Vec<String>,
    },
    /// 房间里的一段消息（看前后、往前翻）。`nextCursor` 是接着翻的位置，没有就是翻到头了。
    RoomMessages {
        messages: Vec<IpcMessagePreviewSummary>,
        #[serde(
            rename = "nextCursor",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        next_cursor: Option<String>,
    },
    /// 确认收件箱的结果。`acknowledged` 为 false 是这一条早就确认过了，位置不往回走；
    /// `pending` 是这个房间里确认位置之后还有几条别人发的。
    InboxAcknowledged {
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "eventId")]
        event_id: String,
        acknowledged: bool,
        pending: u64,
    },
    Presence {
        entries: Vec<IpcPresenceSummary>,
        #[serde(
            rename = "nextCursor",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        next_cursor: Option<String>,
    },
    OpenedContent {
        content: IpcOpenedContent,
    },
    PublishedStatus {
        publication: IpcPublishedStatus,
    },
    SentMessage {
        message: IpcSentMessage,
    },
    ApprovedHandoff {
        handoff: IpcHandoffSubmission,
    },
    PendingTargetedHandoffs {
        handoffs: Vec<IpcPendingTargetedHandoff>,
    },
    ConsumedHandoff {
        handoff: IpcConsumedHandoff,
    },
    ConsumedTargetedHandoff {
        handoff: IpcConsumedTargetedHandoff,
    },
    DeclinedHandoff {
        handoff: IpcDeclinedHandoff,
    },
    DeclinedTargetedHandoff {
        handoff: IpcDeclinedTargetedHandoff,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcDefaultAgentBootstrap {
    pub agent_id: String,
    pub display_name: String,
    pub public_lobby_catalog_id: String,
    pub lobby_language: Option<String>,
}

/// 账号能进的一个房间：公开大厅只有目录，私人房间还带 Matrix 房间与成员状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomSummary {
    pub kind: IpcRoomKind,
    pub catalog_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix_room_id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<IpcRoomMembership>,
}

impl agent_room_bridge_core::room_directory::NamedRoom for IpcRoomSummary {
    fn room_name(&self) -> &str {
        &self.name
    }

    fn room_slug(&self) -> Option<&str> {
        self.slug.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcRoomKind {
    PublicLobby,
    PrivateRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcRoomMembership {
    Invited,
    Joined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcSelfSummary {
    /// 这台电脑上 Bridge 的主人：等消息时主人说话总能叫醒 Agent。不知道时没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<IpcOwnerSummary>,
    #[serde(default)]
    pub room_catalog_id: Option<String>,
    pub agent: IpcAgentSummary,
    pub instance_id: String,
    pub matrix_device_id: String,
    pub room_id: String,
    pub connection_state: IpcBridgeState,
    pub granted_capabilities: Vec<String>,
}

/// 主人：授权这台电脑的账号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOwnerSummary {
    pub principal_id: String,
    pub matrix_user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcAgentSummary {
    pub agent_id: String,
    pub display_name: String,
    pub matrix_user_id: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcActorSummary {
    #[serde(rename_all = "camelCase")]
    Agent {
        agent: IpcAgentSummary,
        instance_id: String,
        provenance: IpcMessageProvenance,
    },
    #[serde(rename_all = "camelCase")]
    Human {
        principal_id: String,
        display_name: String,
        matrix_user_id: String,
        avatar_url: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcContentReference {
    pub content_id: String,
    pub digest_sha256: String,
    pub media_type: String,
    pub size_bytes: u64,
}

// 线上格式直接给 Agent 看：几个互不相干的标记平铺着最好读；`deny_unknown_fields` 也不能和 flatten 一起用。
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMessagePreviewSummary {
    pub conversation: Option<IpcConversationMessage>,
    pub reply_to_message_id: Option<String>,
    /// 回复的是哪条：作者和原文开头，看一眼就知道在回什么。找不到被回复的那条时没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<IpcReplyExcerpt>,
    pub message_id: String,
    pub event_id: String,
    pub room_id: String,
    pub actor: IpcActorSummary,
    pub created_at_unix_ms: i64,
    pub title: String,
    pub summary: String,
    pub content: IpcContentReference,
    pub language: Option<String>,
    pub sensitivity: IpcMessageSensitivity,
    pub risk_flags: Vec<String>,
    /// 是不是读消息的这个 Agent 自己发的。
    #[serde(default)]
    pub from_me: bool,
    /// 提到了读消息的这个 Agent，或者回复的是它发的消息；私人房间里 @所有人 也算。
    #[serde(default)]
    pub mentions_me: bool,
    /// 这条 @所有人：私人房间里群发给每个人的，斟酌要不要每条都回。公开大厅里的不算，不标。
    #[serde(default, skip_serializing_if = "is_false")]
    pub mentions_everyone: bool,
    /// 房间名，在几个房间里时好认。不知道时没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,
    /// 是它加入这个房间之前的消息（刚加入时给的上下文），别去回答过时的问题。
    /// 不知道它什么时候加入的就不标。
    #[serde(default)]
    pub before_join: bool,
}

/// 被回复的那条：作者和原文开头。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcReplyExcerpt {
    pub message_id: String,
    pub actor_name: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPresenceSummary {
    pub room_id: String,
    pub agent: IpcAgentSummary,
    pub instance_id: String,
    pub status: IpcWorkStatus,
    pub observed_at_unix_ms: i64,
    pub lease_expires_at_unix_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<IpcAgentLifecycle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcAgentLifecycle {
    pub connection: IpcAgentConnection,
    pub reception: IpcAgentReception,
    pub reported_status: IpcWorkStatus,
    pub last_active_at_unix_ms: i64,
    pub last_polled_at_unix_ms: Option<i64>,
    pub listening_until_unix_ms: Option<i64>,
    pub offline_since_unix_ms: Option<i64>,
    pub archive_reason: Option<IpcAgentArchiveReason>,
    pub archive_after_days: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcAgentConnection {
    Online,
    Reconnecting,
    Offline,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcAgentReception {
    Waiting,
    OnResume,
    Unknown,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcAgentArchiveReason {
    Expired,
    Capacity,
}

impl From<&agent_room_bridge_core::presence::PresenceObservation> for IpcAgentLifecycle {
    fn from(entry: &agent_room_bridge_core::presence::PresenceObservation) -> Self {
        use agent_room_domain::agent_lifecycle::{
            AgentArchiveReason, AgentConnection, AgentReception,
        };
        use agent_room_domain::agent_status::AgentWorkStatus;
        Self {
            connection: match entry.lifecycle.connection {
                AgentConnection::Online => IpcAgentConnection::Online,
                AgentConnection::Reconnecting => IpcAgentConnection::Reconnecting,
                AgentConnection::Offline => IpcAgentConnection::Offline,
            },
            reception: match entry.lifecycle.reception {
                AgentReception::Waiting => IpcAgentReception::Waiting,
                AgentReception::OnResume => IpcAgentReception::OnResume,
                AgentReception::Unknown => IpcAgentReception::Unknown,
                AgentReception::Unavailable => IpcAgentReception::Unavailable,
            },
            reported_status: match entry.presence().status() {
                AgentWorkStatus::Offline => IpcWorkStatus::Offline,
                AgentWorkStatus::Idle => IpcWorkStatus::Idle,
                AgentWorkStatus::Working => IpcWorkStatus::Working,
                AgentWorkStatus::WaitingInput => IpcWorkStatus::WaitingInput,
                AgentWorkStatus::Blocked => IpcWorkStatus::Blocked,
                AgentWorkStatus::Completed => IpcWorkStatus::Completed,
            },
            last_active_at_unix_ms: entry.last_active_at,
            last_polled_at_unix_ms: entry.last_polled_at,
            listening_until_unix_ms: entry.listening_until,
            offline_since_unix_ms: entry.lifecycle.offline_since,
            archive_reason: entry.lifecycle.archive_reason.map(|reason| match reason {
                AgentArchiveReason::Expired => IpcAgentArchiveReason::Expired,
                AgentArchiveReason::Capacity => IpcAgentArchiveReason::Capacity,
            }),
            archive_after_days: entry.archive_after_days,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenedContent {
    pub content: IpcContentReference,
    pub source_room_id: String,
    pub source_event_id: String,
    pub source_actor: IpcActorSummary,
    pub risk_flags: Vec<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<IpcOpenedAttachment>,
}

/// Verified attachment downloaded on the machine running the Bridge. Re-open the
/// content if this bounded temporary cache has expired or the application restarted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenedAttachment {
    pub name: String,
    pub local_path: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPublishedStatus {
    pub room_id: String,
    pub status: IpcWorkStatus,
    pub lease_expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcSentMessage {
    pub submission_id: String,
    pub state: IpcSubmissionState,
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcConsumedHandoff {
    pub handoff_id: String,
    pub source_room_id: String,
    pub source_event_id: String,
    pub source_actor: IpcActorSummary,
    pub purpose: String,
    pub risk_flags: Vec<String>,
    pub content: IpcContentReference,
    pub body: String,
}

/// 账号级云端交接的发起者。该主体是人类账号，不能伪装成 Agent 实例。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcHumanHandoffSource {
    pub principal_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPendingTargetedHandoff {
    pub handoff_id: String,
    pub source: IpcHumanHandoffSource,
    pub source_room_id: String,
    pub source_event_id: String,
    pub source_message_id: String,
    pub status: IpcHandoffStatus,
    pub purpose: String,
    pub permissions: Vec<IpcHandoffPermission>,
    pub content: IpcContentReference,
    pub created_at_unix_ms: i64,
    pub queued_at_unix_ms: i64,
    pub delivered_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcConsumedTargetedHandoff {
    pub handoff: IpcPendingTargetedHandoff,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcDeclinedHandoff {
    pub handoff_id: String,
    pub status: IpcHandoffStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcDeclinedTargetedHandoff {
    pub handoff_id: String,
    pub status: IpcHandoffStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcHandoffSubmission {
    Submitted {
        #[serde(rename = "handoffId")]
        handoff_id: String,
        reused: bool,
    },
    DeliveryUncertain {
        #[serde(rename = "handoffId")]
        handoff_id: String,
    },
    Resolved {
        #[serde(rename = "handoffId")]
        handoff_id: String,
        status: IpcHandoffStatus,
    },
    Failed {
        #[serde(rename = "handoffId")]
        handoff_id: String,
        code: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcBridgeState {
    Starting,
    Ready,
    Reconnecting,
    Offline,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcWorkStatus {
    Offline,
    Idle,
    Working,
    WaitingInput,
    Blocked,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMessageSensitivity {
    Normal,
    Sensitive,
    Restricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMessageProvenance {
    Human,
    HumanConfirmedAgent,
    AutonomousAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcHandoffPermission {
    ReadText,
    ReadAttachments,
    IncludeMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcHandoffPurpose {
    Inspect,
    Summarize,
    ReplyDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcSubmissionState {
    Submitted,
    UnknownCommit,
    BindingPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcHandoffStatus {
    Approved,
    Delivered,
    Consumed,
    Declined,
    Revoked,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcMethodValidationFailure {
    code: &'static str,
}

impl IpcMethodValidationFailure {
    pub const fn code(self) -> &'static str {
        self.code
    }
}

fn validate_optional_bounded(
    value: Option<&str>,
    maximum_bytes: usize,
    code: &'static str,
) -> Result<(), IpcMethodValidationFailure> {
    value.map_or(Ok(()), |value| validate_bounded(value, maximum_bytes, code))
}

fn validate_bounded(
    value: &str,
    maximum_bytes: usize,
    code: &'static str,
) -> Result<(), IpcMethodValidationFailure> {
    if value.is_empty() || value.len() > maximum_bytes || value.chars().any(char::is_control) {
        return Err(failure(code));
    }
    Ok(())
}

fn validate_human_text(
    value: &str,
    maximum_characters: usize,
    code: &'static str,
) -> Result<(), IpcMethodValidationFailure> {
    if value.trim().is_empty()
        || value.chars().count() > maximum_characters
        || value.chars().any(char::is_control)
    {
        return Err(failure(code));
    }
    Ok(())
}

fn validate_uuid(value: &str, code: &'static str) -> Result<(), IpcMethodValidationFailure> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| failure(code))
}

fn validate_uuid_v7(value: &str, code: &'static str) -> Result<(), IpcMethodValidationFailure> {
    let id = Uuid::parse_str(value).map_err(|_| failure(code))?;
    if id.get_version() != Some(uuid::Version::SortRand) || id.to_string() != value {
        return Err(failure(code));
    }
    Ok(())
}

fn validate_risk_flags(flags: &[String]) -> Result<(), IpcMethodValidationFailure> {
    if flags.len() > limits::RISK_FLAGS {
        return Err(failure("bridge.ipc.risk_flags_invalid"));
    }
    if flags.iter().any(|flag| {
        let mut bytes = flag.bytes();
        !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || flag.len() > limits::RISK_FLAG_BYTES
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return Err(failure("bridge.ipc.risk_flags_invalid"));
    }
    Ok(())
}

pub(crate) const fn failure(code: &'static str) -> IpcMethodValidationFailure {
    IpcMethodValidationFailure { code }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::IpcScope;
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        IpcAckInboxRequest, IpcApproveHandoffRequest, IpcBootstrapDefaultAgentRequest,
        IpcGetMessagesRequest, IpcHandoffPermission, IpcHandoffPurpose, IpcHandoffRequest,
        IpcListHandoffsRequest, IpcListPreviewsRequest, IpcMessageProvenance,
        IpcMessageSensitivity, IpcMessagesAroundRequest, IpcMethod, IpcPublishStatusRequest,
        IpcResponse, IpcRoomHistoryRequest, IpcSendMessageRequest, IpcTimelineGap, IpcWorkStatus,
    };
    use crate::limits;

    #[test]
    fn 每个工具方法映射到独立最小作用域() {
        let id = Uuid::now_v7().to_string();
        let methods = [
            (IpcMethod::GetSelf, IpcScope::SelfRead),
            (
                IpcMethod::BootstrapDefaultAgent(IpcBootstrapDefaultAgentRequest {
                    preferred_language: Some("zh-CN".to_owned()),
                }),
                IpcScope::AgentBootstrap,
            ),
            (
                IpcMethod::ListPreviews(IpcListPreviewsRequest {
                    after_event_id: None,
                    room_id: None,
                    before_event_id: None,
                    limit: 20,
                    keep_waiting: false,
                    wait_ms: None,
                    from_ack: false,
                }),
                IpcScope::PreviewsRead,
            ),
            (
                IpcMethod::ApproveHandoff(IpcApproveHandoffRequest {
                    handoff_id: id.clone(),
                    principal_id: Uuid::now_v7().to_string(),
                    room_id: "!room:matrix.test".to_owned(),
                    source_content_id: Uuid::now_v7().to_string(),
                    target_agent_id: Uuid::now_v7().to_string(),
                    target_instance_id: Uuid::now_v7().to_string(),
                    permissions: vec![IpcHandoffPermission::ReadText],
                    purpose: IpcHandoffPurpose::Summarize,
                    expires_at_unix_ms: 2_000,
                }),
                IpcScope::HandoffApprove,
            ),
            (
                IpcMethod::ListHandoffs(IpcListHandoffsRequest { limit: 20 }),
                IpcScope::HandoffList,
            ),
            (
                IpcMethod::ConsumeHandoff(IpcHandoffRequest {
                    handoff_id: id.clone(),
                }),
                IpcScope::HandoffConsume,
            ),
            (
                IpcMethod::DeclineHandoff(IpcHandoffRequest { handoff_id: id }),
                IpcScope::HandoffDecline,
            ),
        ];

        for (method, scope) in methods {
            assert_eq!(method.required_scope(), scope);
            assert!(method.validate().is_ok());
        }
    }

    #[test]
    fn 按需查看的参数在进入_bridge_之前校验() {
        let event = "$message:matrix.test".to_owned();
        let message = "0198b601-77a1-7bb8-83eb-a8fe68c97e60".to_owned();
        let code = |method: IpcMethod| {
            method
                .validate()
                .map_err(super::IpcMethodValidationFailure::code)
        };
        let get = |ids: Vec<String>| IpcMethod::GetMessages(IpcGetMessagesRequest { ids });
        assert_eq!(code(get(vec![event.clone(), message.clone()])), Ok(()));
        assert_eq!(code(get(Vec::new())), Err("bridge.ipc.message_ids_invalid"));
        assert_eq!(
            code(get(vec![event.clone(); limits::MESSAGE_LOOKUP_IDS + 1])),
            Err("bridge.ipc.message_ids_invalid")
        );
        for bad in ["abc", "550e8400-e29b-41d4-a716-446655440000", "$"] {
            assert_eq!(
                code(get(vec![bad.to_owned()])),
                Err("bridge.ipc.message_id_invalid"),
                "{bad}"
            );
        }

        let around = |before: u16| {
            IpcMethod::MessagesAround(IpcMessagesAroundRequest {
                room_id: None,
                id: message.clone(),
                before,
                after: 0,
            })
        };
        assert_eq!(code(around(limits::AROUND_MESSAGES)), Ok(()));
        assert_eq!(
            code(around(limits::AROUND_MESSAGES + 1)),
            Err("bridge.ipc.around_limit_invalid")
        );

        let history = IpcRoomHistoryRequest {
            room_id: None,
            before: Some(event.clone()),
            after: None,
            limit: 20,
            from: Some("Ada".to_owned()),
            mentions_me: true,
        };
        assert_eq!(code(IpcMethod::RoomHistory(history.clone())), Ok(()));
        let invalid = [
            (
                IpcRoomHistoryRequest {
                    after: Some(message.clone()),
                    ..history.clone()
                },
                "bridge.ipc.event_cursor_invalid",
            ),
            (
                IpcRoomHistoryRequest {
                    limit: 0,
                    ..history.clone()
                },
                "bridge.ipc.preview_limit_invalid",
            ),
            (
                IpcRoomHistoryRequest {
                    limit: limits::PREVIEW_PAGE_SIZE + 1,
                    ..history.clone()
                },
                "bridge.ipc.preview_limit_invalid",
            ),
            (
                IpcRoomHistoryRequest {
                    from: Some("   ".to_owned()),
                    ..history.clone()
                },
                "bridge.ipc.message_from_invalid",
            ),
            (
                IpcRoomHistoryRequest {
                    from: Some("名".repeat(100)),
                    ..history.clone()
                },
                "bridge.ipc.message_from_invalid",
            ),
        ];
        for (request, expected) in invalid {
            assert_eq!(code(IpcMethod::RoomHistory(request)), Err(expected));
        }

        // 都是读消息的权限，可以在会话里调用。
        for method in [
            get(vec![event.clone()]),
            around(1),
            IpcMethod::RoomHistory(history),
        ] {
            assert_eq!(method.required_scope(), IpcScope::PreviewsRead);
            let scoped = IpcMethod::WithSession {
                session_id: "01990d9e-8400-7000-8000-000000000010".to_owned(),
                method: Box::new(method),
            };
            assert_eq!(code(scoped), Ok(()));
        }
    }

    #[test]
    fn 按需查看的线上格式() {
        let method = serde_json::to_value(IpcMethod::RoomHistory(IpcRoomHistoryRequest {
            room_id: None,
            before: None,
            after: None,
            limit: 20,
            from: None,
            mentions_me: false,
        }))
        .expect("能序列化");
        assert_eq!(method, serde_json::json!({"room_history": {"limit": 20}}));
        let messages = serde_json::to_value(IpcResponse::Messages {
            messages: Vec::new(),
            missing: vec!["$gone:matrix.test".to_owned()],
            more: Vec::new(),
        })
        .expect("能序列化");
        assert_eq!(
            messages,
            serde_json::json!({"type": "messages", "messages": [], "missing": ["$gone:matrix.test"]})
        );
        let page = serde_json::to_value(IpcResponse::RoomMessages {
            messages: Vec::new(),
            next_cursor: Some("$next:matrix.test".to_owned()),
        })
        .expect("能序列化");
        assert_eq!(
            page,
            serde_json::json!({"type": "room_messages", "messages": [], "nextCursor": "$next:matrix.test"})
        );
    }

    #[test]
    fn 补不回来的一段的线上格式_没有时不出现() {
        let page = |gaps| IpcResponse::MessagePreviews {
            previews: Vec::new(),
            next_cursor: None,
            typing: Vec::new(),
            gaps,
        };
        let gap = IpcTimelineGap {
            room_id: "!room:matrix.test".to_owned(),
            after_event_id: None,
            before_event_id: "$first:matrix.test".to_owned(),
            reason: "too_many".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(page(vec![gap.clone()])).expect("能序列化"),
            json!({
                "type": "message_previews", "previews": [],
                "gaps": [{"roomId": "!room:matrix.test", "beforeEventId": "$first:matrix.test", "reason": "too_many"}],
            })
        );
        assert_eq!(
            serde_json::to_value(page(Vec::new())).expect("能序列化"),
            json!({"type": "message_previews", "previews": []})
        );
        let parsed: IpcResponse = serde_json::from_value(json!({
            "type": "message_previews", "previews": [],
            "gaps": [{"roomId": "!room:matrix.test", "beforeEventId": "$first:matrix.test", "reason": "too_many"}],
        }))
        .expect("能解析");
        assert_eq!(parsed, page(vec![gap]));
    }

    #[test]
    fn 收件箱确认的线上格式和校验() {
        let event = "$done:matrix.test".to_owned();
        let method = IpcMethod::AckInbox(IpcAckInboxRequest { id: event.clone() });
        assert_eq!(method.required_scope(), IpcScope::PreviewsRead);
        assert!(method.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&method).expect("能序列化"),
            serde_json::json!({"ack_inbox": {"id": event}})
        );
        let reply = serde_json::to_value(IpcResponse::InboxAcknowledged {
            room_id: "!room:matrix.test".to_owned(),
            event_id: event.clone(),
            acknowledged: true,
            pending: 3,
        })
        .expect("能序列化");
        assert_eq!(
            reply,
            serde_json::json!({
                "type": "inbox_acknowledged", "roomId": "!room:matrix.test",
                "eventId": event, "acknowledged": true, "pending": 3,
            })
        );
        for id in ["", "$x", "not-a-uuid"] {
            let invalid = IpcMethod::AckInbox(IpcAckInboxRequest { id: id.to_owned() });
            assert_eq!(
                invalid
                    .validate()
                    .map_err(super::IpcMethodValidationFailure::code),
                Err("bridge.ipc.message_id_invalid"),
                "{id}"
            );
        }

        // 从确认位置开始只对收件箱有用；不带时线上格式不变。
        let request = |from_ack| IpcListPreviewsRequest {
            after_event_id: None,
            room_id: None,
            before_event_id: None,
            limit: 20,
            keep_waiting: false,
            wait_ms: None,
            from_ack,
        };
        assert!(IpcMethod::ReadInbox(request(true)).validate().is_ok());
        assert!(IpcMethod::WaitInbox(request(true)).validate().is_ok());
        assert_eq!(
            IpcMethod::ListPreviews(request(true))
                .validate()
                .map_err(super::IpcMethodValidationFailure::code),
            Err("bridge.ipc.event_cursor_invalid")
        );
        assert_eq!(
            serde_json::to_value(IpcMethod::ReadInbox(request(true))).expect("能序列化"),
            serde_json::json!({"read_inbox": {"afterEventId": null, "roomId": null, "beforeEventId": null, "limit": 20, "fromAck": true}})
        );
        assert_eq!(
            serde_json::to_value(IpcMethod::ReadInbox(request(false))).expect("能序列化"),
            serde_json::json!({"read_inbox": {"afterEventId": null, "roomId": null, "beforeEventId": null, "limit": 20}})
        );
    }

    #[test]
    fn 只有聊天能带_所有人() {
        let document = IpcMethod::SendMessage(IpcSendMessageRequest {
            chat: false,
            mentions: Vec::new(),
            mentions_everyone: true,
            submission_id: None,
            automation_grant_id: None,
            room_id: "!room:matrix.test".to_owned(),
            title: "长文".to_owned(),
            summary: "长文摘要".to_owned(),
            body: "正文".to_owned(),
            media_type: "text/markdown".to_owned(),
            language: Some("zh-CN".to_owned()),
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: Vec::new(),
            provenance: IpcMessageProvenance::HumanConfirmedAgent,
            reply_to_message_id: None,
        });
        assert_eq!(
            document.validate().expect_err("长文不能 @所有人").code(),
            "bridge.ipc.conversation_invalid"
        );
    }

    #[test]
    fn 写入方法在进入业务层前拒绝超限或畸形输入() {
        let invalid_status = IpcMethod::PublishStatus(IpcPublishStatusRequest {
            room_id: "!room:matrix.test".to_owned(),
            status: IpcWorkStatus::Working,
            task_summary: Some("越界".to_owned()),
            progress_basis_points: Some(10_001),
        });
        assert_eq!(
            invalid_status
                .validate()
                .expect_err("超限进度必须失败")
                .code(),
            "bridge.ipc.progress_invalid"
        );

        let invalid_handoff_page = IpcMethod::ListHandoffs(IpcListHandoffsRequest { limit: 0 });
        assert_eq!(
            invalid_handoff_page
                .validate()
                .expect_err("空页大小必须失败")
                .code(),
            "bridge.ipc.handoff_limit_invalid"
        );

        let invalid_message = IpcMethod::SendMessage(IpcSendMessageRequest {
            chat: false,
            mentions: Vec::new(),
            mentions_everyone: false,
            submission_id: None,
            automation_grant_id: None,
            room_id: "!room:matrix.test".to_owned(),
            title: "发送".to_owned(),
            summary: "受限摘要".to_owned(),
            body: "正文".to_owned(),
            media_type: "text/markdown".to_owned(),
            language: Some("zh-CN".to_owned()),
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: vec!["Bad-Flag".to_owned()],
            provenance: IpcMessageProvenance::HumanConfirmedAgent,
            reply_to_message_id: None,
        });
        assert_eq!(
            invalid_message
                .validate()
                .expect_err("畸形风险标签必须失败")
                .code(),
            "bridge.ipc.risk_flags_invalid"
        );

        let invalid_reply = IpcMethod::SendMessage(IpcSendMessageRequest {
            chat: false,
            mentions: Vec::new(),
            mentions_everyone: false,
            submission_id: None,
            automation_grant_id: None,
            room_id: "!room:matrix.test".to_owned(),
            title: "回复".to_owned(),
            summary: "回复摘要".to_owned(),
            body: "正文".to_owned(),
            media_type: "text/markdown".to_owned(),
            language: Some("zh-CN".to_owned()),
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: Vec::new(),
            provenance: IpcMessageProvenance::HumanConfirmedAgent,
            reply_to_message_id: Some("550e8400-e29b-41d4-a716-446655440000".to_owned()),
        });
        assert_eq!(
            invalid_reply
                .validate()
                .expect_err("非 UUIDv7 消息标识必须失败")
                .code(),
            "bridge.ipc.message_id_invalid"
        );

        let invalid_handoff = IpcMethod::ConsumeHandoff(IpcHandoffRequest {
            handoff_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
        });
        assert_eq!(
            invalid_handoff
                .validate()
                .expect_err("非 UUIDv7 交接标识必须失败")
                .code(),
            "bridge.ipc.handoff_id_invalid"
        );

        let duplicate_permissions = IpcMethod::ApproveHandoff(IpcApproveHandoffRequest {
            handoff_id: Uuid::now_v7().to_string(),
            principal_id: Uuid::now_v7().to_string(),
            room_id: "!room:matrix.test".to_owned(),
            source_content_id: Uuid::now_v7().to_string(),
            target_agent_id: Uuid::now_v7().to_string(),
            target_instance_id: Uuid::now_v7().to_string(),
            permissions: vec![
                IpcHandoffPermission::ReadText,
                IpcHandoffPermission::ReadText,
            ],
            purpose: IpcHandoffPurpose::Inspect,
            expires_at_unix_ms: 2_000,
        });
        assert_eq!(
            duplicate_permissions
                .validate()
                .expect_err("重复权限必须失败")
                .code(),
            "bridge.ipc.handoff_permissions_invalid"
        );
    }

    #[test]
    fn 自主发送与自动授权标识必须成对出现() {
        let missing_grant = automated_message(None, IpcMessageProvenance::AutonomousAgent);
        assert_eq!(
            missing_grant
                .validate()
                .expect_err("自主发送缺少授权必须失败")
                .code(),
            "bridge.ipc.automation_grant_required"
        );

        let misplaced_grant = automated_message(
            Some("0198b601-77a1-7bb8-83eb-a8fe68c97e47".to_owned()),
            IpcMessageProvenance::HumanConfirmedAgent,
        );
        assert_eq!(
            misplaced_grant
                .validate()
                .expect_err("人工发送不得挪用自动授权")
                .code(),
            "bridge.ipc.automation_grant_required"
        );
    }

    fn automated_message(
        automation_grant_id: Option<String>,
        provenance: IpcMessageProvenance,
    ) -> IpcMethod {
        IpcMethod::SendMessage(IpcSendMessageRequest {
            chat: false,
            mentions: Vec::new(),
            mentions_everyone: false,
            submission_id: None,
            automation_grant_id,
            room_id: "!room:matrix.test".to_owned(),
            title: "自动发送".to_owned(),
            summary: "授权绑定检查".to_owned(),
            body: "正文".to_owned(),
            media_type: "text/markdown".to_owned(),
            language: Some("zh-CN".to_owned()),
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: Vec::new(),
            provenance,
            reply_to_message_id: None,
        })
    }

    #[test]
    fn 方法参数拒绝未知字段且保持稳定线格式() {
        let method = serde_json::from_value::<IpcMethod>(json!({
            "list_previews": {
                "roomId": "!room:matrix.test",
                "beforeEventId": null,
                "limit": 20
            }
        }))
        .expect("闭合方法可解码");
        assert_eq!(method.name(), "list_previews");

        let unknown = serde_json::from_value::<IpcMethod>(json!({
            "open_content": {
                "contentId": Uuid::from_u128(1).to_string(),
                "unexpected": true
            }
        }));
        assert!(unknown.is_err());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcConversationMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_name: Option<String>,
    pub text: String,
    pub mentions: Vec<String>,
    /// 一批新消息里正文太长、只给了开头时为 true；全文按 ID 取。
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// 只给了开头时，全文有多少个字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_length: Option<u32>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde 的 skip_serializing_if 只接受引用。
const fn is_false(value: &bool) -> bool {
    !*value
}
