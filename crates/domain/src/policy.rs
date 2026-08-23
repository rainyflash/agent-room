use crate::{
    ids::{AgentId, AutomationGrantId, PrincipalId, RoomCatalogId},
    time::UtcMillis,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationGrantStatus {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationGrant {
    id: AutomationGrantId,
    grantor_id: PrincipalId,
    agent_id: AgentId,
    room_catalog_id: RoomCatalogId,
    expires_at: UtcMillis,
    status: AutomationGrantStatus,
}

impl AutomationGrant {
    pub const fn issue(
        id: AutomationGrantId,
        grantor_id: PrincipalId,
        agent_id: AgentId,
        room_catalog_id: RoomCatalogId,
        expires_at: UtcMillis,
    ) -> Self {
        Self {
            id,
            grantor_id,
            agent_id,
            room_catalog_id,
            expires_at,
            status: AutomationGrantStatus::Active,
        }
    }

    pub const fn id(&self) -> AutomationGrantId {
        self.id
    }

    pub const fn grantor_id(&self) -> PrincipalId {
        self.grantor_id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn room_catalog_id(&self) -> RoomCatalogId {
        self.room_catalog_id
    }

    pub const fn status(&self) -> AutomationGrantStatus {
        self.status
    }

    pub fn is_effective(&mut self, now: UtcMillis) -> bool {
        if self.status == AutomationGrantStatus::Active && now >= self.expires_at {
            self.status = AutomationGrantStatus::Expired;
        }

        self.status == AutomationGrantStatus::Active
    }

    pub fn revoke(&mut self) {
        if self.status == AutomationGrantStatus::Active {
            self.status = AutomationGrantStatus::Revoked;
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{AutomationGrant, AutomationGrantStatus};
    use crate::{
        ids::{AgentId, AutomationGrantId, PrincipalId, RoomCatalogId},
        time::UtcMillis,
    };

    proptest! {
        #[test]
        fn 任意次数撤销都保持不可用(revocations in 1_usize..100) {
            let mut grant = AutomationGrant::issue(
                AutomationGrantId::from_uuid(Uuid::from_u128(1)),
                PrincipalId::from_uuid(Uuid::from_u128(2)),
                AgentId::from_uuid(Uuid::from_u128(3)),
                RoomCatalogId::from_uuid(Uuid::from_u128(4)),
                UtcMillis::new(10_000).expect("测试时间有效"),
            );

            for _ in 0..revocations {
                grant.revoke();
            }

            prop_assert_eq!(grant.status(), AutomationGrantStatus::Revoked);
            prop_assert!(!grant.is_effective(UtcMillis::new(1).expect("测试时间有效")));
        }
    }
}
