use std::collections::BTreeSet;

use crate::{
    DomainError, DomainResult,
    ids::{AgentId, AgentInstanceId, AutomationGrantId, PrincipalId, RoomCatalogId},
    time::UtcMillis,
    version::AggregateVersion,
};

pub const AUTOMATION_MAX_MESSAGES_PER_MINUTE: u16 = 60;
pub const AUTOMATION_MAX_TOTAL_MESSAGES: u32 = 10_000;
pub const AUTOMATION_MAX_LIFETIME_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AutomationMessageKind {
    RoomMessage,
    Reply,
}

impl AutomationMessageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RoomMessage => "room_message",
            Self::Reply => "reply",
        }
    }
}

impl TryFrom<&str> for AutomationMessageKind {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "room_message" => Ok(Self::RoomMessage),
            "reply" => Ok(Self::Reply),
            _ => Err(validation(
                "automation_message_kind",
                "不是支持的自动消息类别",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationMessageKinds(BTreeSet<AutomationMessageKind>);

impl AutomationMessageKinds {
    /// 创建非空、去重的自动消息类别集合。
    ///
    /// # Errors
    ///
    /// 集合为空时返回领域校验错误。
    pub fn new(values: impl IntoIterator<Item = AutomationMessageKind>) -> DomainResult<Self> {
        let values = values.into_iter().collect::<BTreeSet<_>>();
        if values.is_empty() {
            return Err(validation(
                "automation_message_kinds",
                "至少允许一种消息类别",
            ));
        }
        Ok(Self(values))
    }

    pub fn contains(&self, value: AutomationMessageKind) -> bool {
        self.0.contains(&value)
    }

    pub fn iter(&self) -> impl Iterator<Item = AutomationMessageKind> + '_ {
        self.0.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationAudience {
    KnownRoomMembers,
    AnyRoomMember,
}

impl AutomationAudience {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KnownRoomMembers => "known_room_members",
            Self::AnyRoomMember => "any_room_member",
        }
    }

    pub const fn allows_unknown_recipients(self) -> bool {
        matches!(self, Self::AnyRoomMember)
    }
}

impl TryFrom<&str> for AutomationAudience {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "known_room_members" => Ok(Self::KnownRoomMembers),
            "any_room_member" => Ok(Self::AnyRoomMember),
            _ => Err(validation(
                "automation_audience",
                "不是支持的自动消息受众范围",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationRiskScanOutcome {
    Passed,
    Rejected,
    Unavailable,
    NotRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationGrantStatus {
    Active,
    Revoked,
    Exhausted,
    Expired,
}

impl AutomationGrantStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Exhausted => "exhausted",
            Self::Expired => "expired",
        }
    }
}

impl TryFrom<&str> for AutomationGrantStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "exhausted" => Ok(Self::Exhausted),
            "expired" => Ok(Self::Expired),
            _ => Err(validation(
                "automation_grant_status",
                "不是支持的自动发言授权状态",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationGrantScope {
    agent_id: AgentId,
    agent_instance_id: Option<AgentInstanceId>,
    room_catalog_id: RoomCatalogId,
    message_kinds: AutomationMessageKinds,
    audience: AutomationAudience,
    requires_risk_scan: bool,
}

impl AutomationGrantScope {
    /// 创建不可扩大的自动发言作用域。
    ///
    /// # Errors
    ///
    /// 允许陌生接收者却关闭风险扫描时返回错误。
    pub fn new(
        agent_id: AgentId,
        agent_instance_id: Option<AgentInstanceId>,
        room_catalog_id: RoomCatalogId,
        message_kinds: AutomationMessageKinds,
        audience: AutomationAudience,
        requires_risk_scan: bool,
    ) -> DomainResult<Self> {
        if audience.allows_unknown_recipients() && !requires_risk_scan {
            return Err(validation(
                "automation_risk_scan",
                "允许陌生接收者时必须执行内容风险扫描",
            ));
        }
        Ok(Self {
            agent_id,
            agent_instance_id,
            room_catalog_id,
            message_kinds,
            audience,
            requires_risk_scan,
        })
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn agent_instance_id(&self) -> Option<AgentInstanceId> {
        self.agent_instance_id
    }

    pub const fn room_catalog_id(&self) -> RoomCatalogId {
        self.room_catalog_id
    }

    pub const fn message_kinds(&self) -> &AutomationMessageKinds {
        &self.message_kinds
    }

    pub const fn audience(&self) -> AutomationAudience {
        self.audience
    }

    pub const fn requires_risk_scan(&self) -> bool {
        self.requires_risk_scan
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationGrantLimits {
    max_messages_per_minute: u16,
    max_total_messages: Option<u32>,
    starts_at: UtcMillis,
    expires_at: UtcMillis,
}

impl AutomationGrantLimits {
    /// 创建系统硬上限内的授权限额。
    ///
    /// # Errors
    ///
    /// 频率、总量或时间窗口为空、倒置或超过系统上限时返回错误。
    pub fn new(
        max_messages_per_minute: u16,
        max_total_messages: Option<u32>,
        starts_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> DomainResult<Self> {
        if !(1..=AUTOMATION_MAX_MESSAGES_PER_MINUTE).contains(&max_messages_per_minute) {
            return Err(validation(
                "automation_max_messages_per_minute",
                "必须在 1 到系统频率上限之间",
            ));
        }
        if max_total_messages
            .is_some_and(|maximum| !(1..=AUTOMATION_MAX_TOTAL_MESSAGES).contains(&maximum))
        {
            return Err(validation(
                "automation_max_total_messages",
                "必须为空或在 1 到系统总量上限之间",
            ));
        }
        let lifetime = expires_at.value().saturating_sub(starts_at.value());
        if !(1..=AUTOMATION_MAX_LIFETIME_MILLIS).contains(&lifetime) {
            return Err(validation(
                "automation_grant_lifetime",
                "生效窗口必须非空且不超过系统期限上限",
            ));
        }
        Ok(Self {
            max_messages_per_minute,
            max_total_messages,
            starts_at,
            expires_at,
        })
    }

    pub const fn max_messages_per_minute(self) -> u16 {
        self.max_messages_per_minute
    }

    pub const fn max_total_messages(self) -> Option<u32> {
        self.max_total_messages
    }

    pub const fn starts_at(self) -> UtcMillis {
        self.starts_at
    }

    pub const fn expires_at(self) -> UtcMillis {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationGrantFields {
    pub id: AutomationGrantId,
    pub grantor_id: PrincipalId,
    pub scope: AutomationGrantScope,
    pub limits: AutomationGrantLimits,
    pub created_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationGrant {
    id: AutomationGrantId,
    grantor_id: PrincipalId,
    scope: AutomationGrantScope,
    limits: AutomationGrantLimits,
    state: AutomationGrantStatus,
    created_at: UtcMillis,
    revoked_at: Option<UtcMillis>,
    version: AggregateVersion,
}

impl AutomationGrant {
    /// 签发一份默认仅对显式作用域有效的自动发言授权。
    ///
    /// # Errors
    ///
    /// 创建时间晚于生效时间时返回错误。
    pub fn issue(fields: AutomationGrantFields) -> DomainResult<Self> {
        if fields.created_at > fields.limits.starts_at() {
            return Err(validation(
                "automation_grant_created_at",
                "创建时间不能晚于生效时间",
            ));
        }
        Ok(Self {
            id: fields.id,
            grantor_id: fields.grantor_id,
            scope: fields.scope,
            limits: fields.limits,
            state: AutomationGrantStatus::Active,
            created_at: fields.created_at,
            revoked_at: None,
            version: AggregateVersion::INITIAL,
        })
    }

    /// 从权威存储恢复授权聚合并验证状态时间线。
    ///
    /// # Errors
    ///
    /// 撤销时间、状态或版本互相矛盾时返回错误。
    pub fn restore(
        fields: AutomationGrantFields,
        state: AutomationGrantStatus,
        revoked_at: Option<UtcMillis>,
        version: AggregateVersion,
    ) -> DomainResult<Self> {
        let grant = Self::issue(fields)?;
        let revocation_is_valid = match state {
            AutomationGrantStatus::Revoked => revoked_at.is_some_and(|revoked_at| {
                revoked_at >= grant.created_at && revoked_at < grant.limits.expires_at()
            }),
            AutomationGrantStatus::Active
            | AutomationGrantStatus::Exhausted
            | AutomationGrantStatus::Expired => revoked_at.is_none(),
        };
        if !revocation_is_valid {
            return Err(invariant(
                "automation_grant",
                "撤销时间必须且只能出现在已撤销状态",
            ));
        }
        Ok(Self {
            state,
            revoked_at,
            version,
            ..grant
        })
    }

    pub const fn id(&self) -> AutomationGrantId {
        self.id
    }

    pub const fn grantor_id(&self) -> PrincipalId {
        self.grantor_id
    }

    pub const fn scope(&self) -> &AutomationGrantScope {
        &self.scope
    }

    pub const fn limits(&self) -> AutomationGrantLimits {
        self.limits
    }

    pub const fn status(&self) -> AutomationGrantStatus {
        self.state
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.created_at
    }

    pub const fn revoked_at(&self) -> Option<UtcMillis> {
        self.revoked_at
    }

    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    /// 立即撤销活动授权；重复撤销保持幂等。
    ///
    /// # Errors
    ///
    /// 撤销时间早于创建时间、晚于到期时间或版本溢出时返回错误。
    pub fn revoke(&mut self, revoked_at: UtcMillis) -> DomainResult<bool> {
        if self.state == AutomationGrantStatus::Revoked {
            return Ok(false);
        }
        if revoked_at < self.created_at || revoked_at >= self.limits.expires_at() {
            return Err(validation(
                "automation_grant_revoked_at",
                "撤销时间必须位于授权生命周期内",
            ));
        }
        self.state = AutomationGrantStatus::Revoked;
        self.revoked_at = Some(revoked_at);
        self.version = self.version.next()?;
        Ok(true)
    }

    pub fn evaluate(
        &self,
        attempt: &AutomationGrantAttempt,
        usage: AutomationUsageSnapshot,
    ) -> AutomationGrantDecision {
        use AutomationGrantDenial as Denial;

        let denial = match self.state {
            AutomationGrantStatus::Revoked => Some(Denial::Revoked),
            AutomationGrantStatus::Exhausted => Some(Denial::TotalLimitExceeded),
            AutomationGrantStatus::Expired => Some(Denial::Expired),
            AutomationGrantStatus::Active => None,
        }
        .or_else(|| (attempt.now < self.limits.starts_at()).then_some(Denial::NotStarted))
        .or_else(|| (attempt.now >= self.limits.expires_at()).then_some(Denial::Expired))
        .or_else(|| (attempt.agent_id != self.scope.agent_id()).then_some(Denial::AgentMismatch))
        .or_else(|| {
            self.scope
                .agent_instance_id()
                .is_some_and(|expected| Some(expected) != attempt.agent_instance_id)
                .then_some(Denial::InstanceMismatch)
        })
        .or_else(|| {
            (attempt.room_catalog_id != self.scope.room_catalog_id())
                .then_some(Denial::RoomMismatch)
        })
        .or_else(|| {
            (!self.scope.message_kinds().contains(attempt.message_kind))
                .then_some(Denial::MessageKindNotAllowed)
        })
        .or_else(|| {
            (attempt.contains_unknown_recipients
                && !self.scope.audience().allows_unknown_recipients())
            .then_some(Denial::UnknownRecipientNotAllowed)
        })
        .or_else(|| {
            self.limits
                .max_total_messages()
                .is_some_and(|maximum| usage.total_messages >= maximum)
                .then_some(Denial::TotalLimitExceeded)
        })
        .or_else(|| {
            (usage.messages_in_current_minute >= u32::from(self.limits.max_messages_per_minute()))
                .then_some(Denial::RateLimitExceeded)
        })
        .or_else(|| match attempt.risk_scan {
            AutomationRiskScanOutcome::Rejected => Some(Denial::RiskScanRejected),
            AutomationRiskScanOutcome::Unavailable | AutomationRiskScanOutcome::NotRequested
                if self.scope.requires_risk_scan() =>
            {
                Some(Denial::RiskScanRequired)
            }
            AutomationRiskScanOutcome::Passed
            | AutomationRiskScanOutcome::Unavailable
            | AutomationRiskScanOutcome::NotRequested => None,
        });

        denial.map_or(
            AutomationGrantDecision::Allowed,
            AutomationGrantDecision::Denied,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationGrantAttempt {
    pub agent_id: AgentId,
    pub agent_instance_id: Option<AgentInstanceId>,
    pub room_catalog_id: RoomCatalogId,
    pub message_kind: AutomationMessageKind,
    pub contains_unknown_recipients: bool,
    pub risk_scan: AutomationRiskScanOutcome,
    pub now: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutomationUsageSnapshot {
    pub total_messages: u32,
    pub messages_in_current_minute: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationGrantDecision {
    Allowed,
    Denied(AutomationGrantDenial),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationGrantDenial {
    NotStarted,
    Revoked,
    Expired,
    AgentMismatch,
    InstanceMismatch,
    RoomMismatch,
    MessageKindNotAllowed,
    UnknownRecipientNotAllowed,
    RateLimitExceeded,
    TotalLimitExceeded,
    RiskScanRequired,
    RiskScanRejected,
}

impl AutomationGrantDenial {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "automation.grant_not_started",
            Self::Revoked => "automation.grant_revoked",
            Self::Expired => "automation.grant_expired",
            Self::AgentMismatch => "automation.agent_mismatch",
            Self::InstanceMismatch => "automation.instance_mismatch",
            Self::RoomMismatch => "automation.room_mismatch",
            Self::MessageKindNotAllowed => "automation.message_kind_not_allowed",
            Self::UnknownRecipientNotAllowed => "automation.unknown_recipient_not_allowed",
            Self::RateLimitExceeded => "automation.rate_limit_exceeded",
            Self::TotalLimitExceeded => "automation.total_limit_exceeded",
            Self::RiskScanRequired => "automation.risk_scan_required",
            Self::RiskScanRejected => "automation.risk_scan_rejected",
        }
    }
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
        AUTOMATION_MAX_LIFETIME_MILLIS, AutomationAudience, AutomationGrant,
        AutomationGrantAttempt, AutomationGrantDecision, AutomationGrantDenial,
        AutomationGrantFields, AutomationGrantLimits, AutomationGrantScope, AutomationGrantStatus,
        AutomationMessageKind, AutomationMessageKinds, AutomationRiskScanOutcome,
        AutomationUsageSnapshot,
    };
    use crate::{
        ids::{AgentId, AgentInstanceId, AutomationGrantId, PrincipalId, RoomCatalogId},
        time::UtcMillis,
        version::AggregateVersion,
    };

    #[test]
    fn 默认精确作用域允许符合限额的消息() {
        let grant = grant(AutomationAudience::KnownRoomMembers, false);

        assert_eq!(
            grant.evaluate(&attempt(), AutomationUsageSnapshot::default()),
            AutomationGrantDecision::Allowed
        );
    }

    #[test]
    fn 逐项拒绝作用域越界和限额耗尽() {
        let grant = grant(AutomationAudience::KnownRoomMembers, false);
        let cases = [
            (
                AutomationGrantAttempt {
                    agent_id: AgentId::from_uuid(Uuid::from_u128(99)),
                    ..attempt()
                },
                AutomationUsageSnapshot::default(),
                AutomationGrantDenial::AgentMismatch,
            ),
            (
                AutomationGrantAttempt {
                    agent_instance_id: None,
                    ..attempt()
                },
                AutomationUsageSnapshot::default(),
                AutomationGrantDenial::InstanceMismatch,
            ),
            (
                AutomationGrantAttempt {
                    room_catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(99)),
                    ..attempt()
                },
                AutomationUsageSnapshot::default(),
                AutomationGrantDenial::RoomMismatch,
            ),
            (
                AutomationGrantAttempt {
                    message_kind: AutomationMessageKind::RoomMessage,
                    ..attempt()
                },
                AutomationUsageSnapshot::default(),
                AutomationGrantDenial::MessageKindNotAllowed,
            ),
            (
                AutomationGrantAttempt {
                    contains_unknown_recipients: true,
                    ..attempt()
                },
                AutomationUsageSnapshot::default(),
                AutomationGrantDenial::UnknownRecipientNotAllowed,
            ),
            (
                attempt(),
                AutomationUsageSnapshot {
                    total_messages: 10,
                    messages_in_current_minute: 0,
                },
                AutomationGrantDenial::TotalLimitExceeded,
            ),
            (
                attempt(),
                AutomationUsageSnapshot {
                    total_messages: 1,
                    messages_in_current_minute: 3,
                },
                AutomationGrantDenial::RateLimitExceeded,
            ),
        ];

        for (attempt, usage, expected) in cases {
            assert_eq!(
                grant.evaluate(&attempt, usage),
                AutomationGrantDecision::Denied(expected)
            );
        }
    }

    #[test]
    fn 生效时间到期撤销和风险扫描均失败关闭() {
        let mut grant = grant(AutomationAudience::AnyRoomMember, true);
        let not_started = AutomationGrantAttempt {
            now: time(999),
            risk_scan: AutomationRiskScanOutcome::Passed,
            ..attempt()
        };
        let expired = AutomationGrantAttempt {
            now: time(61_000),
            risk_scan: AutomationRiskScanOutcome::Passed,
            ..attempt()
        };
        let unscanned = AutomationGrantAttempt {
            risk_scan: AutomationRiskScanOutcome::Unavailable,
            ..attempt()
        };
        assert_eq!(
            grant.evaluate(&not_started, AutomationUsageSnapshot::default()),
            AutomationGrantDecision::Denied(AutomationGrantDenial::NotStarted)
        );
        assert_eq!(
            grant.evaluate(&expired, AutomationUsageSnapshot::default()),
            AutomationGrantDecision::Denied(AutomationGrantDenial::Expired)
        );
        assert_eq!(
            grant.evaluate(&unscanned, AutomationUsageSnapshot::default()),
            AutomationGrantDecision::Denied(AutomationGrantDenial::RiskScanRequired)
        );

        assert!(grant.revoke(time(5_000)).expect("生命周期内可以撤销"));
        assert!(!grant.revoke(time(6_000)).expect("重复撤销保持幂等"));
        assert_eq!(grant.status(), AutomationGrantStatus::Revoked);
        assert_eq!(grant.version(), AggregateVersion::new(1).expect("版本有效"));
        assert_eq!(
            grant.evaluate(&attempt(), AutomationUsageSnapshot::default()),
            AutomationGrantDecision::Denied(AutomationGrantDenial::Revoked)
        );
    }

    #[test]
    fn 无风险扫描不得扩大到陌生接收者() {
        assert!(
            AutomationGrantScope::new(
                agent_id(),
                Some(instance_id()),
                room_id(),
                AutomationMessageKinds::new([AutomationMessageKind::Reply]).expect("类别有效"),
                AutomationAudience::AnyRoomMember,
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn 限额和恢复事实拒绝越界状态() {
        assert!(AutomationGrantLimits::new(0, None, time(1), time(2)).is_err());
        assert!(AutomationGrantLimits::new(1, Some(0), time(1), time(2)).is_err());
        assert!(
            AutomationGrantLimits::new(
                1,
                None,
                time(1),
                time(1 + AUTOMATION_MAX_LIFETIME_MILLIS + 1),
            )
            .is_err()
        );

        let base = fields(AutomationAudience::KnownRoomMembers, false);
        assert!(
            AutomationGrant::restore(
                base.clone(),
                AutomationGrantStatus::Revoked,
                None,
                AggregateVersion::INITIAL,
            )
            .is_err()
        );
        assert!(
            AutomationGrant::restore(
                base,
                AutomationGrantStatus::Active,
                Some(time(5_000)),
                AggregateVersion::INITIAL,
            )
            .is_err()
        );
    }

    #[test]
    fn 持久化枚举使用稳定协议值() {
        assert_eq!(
            AutomationMessageKind::try_from("reply"),
            Ok(AutomationMessageKind::Reply)
        );
        assert_eq!(
            AutomationAudience::try_from("any_room_member"),
            Ok(AutomationAudience::AnyRoomMember)
        );
        assert_eq!(
            AutomationGrantStatus::try_from("exhausted"),
            Ok(AutomationGrantStatus::Exhausted)
        );
        assert!(AutomationMessageKind::try_from("unknown").is_err());
        assert_eq!(
            AutomationGrantDenial::RateLimitExceeded.as_str(),
            "automation.rate_limit_exceeded"
        );
    }

    fn grant(audience: AutomationAudience, requires_risk_scan: bool) -> AutomationGrant {
        AutomationGrant::issue(fields(audience, requires_risk_scan)).expect("测试授权有效")
    }

    fn fields(audience: AutomationAudience, requires_risk_scan: bool) -> AutomationGrantFields {
        AutomationGrantFields {
            id: AutomationGrantId::from_uuid(Uuid::from_u128(1)),
            grantor_id: PrincipalId::from_uuid(Uuid::from_u128(2)),
            scope: AutomationGrantScope::new(
                agent_id(),
                Some(instance_id()),
                room_id(),
                AutomationMessageKinds::new([AutomationMessageKind::Reply]).expect("类别有效"),
                audience,
                requires_risk_scan,
            )
            .expect("作用域有效"),
            limits: AutomationGrantLimits::new(3, Some(10), time(1_000), time(61_000))
                .expect("限额有效"),
            created_at: time(500),
        }
    }

    fn attempt() -> AutomationGrantAttempt {
        AutomationGrantAttempt {
            agent_id: agent_id(),
            agent_instance_id: Some(instance_id()),
            room_catalog_id: room_id(),
            message_kind: AutomationMessageKind::Reply,
            contains_unknown_recipients: false,
            risk_scan: AutomationRiskScanOutcome::Passed,
            now: time(2_000),
        }
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(Uuid::from_u128(3))
    }

    fn instance_id() -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::from_u128(4))
    }

    fn room_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(5))
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
