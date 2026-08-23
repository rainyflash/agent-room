use crate::{DomainError, DomainResult, ids::PrincipalId, version::AggregateVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalStatus {
    Active,
    Suspended,
    Deleting,
    Deleted,
}

impl PrincipalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleting => "deleting",
            Self::Deleted => "deleted",
        }
    }
}

impl TryFrom<&str> for PrincipalStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "deleting" => Ok(Self::Deleting),
            "deleted" => Ok(Self::Deleted),
            _ => Err(DomainError::Validation {
                field: "principal_status",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    id: PrincipalId,
    status: PrincipalStatus,
    version: AggregateVersion,
}

impl Principal {
    pub const fn new(id: PrincipalId) -> Self {
        Self {
            id,
            status: PrincipalStatus::Active,
            version: AggregateVersion::INITIAL,
        }
    }

    pub const fn restore(
        id: PrincipalId,
        status: PrincipalStatus,
        version: AggregateVersion,
    ) -> Self {
        Self {
            id,
            status,
            version,
        }
    }

    pub const fn id(&self) -> PrincipalId {
        self.id
    }

    pub const fn status(&self) -> PrincipalStatus {
        self.status
    }

    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    /// 只有活跃主体可以建立或继续使用认证会话。
    pub const fn allows_authentication(&self) -> bool {
        matches!(self.status, PrincipalStatus::Active)
    }

    pub const fn restore_version(&mut self, version: AggregateVersion) {
        self.version = version;
    }

    /// 暂停主体，重复暂停保持幂等。
    ///
    /// # Errors
    ///
    /// 已删除主体不能再次进入暂停状态。
    pub fn suspend(&mut self) -> DomainResult<()> {
        match self.status {
            PrincipalStatus::Active | PrincipalStatus::Suspended => {
                self.status = PrincipalStatus::Suspended;
                Ok(())
            }
            PrincipalStatus::Deleting | PrincipalStatus::Deleted => {
                Err(DomainError::InvalidTransition {
                    entity: "principal",
                    from: self.status.as_str(),
                    to: "suspended",
                })
            }
        }
    }

    pub fn begin_deletion(&mut self) {
        if self.status != PrincipalStatus::Deleted {
            self.status = PrincipalStatus::Deleting;
        }
    }

    pub fn delete(&mut self) {
        self.status = PrincipalStatus::Deleted;
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{Principal, PrincipalStatus};
    use crate::ids::PrincipalId;

    #[test]
    fn 暂停操作保持幂等且删除后不可恢复() {
        let mut principal = Principal::new(PrincipalId::from_uuid(Uuid::from_u128(1)));

        principal.suspend().expect("首次暂停应成功");
        principal.suspend().expect("重复暂停应保持成功");
        assert_eq!(principal.status(), PrincipalStatus::Suspended);

        principal.delete();
        assert!(principal.suspend().is_err());
    }

    #[test]
    fn 只有活跃主体允许认证() {
        let mut principal = Principal::new(PrincipalId::from_uuid(Uuid::from_u128(2)));
        assert!(principal.allows_authentication());

        principal.suspend().expect("主体可暂停");
        assert!(!principal.allows_authentication());
    }
}
