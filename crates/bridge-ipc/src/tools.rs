use agent_room_bridge_core::ipc::IpcScope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::limits;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMethod {
    BridgeStatus,
    GetSelf,
    ListPreviews(IpcListPreviewsRequest),
    GetPresence(IpcGetPresenceRequest),
    OpenContent(IpcOpenContentRequest),
    PublishStatus(IpcPublishStatusRequest),
    SendMessage(IpcSendMessageRequest),
    ApproveHandoff(IpcApproveHandoffRequest),
    ConsumeHandoff(IpcHandoffRequest),
    DeclineHandoff(IpcHandoffRequest),
}

impl IpcMethod {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::BridgeStatus => "bridge_status",
            Self::GetSelf => "get_self",
            Self::ListPreviews(_) => "list_previews",
            Self::GetPresence(_) => "get_presence",
            Self::OpenContent(_) => "open_content",
            Self::PublishStatus(_) => "publish_status",
            Self::SendMessage(_) => "send_message",
            Self::ApproveHandoff(_) => "approve_handoff",
            Self::ConsumeHandoff(_) => "consume_handoff",
            Self::DeclineHandoff(_) => "decline_handoff",
        }
    }

    pub const fn required_scope(&self) -> IpcScope {
        match self {
            Self::BridgeStatus => IpcScope::BridgeStatusRead,
            Self::GetSelf => IpcScope::SelfRead,
            Self::ListPreviews(_) => IpcScope::PreviewsRead,
            Self::GetPresence(_) => IpcScope::PresenceRead,
            Self::OpenContent(_) => IpcScope::ContentRead,
            Self::PublishStatus(_) => IpcScope::StatusPublish,
            Self::SendMessage(_) => IpcScope::MessageSend,
            Self::ApproveHandoff(_) => IpcScope::HandoffApprove,
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
            Self::BridgeStatus | Self::GetSelf => Ok(()),
            Self::ListPreviews(request) => request.validate(),
            Self::GetPresence(request) => request.validate(),
            Self::OpenContent(request) => request.validate(),
            Self::PublishStatus(request) => request.validate(),
            Self::SendMessage(request) => request.validate(),
            Self::ApproveHandoff(request) => request.validate(),
            Self::ConsumeHandoff(request) | Self::DeclineHandoff(request) => request.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcListPreviewsRequest {
    pub room_id: Option<String>,
    pub before_event_id: Option<String>,
    pub limit: u16,
}

impl IpcListPreviewsRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcGetPresenceRequest {
    pub room_id: String,
    #[serde(default)]
    pub agent_ids: Vec<String>,
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
        self.agent_ids
            .iter()
            .try_for_each(|value| validate_uuid(value, "bridge.ipc.agent_id_invalid"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenContentRequest {
    pub content_id: String,
}

impl IpcOpenContentRequest {
    fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcResponse {
    BridgeStatus {
        state: IpcBridgeState,
        #[serde(rename = "startedAtUnixMs")]
        started_at_unix_ms: i64,
    },
    SelfSummary {
        summary: IpcSelfSummary,
    },
    MessagePreviews {
        previews: Vec<IpcMessagePreviewSummary>,
        #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
    },
    Presence {
        entries: Vec<IpcPresenceSummary>,
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
    ConsumedHandoff {
        handoff: IpcConsumedHandoff,
    },
    DeclinedHandoff {
        handoff: IpcDeclinedHandoff,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcSelfSummary {
    pub agent: IpcAgentSummary,
    pub instance_id: String,
    pub matrix_device_id: String,
    pub connection_state: IpcBridgeState,
    pub granted_capabilities: Vec<String>,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcActorSummary {
    pub agent: IpcAgentSummary,
    pub instance_id: String,
    pub provenance: IpcMessageProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcContentReference {
    pub content_id: String,
    pub digest_sha256: String,
    pub media_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMessagePreviewSummary {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcDeclinedHandoff {
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

const fn failure(code: &'static str) -> IpcMethodValidationFailure {
    IpcMethodValidationFailure { code }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::IpcScope;
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        IpcApproveHandoffRequest, IpcHandoffPermission, IpcHandoffPurpose, IpcHandoffRequest,
        IpcListPreviewsRequest, IpcMessageProvenance, IpcMessageSensitivity, IpcMethod,
        IpcPublishStatusRequest, IpcSendMessageRequest, IpcWorkStatus,
    };

    #[test]
    fn 每个工具方法映射到独立最小作用域() {
        let id = Uuid::now_v7().to_string();
        let methods = [
            (IpcMethod::GetSelf, IpcScope::SelfRead),
            (
                IpcMethod::ListPreviews(IpcListPreviewsRequest {
                    room_id: None,
                    before_event_id: None,
                    limit: 20,
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

        let invalid_message = IpcMethod::SendMessage(IpcSendMessageRequest {
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
