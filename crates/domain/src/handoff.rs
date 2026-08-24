use std::collections::BTreeSet;

use crate::{
    DomainError, DomainResult,
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    messages::{MessageProvenance, MessageRiskFlags},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

const MAX_MATRIX_EVENT_ID_BYTES: usize = 512;
const MAX_FAILURE_CODE_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandoffPermission {
    ReadText,
    ReadAttachments,
    IncludeMetadata,
}

impl HandoffPermission {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadText => "read_text",
            Self::ReadAttachments => "read_attachments",
            Self::IncludeMetadata => "include_metadata",
        }
    }
}

impl TryFrom<&str> for HandoffPermission {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "read_text" => Ok(Self::ReadText),
            "read_attachments" => Ok(Self::ReadAttachments),
            "include_metadata" => Ok(Self::IncludeMetadata),
            _ => Err(DomainError::Validation {
                field: "handoff_permission",
                reason: "不是支持的内容范围",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffPermissions(BTreeSet<HandoffPermission>);

impl HandoffPermissions {
    /// 创建非空、去重的上下文读取范围。
    ///
    /// # Errors
    ///
    /// 没有选择任何内容范围时返回校验错误。
    pub fn new(permissions: impl IntoIterator<Item = HandoffPermission>) -> DomainResult<Self> {
        let permissions = permissions.into_iter().collect::<BTreeSet<_>>();
        if permissions.is_empty() {
            return Err(DomainError::Validation {
                field: "handoff_permissions",
                reason: "至少选择一个内容范围",
            });
        }
        Ok(Self(permissions))
    }

    pub fn contains(&self, permission: HandoffPermission) -> bool {
        self.0.contains(&permission)
    }

    pub fn iter(&self) -> impl Iterator<Item = HandoffPermission> + '_ {
        self.0.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffPurpose {
    Inspect,
    Summarize,
    ReplyDraft,
}

impl HandoffPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Summarize => "summarize",
            Self::ReplyDraft => "reply_draft",
        }
    }
}

impl TryFrom<&str> for HandoffPurpose {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "inspect" => Ok(Self::Inspect),
            "summarize" => Ok(Self::Summarize),
            "reply_draft" => Ok(Self::ReplyDraft),
            _ => Err(DomainError::Validation {
                field: "handoff_purpose",
                reason: "不是支持的交付用途",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffSourceEventId(String);

impl HandoffSourceEventId {
    /// 创建受限 Matrix 事件标识。
    ///
    /// # Errors
    ///
    /// 标识缺少 `$` 前缀、为空、超长或包含控制字符时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if !(4..=MAX_MATRIX_EVENT_ID_BYTES).contains(&value.len())
            || !value.starts_with('$')
            || value.chars().any(char::is_control)
        {
            return Err(DomainError::Validation {
                field: "handoff_source_event_id",
                reason: "不是合法的 Matrix 事件标识",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffSourceActor {
    agent_id: AgentId,
    instance_id: AgentInstanceId,
    provenance: MessageProvenance,
}

impl HandoffSourceActor {
    pub const fn new(
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        provenance: MessageProvenance,
    ) -> Self {
        Self {
            agent_id,
            instance_id,
            provenance,
        }
    }

    pub const fn agent_id(self) -> AgentId {
        self.agent_id
    }

    pub const fn instance_id(self) -> AgentInstanceId {
        self.instance_id
    }

    pub const fn provenance(self) -> MessageProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffSource {
    room_id: MatrixRoomReference,
    event_id: HandoffSourceEventId,
    message_id: MessageId,
    actor: HandoffSourceActor,
}

impl HandoffSource {
    pub const fn new(
        room_id: MatrixRoomReference,
        event_id: HandoffSourceEventId,
        message_id: MessageId,
        actor: HandoffSourceActor,
    ) -> Self {
        Self {
            room_id,
            event_id,
            message_id,
            actor,
        }
    }

    pub const fn room_id(&self) -> &MatrixRoomReference {
        &self.room_id
    }

    pub const fn event_id(&self) -> &HandoffSourceEventId {
        &self.event_id
    }

    pub const fn message_id(&self) -> MessageId {
        self.message_id
    }

    pub const fn actor(&self) -> HandoffSourceActor {
        self.actor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffContentReference {
    content_id: ContentId,
    digest: Sha256Digest,
    byte_length: ContentByteLength,
    media_type: ContentMediaType,
}

impl HandoffContentReference {
    pub const fn new(
        content_id: ContentId,
        digest: Sha256Digest,
        byte_length: ContentByteLength,
        media_type: ContentMediaType,
    ) -> Self {
        Self {
            content_id,
            digest,
            byte_length,
            media_type,
        }
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub const fn byte_length(&self) -> ContentByteLength {
        self.byte_length
    }

    pub const fn media_type(&self) -> &ContentMediaType {
        &self.media_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffFailureCode(String);

impl HandoffFailureCode {
    /// 创建可安全进入协议和审计记录的稳定失败码。
    ///
    /// # Errors
    ///
    /// 失败码不是长度受限的小写点分标识时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.')
            });
        if value.len() > MAX_FAILURE_CODE_BYTES || !valid {
            return Err(DomainError::Validation {
                field: "handoff_failure_code",
                reason: "必须是长度受限的小写点分标识",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffStatus {
    Proposed,
    Approved,
    Delivered,
    Consumed,
    Declined,
    Revoked,
    Expired,
    Failed,
}

impl HandoffStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Delivered => "delivered",
            Self::Consumed => "consumed",
            Self::Declined => "declined",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHandoffFields {
    pub id: HandoffId,
    pub requester_agent_id: AgentId,
    pub requester_instance_id: AgentInstanceId,
    pub source: HandoffSource,
    pub target_agent_id: AgentId,
    pub target_instance_id: AgentInstanceId,
    pub content: HandoffContentReference,
    pub permissions: HandoffPermissions,
    pub purpose: HandoffPurpose,
    pub risk_flags: MessageRiskFlags,
    pub proposed_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHandoff {
    fields: ContextHandoffFields,
    status: HandoffStatus,
    approved_by_principal_id: Option<PrincipalId>,
    approved_at: Option<UtcMillis>,
    delivered_at: Option<UtcMillis>,
    consumed_at: Option<UtcMillis>,
    resolved_at: Option<UtcMillis>,
    failure_code: Option<HandoffFailureCode>,
}

impl ContextHandoff {
    /// 创建尚未获得用户授权的上下文交付提案。
    ///
    /// # Errors
    ///
    /// 到期时间不晚于提案时间时返回校验错误。
    pub fn propose(fields: ContextHandoffFields) -> DomainResult<Self> {
        if fields.expires_at <= fields.proposed_at {
            return Err(DomainError::Validation {
                field: "handoff_expires_at",
                reason: "必须晚于提案时间",
            });
        }
        Ok(Self {
            fields,
            status: HandoffStatus::Proposed,
            approved_by_principal_id: None,
            approved_at: None,
            delivered_at: None,
            consumed_at: None,
            resolved_at: None,
            failure_code: None,
        })
    }

    pub const fn fields(&self) -> &ContextHandoffFields {
        &self.fields
    }

    pub const fn status(&self) -> HandoffStatus {
        self.status
    }

    pub const fn approved_by_principal_id(&self) -> Option<PrincipalId> {
        self.approved_by_principal_id
    }

    pub const fn approved_at(&self) -> Option<UtcMillis> {
        self.approved_at
    }

    pub const fn delivered_at(&self) -> Option<UtcMillis> {
        self.delivered_at
    }

    pub const fn consumed_at(&self) -> Option<UtcMillis> {
        self.consumed_at
    }

    pub const fn resolved_at(&self) -> Option<UtcMillis> {
        self.resolved_at
    }

    pub const fn failure_code(&self) -> Option<&HandoffFailureCode> {
        self.failure_code.as_ref()
    }

    /// 记录用户对精确目标、范围、用途和期限的明确批准。
    ///
    /// # Errors
    ///
    /// 提案已到期、已离开提案态或重复批准主体不一致时返回错误。
    pub fn approve(
        &mut self,
        principal_id: PrincipalId,
        approved_at: UtcMillis,
    ) -> DomainResult<()> {
        if self.status == HandoffStatus::Approved {
            if self.approved_by_principal_id == Some(principal_id) {
                return Ok(());
            }
            return Err(DomainError::Forbidden {
                action: "以不同主体重复批准上下文交付",
            });
        }
        self.require_status(HandoffStatus::Proposed, HandoffStatus::Approved)?;
        self.validate_active_time(approved_at, self.fields.proposed_at, "handoff_approved_at")?;
        self.status = HandoffStatus::Approved;
        self.approved_by_principal_id = Some(principal_id);
        self.approved_at = Some(approved_at);
        Ok(())
    }

    /// 标记目标 Bridge 已建立本地一次性上下文包。
    ///
    /// # Errors
    ///
    /// 未获批准、已经终结或交付时间无效时返回错误。
    pub fn mark_delivered(&mut self, delivered_at: UtcMillis) -> DomainResult<()> {
        if self.status == HandoffStatus::Delivered {
            return Ok(());
        }
        self.require_status(HandoffStatus::Approved, HandoffStatus::Delivered)?;
        let approved_at = self.approved_at.ok_or(DomainError::InvariantViolation {
            entity: "context_handoff",
            rule: "批准态必须记录批准时间",
        })?;
        self.validate_active_time(delivered_at, approved_at, "handoff_delivered_at")?;
        self.status = HandoffStatus::Delivered;
        self.delivered_at = Some(delivered_at);
        Ok(())
    }

    /// 消费并终结本地上下文包，重复消费回执保持幂等。
    ///
    /// # Errors
    ///
    /// 包尚未交付、已经终结或消费时间无效时返回错误。
    pub fn consume(&mut self, consumed_at: UtcMillis) -> DomainResult<()> {
        if self.status == HandoffStatus::Consumed {
            return Ok(());
        }
        self.require_status(HandoffStatus::Delivered, HandoffStatus::Consumed)?;
        let delivered_at = self.delivered_at.ok_or(DomainError::InvariantViolation {
            entity: "context_handoff",
            rule: "已交付状态必须记录交付时间",
        })?;
        self.validate_active_time(consumed_at, delivered_at, "handoff_consumed_at")?;
        self.status = HandoffStatus::Consumed;
        self.consumed_at = Some(consumed_at);
        self.resolved_at = Some(consumed_at);
        Ok(())
    }

    /// 拒绝提案或销毁尚未消费的本地上下文包。
    ///
    /// # Errors
    ///
    /// 已消费或其他终态不能改写为拒绝。
    pub fn decline(&mut self, declined_at: UtcMillis) -> DomainResult<()> {
        if self.status == HandoffStatus::Declined {
            return Ok(());
        }
        if !matches!(
            self.status,
            HandoffStatus::Proposed | HandoffStatus::Approved | HandoffStatus::Delivered
        ) {
            return Err(self.invalid_transition(HandoffStatus::Declined));
        }
        self.validate_active_time(
            declined_at,
            self.latest_active_time(),
            "handoff_declined_at",
        )?;
        self.status = HandoffStatus::Declined;
        self.resolved_at = Some(declined_at);
        Ok(())
    }

    /// 撤销已经批准但尚未消费的上下文交付。
    ///
    /// # Errors
    ///
    /// 未批准提案、已消费或其他终态不能撤销。
    pub fn revoke(&mut self, revoked_at: UtcMillis) -> DomainResult<()> {
        if self.status == HandoffStatus::Revoked {
            return Ok(());
        }
        if !matches!(
            self.status,
            HandoffStatus::Approved | HandoffStatus::Delivered
        ) {
            return Err(self.invalid_transition(HandoffStatus::Revoked));
        }
        self.validate_active_time(revoked_at, self.latest_active_time(), "handoff_revoked_at")?;
        self.status = HandoffStatus::Revoked;
        self.resolved_at = Some(revoked_at);
        Ok(())
    }

    /// 在授权期限到达后关闭尚未终结的交付。
    ///
    /// # Errors
    ///
    /// 尚未到期或已经进入其他终态时返回错误。
    pub fn expire(&mut self, observed_at: UtcMillis) -> DomainResult<()> {
        if self.status == HandoffStatus::Expired {
            return Ok(());
        }
        if !matches!(
            self.status,
            HandoffStatus::Proposed | HandoffStatus::Approved | HandoffStatus::Delivered
        ) {
            return Err(self.invalid_transition(HandoffStatus::Expired));
        }
        if observed_at < self.fields.expires_at {
            return Err(DomainError::InvariantViolation {
                entity: "context_handoff",
                rule: "尚未到达交付授权期限",
            });
        }
        self.status = HandoffStatus::Expired;
        self.resolved_at = Some(observed_at);
        Ok(())
    }

    /// 记录批准后的稳定投递失败，重复相同失败保持幂等。
    ///
    /// # Errors
    ///
    /// 未批准、已交付、已终结、时间无效或失败码冲突时返回错误。
    pub fn fail(
        &mut self,
        failure_code: HandoffFailureCode,
        failed_at: UtcMillis,
    ) -> DomainResult<()> {
        if self.status == HandoffStatus::Failed {
            if self.failure_code.as_ref() == Some(&failure_code) {
                return Ok(());
            }
            return Err(DomainError::InvariantViolation {
                entity: "context_handoff",
                rule: "失败终态不能改写失败原因",
            });
        }
        self.require_status(HandoffStatus::Approved, HandoffStatus::Failed)?;
        let approved_at = self.approved_at.ok_or(DomainError::InvariantViolation {
            entity: "context_handoff",
            rule: "批准态必须记录批准时间",
        })?;
        self.validate_active_time(failed_at, approved_at, "handoff_failed_at")?;
        self.status = HandoffStatus::Failed;
        self.failure_code = Some(failure_code);
        self.resolved_at = Some(failed_at);
        Ok(())
    }

    pub fn is_consumable_at(&self, observed_at: UtcMillis) -> bool {
        self.status == HandoffStatus::Delivered && observed_at < self.fields.expires_at
    }

    fn require_status(&self, expected: HandoffStatus, target: HandoffStatus) -> DomainResult<()> {
        if self.status != expected {
            return Err(self.invalid_transition(target));
        }
        Ok(())
    }

    fn validate_active_time(
        &self,
        value: UtcMillis,
        earliest: UtcMillis,
        field: &'static str,
    ) -> DomainResult<()> {
        if value < earliest {
            return Err(DomainError::Validation {
                field,
                reason: "不能早于前一阶段",
            });
        }
        if value >= self.fields.expires_at {
            return Err(DomainError::InvariantViolation {
                entity: "context_handoff",
                rule: "交付授权已经到期",
            });
        }
        Ok(())
    }

    fn latest_active_time(&self) -> UtcMillis {
        self.delivered_at
            .or(self.approved_at)
            .unwrap_or(self.fields.proposed_at)
    }

    const fn invalid_transition(&self, target: HandoffStatus) -> DomainError {
        DomainError::InvalidTransition {
            entity: "context_handoff",
            from: self.status.as_str(),
            to: target.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffFailureCode,
        HandoffPermission, HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
        HandoffSourceEventId, HandoffStatus,
    };
    use crate::{
        content::{ContentByteLength, ContentMediaType, Sha256Digest},
        ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
        messages::{MessageProvenance, MessageRiskFlag, MessageRiskFlags},
        rooms::MatrixRoomReference,
        time::UtcMillis,
    };

    #[test]
    fn 未经用户批准不能投递或消费() {
        let mut handoff = proposed_handoff();

        assert!(handoff.mark_delivered(time(1_200)).is_err());
        assert!(handoff.consume(time(1_300)).is_err());
        assert_eq!(handoff.status(), HandoffStatus::Proposed);
        assert!(handoff.approved_by_principal_id().is_none());
    }

    #[test]
    fn 批准投递消费形成单向且幂等的生命周期() {
        let mut handoff = proposed_handoff();
        let principal_id = PrincipalId::from_uuid(Uuid::from_u128(10));

        handoff
            .approve(principal_id, time(1_100))
            .expect("批准有效");
        handoff
            .approve(principal_id, time(1_150))
            .expect("重复批准幂等");
        handoff.mark_delivered(time(1_200)).expect("交付有效");
        handoff.mark_delivered(time(1_250)).expect("重复交付幂等");
        assert!(handoff.is_consumable_at(time(1_999)));
        handoff.consume(time(1_300)).expect("消费有效");
        handoff.consume(time(2_500)).expect("终态回执幂等");

        assert_eq!(handoff.status(), HandoffStatus::Consumed);
        assert_eq!(handoff.approved_at(), Some(time(1_100)));
        assert_eq!(handoff.delivered_at(), Some(time(1_200)));
        assert_eq!(handoff.consumed_at(), Some(time(1_300)));
        assert!(!handoff.is_consumable_at(time(1_301)));
        assert!(handoff.revoke(time(1_400)).is_err());
    }

    #[test]
    fn 到期时间是硬边界且不能覆盖既有终态() {
        let mut handoff = proposed_handoff();
        assert!(handoff.expire(time(1_999)).is_err());
        handoff.expire(time(2_000)).expect("到期瞬间关闭");
        handoff.expire(time(2_100)).expect("重复到期幂等");

        assert_eq!(handoff.status(), HandoffStatus::Expired);
        assert!(
            handoff
                .approve(PrincipalId::from_uuid(Uuid::from_u128(10)), time(2_100))
                .is_err()
        );
    }

    #[test]
    fn 拒绝撤销与失败保留各自终态原因() {
        let mut declined = proposed_handoff();
        declined.decline(time(1_050)).expect("用户可以拒绝提案");
        assert_eq!(declined.status(), HandoffStatus::Declined);

        let mut revoked = approved_handoff();
        revoked.mark_delivered(time(1_200)).expect("交付有效");
        revoked.revoke(time(1_300)).expect("消费前允许撤销");
        assert_eq!(revoked.status(), HandoffStatus::Revoked);

        let mut failed = approved_handoff();
        let failure = HandoffFailureCode::new("handoff.target_unreachable").expect("失败码有效");
        failed.fail(failure.clone(), time(1_200)).expect("失败有效");
        failed.fail(failure, time(1_300)).expect("相同失败幂等");
        assert_eq!(failed.status(), HandoffStatus::Failed);
        assert_eq!(
            failed.failure_code().map(HandoffFailureCode::as_str),
            Some("handoff.target_unreachable")
        );
        assert!(
            failed
                .fail(
                    HandoffFailureCode::new("handoff.timeout").expect("失败码有效"),
                    time(1_300),
                )
                .is_err()
        );
    }

    #[test]
    fn 内容范围来源标识和批准主体拒绝宽松输入() {
        assert!(HandoffPermissions::new([]).is_err());
        assert!(HandoffSourceEventId::new("javascript:alert(1)").is_err());
        assert!(HandoffFailureCode::new("HANDOFF-FAILED").is_err());

        let mut handoff = approved_handoff();
        assert!(
            handoff
                .approve(PrincipalId::from_uuid(Uuid::from_u128(11)), time(1_150))
                .is_err()
        );
    }

    fn approved_handoff() -> ContextHandoff {
        let mut handoff = proposed_handoff();
        handoff
            .approve(PrincipalId::from_uuid(Uuid::from_u128(10)), time(1_100))
            .expect("批准有效");
        handoff
    }

    fn proposed_handoff() -> ContextHandoff {
        ContextHandoff::propose(ContextHandoffFields {
            id: HandoffId::from_uuid(Uuid::from_u128(1)),
            requester_agent_id: AgentId::from_uuid(Uuid::from_u128(2)),
            requester_instance_id: AgentInstanceId::from_uuid(Uuid::from_u128(3)),
            source: HandoffSource::new(
                MatrixRoomReference::new("!builders:agent-room.test").expect("房间有效"),
                HandoffSourceEventId::new("$source:agent-room.test").expect("事件有效"),
                MessageId::from_uuid(Uuid::from_u128(4)),
                HandoffSourceActor::new(
                    AgentId::from_uuid(Uuid::from_u128(5)),
                    AgentInstanceId::from_uuid(Uuid::from_u128(6)),
                    MessageProvenance::AutonomousAgent,
                ),
            ),
            target_agent_id: AgentId::from_uuid(Uuid::from_u128(2)),
            target_instance_id: AgentInstanceId::from_uuid(Uuid::from_u128(7)),
            content: HandoffContentReference::new(
                ContentId::from_uuid(Uuid::from_u128(8)),
                Sha256Digest::from_bytes([9; 32]),
                ContentByteLength::new(128).expect("长度有效"),
                ContentMediaType::new("text/markdown").expect("媒体类型有效"),
            ),
            permissions: HandoffPermissions::new([
                HandoffPermission::ReadText,
                HandoffPermission::IncludeMetadata,
            ])
            .expect("范围有效"),
            purpose: HandoffPurpose::Summarize,
            risk_flags: MessageRiskFlags::new([
                MessageRiskFlag::new("untrusted_instructions").expect("风险标签有效")
            ])
            .expect("风险集合有效"),
            proposed_at: time(1_000),
            expires_at: time(2_000),
        })
        .expect("提案有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("时间有效")
    }
}
