use crate::{
    DomainError, DomainResult,
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    time::UtcMillis,
};

const MAX_TARGET_REFERENCE_LENGTH: usize = 1_024;
const MAX_DESCRIPTION_LENGTH: usize = 4_096;
const MAX_EVENT_REFERENCE_LENGTH: usize = 1_024;
const MAX_SUBMITTED_EXCERPT_LENGTH: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationTargetKind {
    Principal,
    Agent,
    Room,
    Event,
    FederationPeer,
}

impl ModerationTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Principal => "principal",
            Self::Agent => "agent",
            Self::Room => "room",
            Self::Event => "event",
            Self::FederationPeer => "federation_peer",
        }
    }
}

impl TryFrom<&str> for ModerationTargetKind {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "principal" => Ok(Self::Principal),
            "agent" => Ok(Self::Agent),
            "room" => Ok(Self::Room),
            "event" => Ok(Self::Event),
            "federation_peer" => Ok(Self::FederationPeer),
            _ => Err(validation(
                "moderation_target_kind",
                "不是支持的治理目标类别",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationTarget {
    kind: ModerationTargetKind,
    reference: String,
}

impl ModerationTarget {
    /// 创建不包含正文的治理目标引用。
    ///
    /// # Errors
    ///
    /// 引用为空、包含控制字符或超过上限时返回错误。
    pub fn new(kind: ModerationTargetKind, reference: impl Into<String>) -> DomainResult<Self> {
        let reference = reference.into();
        validate_text(
            "moderation_target_reference",
            &reference,
            MAX_TARGET_REFERENCE_LENGTH,
            false,
        )?;
        Ok(Self { kind, reference })
    }

    pub const fn kind(&self) -> ModerationTargetKind {
        self.kind
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationReason {
    Spam,
    Harassment,
    Impersonation,
    MaliciousContent,
    PrivacyViolation,
    UnsafeAutomation,
    Other,
}

impl ModerationReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Harassment => "harassment",
            Self::Impersonation => "impersonation",
            Self::MaliciousContent => "malicious_content",
            Self::PrivacyViolation => "privacy_violation",
            Self::UnsafeAutomation => "unsafe_automation",
            Self::Other => "other",
        }
    }
}

impl TryFrom<&str> for ModerationReason {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "spam" => Ok(Self::Spam),
            "harassment" => Ok(Self::Harassment),
            "impersonation" => Ok(Self::Impersonation),
            "malicious_content" => Ok(Self::MaliciousContent),
            "privacy_violation" => Ok(Self::PrivacyViolation),
            "unsafe_automation" => Ok(Self::UnsafeAutomation),
            "other" => Ok(Self::Other),
            _ => Err(validation("moderation_reason", "不是支持的举报原因")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationEvidence {
    room_catalog_id: Option<RoomCatalogId>,
    matrix_event_id: Option<String>,
    reporter_submitted_excerpt: Option<String>,
    end_to_end_encrypted: bool,
}

impl ModerationEvidence {
    /// 创建最小举报证据。
    ///
    /// 正文摘录只接受调用者显式提交的字段；空白摘录不会被保存。
    ///
    /// # Errors
    ///
    /// 事件引用或显式摘录包含控制字符、为空或超过上限时返回错误。
    pub fn new(
        room_catalog_id: Option<RoomCatalogId>,
        matrix_event_id: Option<String>,
        reporter_submitted_excerpt: Option<String>,
        end_to_end_encrypted: bool,
    ) -> DomainResult<Self> {
        if let Some(event_id) = matrix_event_id.as_deref() {
            validate_text(
                "moderation_evidence_event_id",
                event_id,
                MAX_EVENT_REFERENCE_LENGTH,
                false,
            )?;
        }
        if let Some(excerpt) = reporter_submitted_excerpt.as_deref() {
            validate_text(
                "moderation_evidence_excerpt",
                excerpt,
                MAX_SUBMITTED_EXCERPT_LENGTH,
                false,
            )?;
        }
        if reporter_submitted_excerpt.is_some() && matrix_event_id.is_none() {
            return Err(validation(
                "moderation_evidence_excerpt",
                "显式正文证据必须绑定一个 Matrix 事件引用",
            ));
        }
        Ok(Self {
            room_catalog_id,
            matrix_event_id,
            reporter_submitted_excerpt,
            end_to_end_encrypted,
        })
    }

    pub const fn room_catalog_id(&self) -> Option<RoomCatalogId> {
        self.room_catalog_id
    }

    pub fn matrix_event_id(&self) -> Option<&str> {
        self.matrix_event_id.as_deref()
    }

    pub fn reporter_submitted_excerpt(&self) -> Option<&str> {
        self.reporter_submitted_excerpt.as_deref()
    }

    pub const fn end_to_end_encrypted(&self) -> bool {
        self.end_to_end_encrypted
    }

    pub const fn discloses_plaintext(&self) -> bool {
        self.reporter_submitted_excerpt.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationCaseState {
    Open,
    InReview,
    Resolved,
    Dismissed,
}

impl ModerationCaseState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InReview => "in_review",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Resolved | Self::Dismissed)
    }
}

impl TryFrom<&str> for ModerationCaseState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open" => Ok(Self::Open),
            "in_review" => Ok(Self::InReview),
            "resolved" => Ok(Self::Resolved),
            "dismissed" => Ok(Self::Dismissed),
            _ => Err(validation("moderation_case_state", "不是支持的案件状态")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationCase {
    id: ModerationCaseId,
    reporter_principal_id: PrincipalId,
    target: ModerationTarget,
    reason: ModerationReason,
    description: String,
    evidence: ModerationEvidence,
    state: ModerationCaseState,
    created_at: UtcMillis,
    resolved_at: Option<UtcMillis>,
}

impl ModerationCase {
    /// 创建一个仅包含用户明确提供信息的举报案件。
    ///
    /// # Errors
    ///
    /// 描述包含控制字符或超过长度上限时返回错误。
    pub fn open(
        id: ModerationCaseId,
        reporter_principal_id: PrincipalId,
        target: ModerationTarget,
        reason: ModerationReason,
        description: impl Into<String>,
        evidence: ModerationEvidence,
        created_at: UtcMillis,
    ) -> DomainResult<Self> {
        let description = description.into();
        validate_text(
            "moderation_case_description",
            &description,
            MAX_DESCRIPTION_LENGTH,
            true,
        )?;
        Ok(Self {
            id,
            reporter_principal_id,
            target,
            reason,
            description,
            evidence,
            state: ModerationCaseState::Open,
            created_at,
            resolved_at: None,
        })
    }

    /// 从权威存储恢复案件。
    ///
    /// # Errors
    ///
    /// 终态与解决时间不一致时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: ModerationCaseId,
        reporter_principal_id: PrincipalId,
        target: ModerationTarget,
        reason: ModerationReason,
        description: impl Into<String>,
        evidence: ModerationEvidence,
        state: ModerationCaseState,
        created_at: UtcMillis,
        resolved_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        let mut case = Self::open(
            id,
            reporter_principal_id,
            target,
            reason,
            description,
            evidence,
            created_at,
        )?;
        if state.is_terminal() != resolved_at.is_some()
            || resolved_at.is_some_and(|resolved_at| resolved_at < created_at)
        {
            return Err(invariant(
                "moderation_case",
                "案件终态必须与解决时间一致且不能早于创建时间",
            ));
        }
        case.state = state;
        case.resolved_at = resolved_at;
        Ok(case)
    }

    pub const fn id(&self) -> ModerationCaseId {
        self.id
    }

    pub const fn reporter_principal_id(&self) -> PrincipalId {
        self.reporter_principal_id
    }

    pub const fn target(&self) -> &ModerationTarget {
        &self.target
    }

    pub const fn reason(&self) -> ModerationReason {
        self.reason
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub const fn evidence(&self) -> &ModerationEvidence {
        &self.evidence
    }

    pub const fn state(&self) -> ModerationCaseState {
        self.state
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.created_at
    }

    pub const fn resolved_at(&self) -> Option<UtcMillis> {
        self.resolved_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationActionKind {
    Hide,
    Mute,
    Kick,
    Ban,
}

impl ModerationActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::Mute => "mute",
            Self::Kick => "kick",
            Self::Ban => "ban",
        }
    }

    pub const fn target_kind(self) -> ModerationTargetKind {
        match self {
            Self::Hide => ModerationTargetKind::Event,
            Self::Mute | Self::Kick | Self::Ban => ModerationTargetKind::Principal,
        }
    }
}

impl TryFrom<&str> for ModerationActionKind {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "hide" => Ok(Self::Hide),
            "mute" => Ok(Self::Mute),
            "kick" => Ok(Self::Kick),
            "ban" => Ok(Self::Ban),
            _ => Err(validation(
                "moderation_action_kind",
                "不是支持的房间治理动作",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationActionStatus {
    Pending,
    Applied,
    Failed,
    Reversed,
}

impl ModerationActionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied => "applied",
            Self::Failed => "failed",
            Self::Reversed => "reversed",
        }
    }
}

impl TryFrom<&str> for ModerationActionStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "applied" => Ok(Self::Applied),
            "failed" => Ok(Self::Failed),
            "reversed" => Ok(Self::Reversed),
            _ => Err(validation(
                "moderation_action_status",
                "不是支持的治理动作状态",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationAction {
    id: ModerationActionId,
    case_id: Option<ModerationCaseId>,
    actor_principal_id: PrincipalId,
    room_catalog_id: RoomCatalogId,
    kind: ModerationActionKind,
    target: ModerationTarget,
    reason: ModerationReason,
    starts_at: UtcMillis,
    expires_at: Option<UtcMillis>,
    status: ModerationActionStatus,
    failure_code: Option<String>,
    reversed_at: Option<UtcMillis>,
}

impl ModerationAction {
    /// 预留一个尚未产生外部副作用的治理动作。
    ///
    /// # Errors
    ///
    /// 动作与目标类别不匹配，或到期时间无效时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn reserve(
        id: ModerationActionId,
        case_id: Option<ModerationCaseId>,
        actor_principal_id: PrincipalId,
        room_catalog_id: RoomCatalogId,
        kind: ModerationActionKind,
        target: ModerationTarget,
        reason: ModerationReason,
        starts_at: UtcMillis,
        expires_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        if target.kind() != kind.target_kind() {
            return Err(validation(
                "moderation_action_target",
                "治理动作与目标类别不匹配",
            ));
        }
        if expires_at.is_some_and(|expires_at| expires_at <= starts_at) {
            return Err(validation(
                "moderation_action_expires_at",
                "到期时间必须晚于开始时间",
            ));
        }
        Ok(Self {
            id,
            case_id,
            actor_principal_id,
            room_catalog_id,
            kind,
            target,
            reason,
            starts_at,
            expires_at,
            status: ModerationActionStatus::Pending,
            failure_code: None,
            reversed_at: None,
        })
    }

    /// 从权威存储恢复治理动作。
    ///
    /// # Errors
    ///
    /// 状态、失败码与撤销时间互相矛盾时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: ModerationActionId,
        case_id: Option<ModerationCaseId>,
        actor_principal_id: PrincipalId,
        room_catalog_id: RoomCatalogId,
        kind: ModerationActionKind,
        target: ModerationTarget,
        reason: ModerationReason,
        starts_at: UtcMillis,
        expires_at: Option<UtcMillis>,
        status: ModerationActionStatus,
        failure_code: Option<String>,
        reversed_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        let mut action = Self::reserve(
            id,
            case_id,
            actor_principal_id,
            room_catalog_id,
            kind,
            target,
            reason,
            starts_at,
            expires_at,
        )?;
        let consistent = match status {
            ModerationActionStatus::Pending | ModerationActionStatus::Applied => {
                failure_code.is_none() && reversed_at.is_none()
            }
            ModerationActionStatus::Failed => failure_code.is_some() && reversed_at.is_none(),
            ModerationActionStatus::Reversed => failure_code.is_none() && reversed_at.is_some(),
        };
        if !consistent || reversed_at.is_some_and(|reversed_at| reversed_at < starts_at) {
            return Err(invariant(
                "moderation_action",
                "动作状态必须与失败码和撤销时间一致",
            ));
        }
        action.status = status;
        action.failure_code = failure_code;
        action.reversed_at = reversed_at;
        Ok(action)
    }

    /// 标记外部治理副作用已经应用。
    ///
    /// # Errors
    ///
    /// 动作已经失败或撤销时返回错误。
    pub fn mark_applied(&mut self) -> DomainResult<bool> {
        match self.status {
            ModerationActionStatus::Pending => {
                self.status = ModerationActionStatus::Applied;
                Ok(true)
            }
            ModerationActionStatus::Applied => Ok(false),
            ModerationActionStatus::Failed | ModerationActionStatus::Reversed => Err(invariant(
                "moderation_action",
                "失败或已撤销的动作不能标记为已应用",
            )),
        }
    }

    /// 标记外部治理副作用失败。
    ///
    /// # Errors
    ///
    /// 失败码无效、动作已经应用或试图改写既有失败原因时返回错误。
    pub fn mark_failed(&mut self, failure_code: impl Into<String>) -> DomainResult<bool> {
        let failure_code = failure_code.into();
        validate_text("moderation_action_failure_code", &failure_code, 128, false)?;
        match self.status {
            ModerationActionStatus::Pending => {
                self.status = ModerationActionStatus::Failed;
                self.failure_code = Some(failure_code);
                Ok(true)
            }
            ModerationActionStatus::Failed
                if self.failure_code.as_deref() == Some(&failure_code) =>
            {
                Ok(false)
            }
            ModerationActionStatus::Applied | ModerationActionStatus::Reversed => Err(invariant(
                "moderation_action",
                "已应用或已撤销的动作不能标记为失败",
            )),
            ModerationActionStatus::Failed => {
                Err(invariant("moderation_action", "失败动作不能改写失败原因"))
            }
        }
    }

    /// 撤销已经应用的治理动作。
    ///
    /// # Errors
    ///
    /// 动作尚未应用、已经失败或撤销时间早于动作开始时间时返回错误。
    pub fn reverse(&mut self, reversed_at: UtcMillis) -> DomainResult<bool> {
        match self.status {
            ModerationActionStatus::Applied if reversed_at >= self.starts_at => {
                self.status = ModerationActionStatus::Reversed;
                self.reversed_at = Some(reversed_at);
                Ok(true)
            }
            ModerationActionStatus::Reversed => Ok(false),
            ModerationActionStatus::Applied => Err(validation(
                "moderation_action_reversed_at",
                "撤销时间不能早于动作开始时间",
            )),
            ModerationActionStatus::Pending | ModerationActionStatus::Failed => {
                Err(invariant("moderation_action", "只有已应用动作可以撤销"))
            }
        }
    }

    pub const fn id(&self) -> ModerationActionId {
        self.id
    }

    pub const fn case_id(&self) -> Option<ModerationCaseId> {
        self.case_id
    }

    pub const fn actor_principal_id(&self) -> PrincipalId {
        self.actor_principal_id
    }

    pub const fn room_catalog_id(&self) -> RoomCatalogId {
        self.room_catalog_id
    }

    pub const fn kind(&self) -> ModerationActionKind {
        self.kind
    }

    pub const fn target(&self) -> &ModerationTarget {
        &self.target
    }

    pub const fn reason(&self) -> ModerationReason {
        self.reason
    }

    pub const fn starts_at(&self) -> UtcMillis {
        self.starts_at
    }

    pub const fn expires_at(&self) -> Option<UtcMillis> {
        self.expires_at
    }

    pub const fn status(&self) -> ModerationActionStatus {
        self.status
    }

    pub fn failure_code(&self) -> Option<&str> {
        self.failure_code.as_deref()
    }

    pub const fn reversed_at(&self) -> Option<UtcMillis> {
        self.reversed_at
    }

    pub fn is_effective_at(&self, now: UtcMillis) -> bool {
        self.status == ModerationActionStatus::Applied
            && self.expires_at.is_none_or(|expires_at| now < expires_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationRole {
    None,
    RoomManager,
    PlatformModerator,
    AuditReader,
}

impl ModerationRole {
    pub const fn allows(self, action: ModerationActionKind) -> bool {
        match self {
            Self::None | Self::AuditReader => false,
            Self::RoomManager | Self::PlatformModerator => matches!(
                action,
                ModerationActionKind::Hide
                    | ModerationActionKind::Mute
                    | ModerationActionKind::Kick
                    | ModerationActionKind::Ban
            ),
        }
    }

    pub const fn can_read_audit(self) -> bool {
        matches!(self, Self::PlatformModerator | Self::AuditReader)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationAuditOutcome {
    Allowed,
    Denied,
    Failed,
}

impl ModerationAuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationAuditEvent {
    pub id: AuditEventId,
    pub occurred_at: UtcMillis,
    pub actor_principal_id: PrincipalId,
    pub action: String,
    pub target: ModerationTarget,
    pub outcome: ModerationAuditOutcome,
    pub reason: Option<ModerationReason>,
    pub correlation_id: AuditEventId,
    pub room_catalog_id: Option<RoomCatalogId>,
}

impl ModerationAuditEvent {
    /// 验证审计动作码，并保持元数据为固定白名单字段。
    ///
    /// # Errors
    ///
    /// 动作码为空、含控制字符或超过上限时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AuditEventId,
        occurred_at: UtcMillis,
        actor_principal_id: PrincipalId,
        action: impl Into<String>,
        target: ModerationTarget,
        outcome: ModerationAuditOutcome,
        reason: Option<ModerationReason>,
        correlation_id: AuditEventId,
        room_catalog_id: Option<RoomCatalogId>,
    ) -> DomainResult<Self> {
        let action = action.into();
        validate_text("moderation_audit_action", &action, 128, false)?;
        Ok(Self {
            id,
            occurred_at,
            actor_principal_id,
            action,
            target,
            outcome,
            reason,
            correlation_id,
            room_catalog_id,
        })
    }
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum_characters: usize,
    allow_empty: bool,
) -> DomainResult<()> {
    let characters = value.chars().count();
    if (!allow_empty && value.trim().is_empty())
        || characters > maximum_characters
        || value.chars().any(char::is_control)
    {
        return Err(validation(field, "文本为空、包含控制字符或超过长度上限"));
    }
    Ok(())
}

const fn validation(field: &'static str, reason: &'static str) -> DomainError {
    DomainError::Validation { field, reason }
}

const fn invariant(entity: &'static str, rule: &'static str) -> DomainError {
    DomainError::InvariantViolation { entity, rule }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationEvidence,
        ModerationReason, ModerationRole, ModerationTarget, ModerationTargetKind,
    };
    use crate::{
        ids::{ModerationActionId, PrincipalId, RoomCatalogId},
        time::UtcMillis,
    };

    #[test]
    fn 私聊证据只有举报者显式提交时才包含正文() {
        let reference_only = ModerationEvidence::new(
            Some(room_id()),
            Some("$event:matrix.test".to_owned()),
            None,
            true,
        )
        .expect("仅引用的证据有效");
        assert!(!reference_only.discloses_plaintext());

        let disclosed = ModerationEvidence::new(
            Some(room_id()),
            Some("$event:matrix.test".to_owned()),
            Some("举报者明确选择的最小摘录".to_owned()),
            true,
        )
        .expect("显式提交证据有效");
        assert!(disclosed.discloses_plaintext());
        assert!(
            ModerationEvidence::new(
                Some(room_id()),
                None,
                Some("不能脱离事件提交正文".to_owned()),
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn 治理动作必须匹配目标且只能从已应用状态撤销() {
        let mut action = ModerationAction::reserve(
            ModerationActionId::from_uuid(Uuid::now_v7()),
            None,
            principal_id(),
            room_id(),
            ModerationActionKind::Mute,
            ModerationTarget::new(ModerationTargetKind::Principal, principal_id().to_string())
                .expect("目标有效"),
            ModerationReason::Spam,
            time(1_000),
            Some(time(10_000)),
        )
        .expect("动作有效");

        assert!(action.reverse(time(2_000)).is_err());
        assert!(action.mark_applied().expect("可应用"));
        assert!(action.is_effective_at(time(2_000)));
        assert!(action.reverse(time(3_000)).expect("可撤销"));
        assert_eq!(action.status(), ModerationActionStatus::Reversed);
        assert!(!action.is_effective_at(time(4_000)));
    }

    #[test]
    fn 房间管理者不能读取受限审计而审计角色不能执行治理() {
        assert!(ModerationRole::RoomManager.allows(ModerationActionKind::Hide));
        assert!(!ModerationRole::RoomManager.can_read_audit());
        assert!(!ModerationRole::AuditReader.allows(ModerationActionKind::Ban));
        assert!(ModerationRole::AuditReader.can_read_audit());
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn room_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
