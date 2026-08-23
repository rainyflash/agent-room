use crate::{
    DomainError, DomainResult,
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Registered,
    Active,
    Suspended,
    Retired,
}

impl AgentStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    id: AgentId,
    owner_id: PrincipalId,
    status: AgentStatus,
}

impl Agent {
    pub const fn register(id: AgentId, owner_id: PrincipalId) -> Self {
        Self {
            id,
            owner_id,
            status: AgentStatus::Registered,
        }
    }

    pub const fn id(&self) -> AgentId {
        self.id
    }

    pub const fn owner_id(&self) -> PrincipalId {
        self.owner_id
    }

    pub const fn status(&self) -> AgentStatus {
        self.status
    }

    /// 激活已登记的 Agent，重复激活保持幂等。
    ///
    /// # Errors
    ///
    /// 已暂停或退役的 Agent 不能直接激活。
    pub fn activate(&mut self) -> DomainResult<()> {
        match self.status {
            AgentStatus::Registered | AgentStatus::Active => {
                self.status = AgentStatus::Active;
                Ok(())
            }
            AgentStatus::Suspended | AgentStatus::Retired => Err(DomainError::InvalidTransition {
                entity: "agent",
                from: self.status.label(),
                to: "active",
            }),
        }
    }

    /// 暂停 Agent，重复暂停保持幂等。
    ///
    /// # Errors
    ///
    /// 已退役 Agent 不能进入暂停状态。
    pub fn suspend(&mut self) -> DomainResult<()> {
        match self.status {
            AgentStatus::Registered | AgentStatus::Active | AgentStatus::Suspended => {
                self.status = AgentStatus::Suspended;
                Ok(())
            }
            AgentStatus::Retired => Err(DomainError::InvalidTransition {
                entity: "agent",
                from: self.status.label(),
                to: "suspended",
            }),
        }
    }

    pub fn retire(&mut self) {
        self.status = AgentStatus::Retired;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceStatus {
    Offline,
    Online,
    Revoked,
}

impl AgentInstanceStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Online => "online",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    id: AgentInstanceId,
    agent_id: AgentId,
    device_id: DeviceId,
    status: AgentInstanceStatus,
    lease_expires_at: Option<UtcMillis>,
}

impl AgentInstance {
    pub const fn new(id: AgentInstanceId, agent_id: AgentId, device_id: DeviceId) -> Self {
        Self {
            id,
            agent_id,
            device_id,
            status: AgentInstanceStatus::Offline,
            lease_expires_at: None,
        }
    }

    pub const fn id(&self) -> AgentInstanceId {
        self.id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub const fn status(&self) -> AgentInstanceStatus {
        self.status
    }

    pub const fn lease_expires_at(&self) -> Option<UtcMillis> {
        self.lease_expires_at
    }

    /// 连接实例并创建新的在线租约。
    ///
    /// # Errors
    ///
    /// 实例已撤销或租约截止时间溢出时返回错误。
    pub fn connect(&mut self, now: UtcMillis, lease: DurationMillis) -> DomainResult<()> {
        if self.status == AgentInstanceStatus::Revoked {
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: self.status.label(),
                to: "online",
            });
        }

        self.status = AgentInstanceStatus::Online;
        self.lease_expires_at = Some(now.checked_add(lease)?);
        Ok(())
    }

    /// 续租当前在线实例。
    ///
    /// # Errors
    ///
    /// 实例不在线、原租约已经过期或新截止时间溢出时返回错误。
    pub fn renew(&mut self, now: UtcMillis, lease: DurationMillis) -> DomainResult<()> {
        if self.status != AgentInstanceStatus::Online {
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: self.status.label(),
                to: "online",
            });
        }

        if self
            .lease_expires_at
            .is_some_and(|expires_at| expires_at < now)
        {
            self.status = AgentInstanceStatus::Offline;
            self.lease_expires_at = None;
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: "expired",
                to: "online",
            });
        }

        self.lease_expires_at = Some(now.checked_add(lease)?);
        Ok(())
    }

    pub fn expire(&mut self, now: UtcMillis) -> bool {
        let should_expire = self.status == AgentInstanceStatus::Online
            && self
                .lease_expires_at
                .is_some_and(|expires_at| expires_at <= now);

        if should_expire {
            self.status = AgentInstanceStatus::Offline;
            self.lease_expires_at = None;
        }

        should_expire
    }

    pub fn revoke(&mut self) {
        self.status = AgentInstanceStatus::Revoked;
        self.lease_expires_at = None;
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{AgentInstance, AgentInstanceStatus};
    use crate::{
        ids::{AgentId, AgentInstanceId, DeviceId},
        time::{DurationMillis, UtcMillis},
    };

    fn instance() -> AgentInstance {
        AgentInstance::new(
            AgentInstanceId::from_uuid(Uuid::from_u128(1)),
            AgentId::from_uuid(Uuid::from_u128(2)),
            DeviceId::from_uuid(Uuid::from_u128(3)),
        )
    }

    #[test]
    fn 租约到期后必须重新连接而不是续租() {
        let mut instance = instance();
        let now = UtcMillis::new(1_000).expect("测试时间有效");
        let lease = DurationMillis::new(100).expect("测试租约有效");
        instance.connect(now, lease).expect("连接应成功");

        let late = UtcMillis::new(1_101).expect("测试时间有效");
        assert!(instance.renew(late, lease).is_err());
        assert_eq!(instance.status(), AgentInstanceStatus::Offline);
    }

    #[test]
    fn 撤销实例是不可逆操作() {
        let mut instance = instance();
        instance.revoke();
        instance.revoke();

        let now = UtcMillis::new(1_000).expect("测试时间有效");
        let lease = DurationMillis::new(100).expect("测试租约有效");
        assert!(instance.connect(now, lease).is_err());
    }
}
