use crate::{DomainError, DomainResult, ids::PrincipalId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalStatus {
    Active,
    Suspended,
    Deleted,
}

impl PrincipalStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    id: PrincipalId,
    status: PrincipalStatus,
}

impl Principal {
    pub const fn new(id: PrincipalId) -> Self {
        Self {
            id,
            status: PrincipalStatus::Active,
        }
    }

    pub const fn id(&self) -> PrincipalId {
        self.id
    }

    pub const fn status(&self) -> PrincipalStatus {
        self.status
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
            PrincipalStatus::Deleted => Err(DomainError::InvalidTransition {
                entity: "principal",
                from: self.status.label(),
                to: "suspended",
            }),
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
}
