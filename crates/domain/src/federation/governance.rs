use crate::{
    DomainError, DomainResult,
    ids::{FederationRuleId, PrincipalId},
    time::UtcMillis,
};

const MAX_SERVER_NAME_LENGTH: usize = 255;
const MAX_REFERENCE_LENGTH: usize = 1_024;
const MAX_REASON_LENGTH: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FederationServerName(String);

impl FederationServerName {
    /// 创建规范化的 Matrix 服务名。
    ///
    /// # Errors
    ///
    /// 服务名为空、包含空白或控制字符、超长或看起来像 Matrix 用户/房间标识时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into().to_ascii_lowercase();
        validate_reference("federation_server_name", &value, MAX_SERVER_NAME_LENGTH)?;
        if value.starts_with(['@', '!', '#'])
            || value.contains('/')
            || value.chars().any(char::is_whitespace)
        {
            return Err(validation(
                "federation_server_name",
                "不是可接受的 Matrix 服务名",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederationScope {
    Server {
        peer: FederationServerName,
    },
    Room {
        peer: FederationServerName,
        room_id: String,
    },
    User {
        peer: FederationServerName,
        user_id: String,
    },
}

impl FederationScope {
    /// 建立整个联邦对端作用域。
    ///
    /// # Errors
    ///
    /// 服务名无效时返回错误。
    pub fn server(peer: impl Into<String>) -> DomainResult<Self> {
        Ok(Self::Server {
            peer: FederationServerName::new(peer)?,
        })
    }

    /// 建立单房间作用域。
    ///
    /// # Errors
    ///
    /// 服务名或 Matrix 房间标识无效时返回错误。
    pub fn room(peer: impl Into<String>, room_id: impl Into<String>) -> DomainResult<Self> {
        let room_id = room_id.into();
        validate_matrix_reference("federation_room_id", &room_id, &['!', '#'])?;
        Ok(Self::Room {
            peer: FederationServerName::new(peer)?,
            room_id,
        })
    }

    /// 建立单用户作用域。
    ///
    /// # Errors
    ///
    /// 服务名或 Matrix 用户标识无效时返回错误。
    pub fn user(peer: impl Into<String>, user_id: impl Into<String>) -> DomainResult<Self> {
        let user_id = user_id.into();
        validate_matrix_reference("federation_user_id", &user_id, &['@'])?;
        Ok(Self::User {
            peer: FederationServerName::new(peer)?,
            user_id,
        })
    }

    pub const fn peer(&self) -> &FederationServerName {
        match self {
            Self::Server { peer } | Self::Room { peer, .. } | Self::User { peer, .. } => peer,
        }
    }

    pub(crate) fn matches(
        &self,
        peer: &FederationServerName,
        room_id: &str,
        user_id: &str,
    ) -> bool {
        match self {
            Self::Server { peer: expected } => expected == peer,
            Self::Room {
                peer: expected,
                room_id: expected_room,
            } => expected == peer && expected_room == room_id,
            Self::User {
                peer: expected,
                user_id: expected_user,
            } => expected == peer && expected_user == user_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FederationDisposition {
    Allow,
    Throttle,
    Quarantine,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationRuleAuditAction {
    Created,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationRuleAudit {
    pub rule_id: FederationRuleId,
    pub action: FederationRuleAuditAction,
    pub actor: PrincipalId,
    pub occurred_at: UtcMillis,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationRule {
    id: FederationRuleId,
    scope: FederationScope,
    disposition: FederationDisposition,
    created_by: PrincipalId,
    created_at: UtcMillis,
    reason: String,
    expires_at: Option<UtcMillis>,
    revocation: Option<FederationRuleAudit>,
}

impl FederationRule {
    /// 创建可到期、可撤销且具备操作者审计的联邦规则。
    ///
    /// # Errors
    ///
    /// 原因为空或超限，以及到期时间不晚于创建时间时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: FederationRuleId,
        scope: FederationScope,
        disposition: FederationDisposition,
        created_by: PrincipalId,
        created_at: UtcMillis,
        reason: impl Into<String>,
        expires_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        let reason = reason.into();
        validate_reason(&reason)?;
        if expires_at.is_some_and(|expires_at| expires_at <= created_at) {
            return Err(validation("federation_rule_expires_at", "必须晚于创建时间"));
        }
        Ok(Self {
            id,
            scope,
            disposition,
            created_by,
            created_at,
            reason,
            expires_at,
            revocation: None,
        })
    }

    pub const fn id(&self) -> FederationRuleId {
        self.id
    }

    pub const fn scope(&self) -> &FederationScope {
        &self.scope
    }

    pub const fn disposition(&self) -> FederationDisposition {
        self.disposition
    }

    pub fn is_active_at(&self, now: UtcMillis) -> bool {
        self.revocation.is_none() && self.expires_at.is_none_or(|expires_at| now < expires_at)
    }

    pub fn creation_audit(&self) -> FederationRuleAudit {
        FederationRuleAudit {
            rule_id: self.id,
            action: FederationRuleAuditAction::Created,
            actor: self.created_by,
            occurred_at: self.created_at,
            reason: self.reason.clone(),
        }
    }

    pub const fn revocation_audit(&self) -> Option<&FederationRuleAudit> {
        self.revocation.as_ref()
    }

    /// 撤销规则并保留独立审计事实。
    ///
    /// # Errors
    ///
    /// 撤销时间早于创建时间，或撤销原因为空/超限时返回错误。
    pub fn revoke(
        &mut self,
        actor: PrincipalId,
        occurred_at: UtcMillis,
        reason: impl Into<String>,
    ) -> DomainResult<bool> {
        if self.revocation.is_some() {
            return Ok(false);
        }
        if occurred_at < self.created_at {
            return Err(validation("federation_rule_revoked_at", "不能早于创建时间"));
        }
        let reason = reason.into();
        validate_reason(&reason)?;
        self.revocation = Some(FederationRuleAudit {
            rule_id: self.id,
            action: FederationRuleAuditAction::Revoked,
            actor,
            occurred_at,
            reason,
        });
        Ok(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationGovernanceDecision {
    pub disposition: FederationDisposition,
    pub matching_rule_ids: Vec<FederationRuleId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FederationPolicySet {
    rules: Vec<FederationRule>,
}

impl FederationPolicySet {
    /// 注册规则，拒绝相同规则标识对应多个事实。
    ///
    /// # Errors
    ///
    /// 规则标识重复时返回不变式错误。
    pub fn register(&mut self, rule: FederationRule) -> DomainResult<()> {
        if self.rules.iter().any(|existing| existing.id == rule.id) {
            return Err(DomainError::InvariantViolation {
                entity: "federation_policy_set",
                rule: "规则标识必须唯一",
            });
        }
        self.rules.push(rule);
        Ok(())
    }

    pub fn rule_mut(&mut self, id: FederationRuleId) -> Option<&mut FederationRule> {
        self.rules.iter_mut().find(|rule| rule.id == id)
    }

    pub fn evaluate(
        &self,
        peer: &FederationServerName,
        room_id: &str,
        user_id: &str,
        now: UtcMillis,
    ) -> FederationGovernanceDecision {
        let mut matching = self
            .rules
            .iter()
            .filter(|rule| rule.is_active_at(now) && rule.scope.matches(peer, room_id, user_id))
            .collect::<Vec<_>>();
        matching.sort_by_key(|rule| rule.id.as_uuid());
        let disposition = matching
            .iter()
            .map(|rule| rule.disposition)
            .max()
            .unwrap_or(FederationDisposition::Allow);
        FederationGovernanceDecision {
            disposition,
            matching_rule_ids: matching.iter().map(|rule| rule.id).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationSignal {
    SuccessfulDelivery,
    VerifiedOperator,
    RateLimitViolation,
    ReplayAttempt,
    InvalidSignature,
}

impl ReputationSignal {
    const fn delta(self) -> i16 {
        match self {
            Self::SuccessfulDelivery => 1,
            Self::VerifiedOperator => 15,
            Self::RateLimitViolation => -10,
            Self::ReplayAttempt => -20,
            Self::InvalidSignature => -35,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationTier {
    Trusted,
    Neutral,
    Degraded,
    Hostile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FederationReputation {
    score: i16,
}

impl FederationReputation {
    pub const fn score(self) -> i16 {
        self.score
    }

    pub const fn tier(self) -> ReputationTier {
        match self.score {
            40..=100 => ReputationTier::Trusted,
            -19..=39 => ReputationTier::Neutral,
            -59..=-20 => ReputationTier::Degraded,
            _ => ReputationTier::Hostile,
        }
    }

    pub fn observe(&mut self, signal: ReputationSignal) {
        self.score = self.score.saturating_add(signal.delta()).clamp(-100, 100);
    }
}

fn validate_matrix_reference(
    field: &'static str,
    value: &str,
    allowed_prefixes: &[char],
) -> DomainResult<()> {
    validate_reference(field, value, MAX_REFERENCE_LENGTH)?;
    if !value.starts_with(allowed_prefixes) || !value.contains(':') {
        return Err(validation(field, "不是完整 Matrix 标识"));
    }
    Ok(())
}

fn validate_reference(field: &'static str, value: &str, maximum: usize) -> DomainResult<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(validation(field, "为空、包含控制字符或超过长度上限"));
    }
    Ok(())
}

fn validate_reason(reason: &str) -> DomainResult<()> {
    validate_reference("federation_rule_reason", reason, MAX_REASON_LENGTH)
}

const fn validation(field: &'static str, reason: &'static str) -> DomainError {
    DomainError::Validation { field, reason }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        federation::{
            FederationDisposition, FederationPolicySet, FederationReputation, FederationRule,
            FederationRuleAuditAction, FederationScope, FederationServerName, ReputationSignal,
            ReputationTier,
        },
        ids::{FederationRuleId, PrincipalId},
        time::UtcMillis,
    };

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("时间有效")
    }

    fn principal(value: u128) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(value))
    }

    fn rule_id(value: u128) -> FederationRuleId {
        FederationRuleId::from_uuid(Uuid::from_u128(value))
    }

    fn rule(
        id: u128,
        scope: FederationScope,
        disposition: FederationDisposition,
    ) -> FederationRule {
        FederationRule::new(
            rule_id(id),
            scope,
            disposition,
            principal(1),
            time(100),
            "自动验收规则",
            None,
        )
        .expect("规则有效")
    }

    #[test]
    fn 用户房间和对端规则取最严格交集而不是互相覆盖() {
        let mut policies = FederationPolicySet::default();
        policies
            .register(rule(
                1,
                FederationScope::server("peer.example").expect("作用域有效"),
                FederationDisposition::Throttle,
            ))
            .expect("规则可注册");
        policies
            .register(rule(
                2,
                FederationScope::room("peer.example", "!room:local.example").expect("作用域有效"),
                FederationDisposition::Quarantine,
            ))
            .expect("规则可注册");
        policies
            .register(rule(
                3,
                FederationScope::user("peer.example", "@bad:peer.example").expect("作用域有效"),
                FederationDisposition::Block,
            ))
            .expect("规则可注册");

        let decision = policies.evaluate(
            &FederationServerName::new("peer.example").expect("服务名有效"),
            "!room:local.example",
            "@bad:peer.example",
            time(200),
        );
        assert_eq!(decision.disposition, FederationDisposition::Block);
        assert_eq!(decision.matching_rule_ids.len(), 3);
    }

    #[test]
    fn 联邦规则可逆且保留创建与撤销审计() {
        let mut rule = rule(
            1,
            FederationScope::server("peer.example").expect("作用域有效"),
            FederationDisposition::Block,
        );
        assert_eq!(
            rule.creation_audit().action,
            FederationRuleAuditAction::Created
        );
        assert!(
            rule.revoke(principal(2), time(200), "人工复核后解除")
                .expect("撤销有效")
        );
        assert!(
            !rule
                .revoke(principal(2), time(300), "重复撤销")
                .expect("重复撤销幂等")
        );
        assert_eq!(
            rule.revocation_audit().expect("有撤销审计").action,
            FederationRuleAuditAction::Revoked
        );
        assert!(!rule.is_active_at(time(300)));
    }

    #[test]
    fn 信誉信号有界并映射为离散等级() {
        let mut reputation = FederationReputation::default();
        for _ in 0..10 {
            reputation.observe(ReputationSignal::InvalidSignature);
        }
        assert_eq!(reputation.score(), -100);
        assert_eq!(reputation.tier(), ReputationTier::Hostile);
        for _ in 0..20 {
            reputation.observe(ReputationSignal::VerifiedOperator);
        }
        assert_eq!(reputation.score(), 100);
        assert_eq!(reputation.tier(), ReputationTier::Trusted);
    }
}
