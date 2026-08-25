use crate::{
    DomainError, DomainResult,
    ids::{AgentId, PrincipalId, RoomCatalogId},
    version::AggregateVersion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSessionLifecycle {
    Provisioning,
    Active,
    Failed,
}

impl DirectSessionLifecycle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Failed => "failed",
        }
    }
}

impl TryFrom<&str> for DirectSessionLifecycle {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "provisioning" => Ok(Self::Provisioning),
            "active" => Ok(Self::Active),
            "failed" => Ok(Self::Failed),
            _ => Err(validation(
                "direct_session_lifecycle",
                "不是支持的直接会话生命周期",
            )),
        }
    }
}

/// 当前产品表面中的稳定直接会话。
///
/// 消息与已读状态不属于该聚合；它们始终由对应 Matrix Room 负责。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSession {
    catalog_id: RoomCatalogId,
    principal_id: PrincipalId,
    target_agent_id: AgentId,
    lifecycle: DirectSessionLifecycle,
    version: AggregateVersion,
}

impl DirectSession {
    pub const fn reserve(
        catalog_id: RoomCatalogId,
        principal_id: PrincipalId,
        target_agent_id: AgentId,
    ) -> Self {
        Self {
            catalog_id,
            principal_id,
            target_agent_id,
            lifecycle: DirectSessionLifecycle::Provisioning,
            version: AggregateVersion::INITIAL,
        }
    }

    /// 从权威存储恢复直接会话。
    ///
    /// # Errors
    ///
    /// 失败状态使用初始版本，或活动状态仍是初始版本时返回错误。
    pub fn restore(
        catalog_id: RoomCatalogId,
        principal_id: PrincipalId,
        target_agent_id: AgentId,
        lifecycle: DirectSessionLifecycle,
        version: AggregateVersion,
    ) -> DomainResult<Self> {
        let version_is_initial = version == AggregateVersion::INITIAL;
        let lifecycle_matches_version = match lifecycle {
            DirectSessionLifecycle::Provisioning => version_is_initial,
            DirectSessionLifecycle::Active | DirectSessionLifecycle::Failed => !version_is_initial,
        };
        if !lifecycle_matches_version {
            return Err(invariant("direct_session", "生命周期必须与乐观锁版本一致"));
        }
        Ok(Self {
            catalog_id,
            principal_id,
            target_agent_id,
            lifecycle,
            version,
        })
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    pub const fn target_agent_id(&self) -> AgentId {
        self.target_agent_id
    }

    pub const fn lifecycle(&self) -> DirectSessionLifecycle {
        self.lifecycle
    }

    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    pub const fn is_active(&self) -> bool {
        matches!(self.lifecycle, DirectSessionLifecycle::Active)
    }

    /// 把预留会话推进为活动状态。
    ///
    /// # Errors
    ///
    /// 会话已失败或版本无法递增时返回错误。
    pub fn activate(&mut self) -> DomainResult<bool> {
        match self.lifecycle {
            DirectSessionLifecycle::Active => Ok(false),
            DirectSessionLifecycle::Failed => Err(invariant(
                "direct_session",
                "失败会话不能直接恢复为活动状态",
            )),
            DirectSessionLifecycle::Provisioning => {
                self.lifecycle = DirectSessionLifecycle::Active;
                self.version = self.version.next()?;
                Ok(true)
            }
        }
    }

    /// 标记无法对账的建房失败。
    ///
    /// # Errors
    ///
    /// 活动会话不能回退为失败状态，版本无法递增时返回错误。
    pub fn fail(&mut self) -> DomainResult<bool> {
        match self.lifecycle {
            DirectSessionLifecycle::Failed => Ok(false),
            DirectSessionLifecycle::Active => {
                Err(invariant("direct_session", "活动会话不能回退为失败状态"))
            }
            DirectSessionLifecycle::Provisioning => {
                self.lifecycle = DirectSessionLifecycle::Failed;
                self.version = self.version.next()?;
                Ok(true)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectPresenceDisclosure {
    Coarse,
    Hidden,
}

/// 主体与 Agent 之间的双向联系策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectContactPolicy {
    principal_id: PrincipalId,
    agent_id: AgentId,
    principal_blocks_agent: bool,
    agent_blocks_principal: bool,
}

impl DirectContactPolicy {
    pub const fn new(principal_id: PrincipalId, agent_id: AgentId) -> Self {
        Self {
            principal_id,
            agent_id,
            principal_blocks_agent: false,
            agent_blocks_principal: false,
        }
    }

    pub const fn restore(
        principal_id: PrincipalId,
        agent_id: AgentId,
        principal_blocks_agent: bool,
        agent_blocks_principal: bool,
    ) -> Self {
        Self {
            principal_id,
            agent_id,
            principal_blocks_agent,
            agent_blocks_principal,
        }
    }

    pub const fn principal_id(self) -> PrincipalId {
        self.principal_id
    }

    pub const fn agent_id(self) -> AgentId {
        self.agent_id
    }

    pub const fn principal_blocks_agent(self) -> bool {
        self.principal_blocks_agent
    }

    pub const fn agent_blocks_principal(self) -> bool {
        self.agent_blocks_principal
    }

    pub const fn delivery_allowed(self) -> bool {
        !self.principal_blocks_agent && !self.agent_blocks_principal
    }

    pub const fn exact_presence_disclosure(self) -> DirectPresenceDisclosure {
        if self.delivery_allowed() {
            DirectPresenceDisclosure::Coarse
        } else {
            DirectPresenceDisclosure::Hidden
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
        DirectContactPolicy, DirectPresenceDisclosure, DirectSession, DirectSessionLifecycle,
    };
    use crate::{
        ids::{AgentId, PrincipalId, RoomCatalogId},
        version::AggregateVersion,
    };

    #[test]
    fn 预留会话只能单向推进为活动状态() {
        let mut session = DirectSession::reserve(catalog_id(), principal_id(), agent_id());

        assert!(session.activate().expect("预留会话可激活"));
        assert!(!session.activate().expect("重复激活幂等"));
        assert!(session.is_active());
        assert_eq!(session.version().value(), 1);
        assert!(session.fail().is_err());
    }

    #[test]
    fn 恢复时拒绝生命周期与版本矛盾() {
        assert!(
            DirectSession::restore(
                catalog_id(),
                principal_id(),
                agent_id(),
                DirectSessionLifecycle::Active,
                AggregateVersion::INITIAL,
            )
            .is_err()
        );
    }

    #[test]
    fn 任一方向屏蔽都会停止投递并隐藏精确在线状态() {
        let policy = DirectContactPolicy::restore(principal_id(), agent_id(), false, true);

        assert!(!policy.delivery_allowed());
        assert_eq!(
            policy.exact_presence_disclosure(),
            DirectPresenceDisclosure::Hidden
        );
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(1))
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(2))
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(Uuid::from_u128(3))
    }
}
