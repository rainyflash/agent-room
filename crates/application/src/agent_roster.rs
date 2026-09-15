use crate::{
    authentication::AuthenticatedPrincipal,
    moderation::{ModerationFailure, ModerationFailureKind, ModerationResult},
    ports::{Clock, MatrixResult, MatrixRoomId, ModerationAuthority, PortFuture},
};
use agent_room_domain::{
    agent_lifecycle::AgentRosterPolicy,
    ids::RoomCatalogId,
    moderation::{ModerationTarget, ModerationTargetKind},
};
use std::sync::Arc;

pub const AGENT_ROSTER_POLICY_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.roster.policy.v1";

pub trait AgentRosterPolicyPublisher: Send + Sync {
    fn publish<'a>(
        &'a self,
        room: &'a MatrixRoomId,
        policy: AgentRosterPolicy,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

pub struct AgentRosterService {
    authority: Arc<dyn ModerationAuthority>,
    publisher: Arc<dyn AgentRosterPolicyPublisher>,
    clock: Arc<dyn Clock>,
}

impl AgentRosterService {
    pub fn new(
        authority: Arc<dyn ModerationAuthority>,
        publisher: Arc<dyn AgentRosterPolicyPublisher>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            authority,
            publisher,
            clock,
        }
    }

    /// A room rule, not a per-device hide preference. Reconnection never requires re-creation.
    /// # Errors
    /// Rejects expired sessions, non-managers and unavailable authority/publication services.
    pub async fn update(
        &self,
        actor: AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        policy: AgentRosterPolicy,
    ) -> ModerationResult<()> {
        const OP: &str = "agent_roster.update";
        let failure = |kind| ModerationFailure::new(OP, kind);
        if self.clock.now() >= actor.expires_at {
            return Err(failure(ModerationFailureKind::Forbidden));
        }
        let target = ModerationTarget::new(ModerationTargetKind::Room, catalog_id.to_string())
            .map_err(|_| failure(ModerationFailureKind::InvalidRequest))?;
        let room = self
            .authority
            .inspect_room(actor.principal_id, catalog_id, &target)
            .await
            .map_err(|_| failure(ModerationFailureKind::DependencyUnavailable))?
            .ok_or_else(|| failure(ModerationFailureKind::NotFound))?;
        if !room.role.can_moderate_room() {
            return Err(failure(ModerationFailureKind::Forbidden));
        }
        self.publisher
            .publish(&room.matrix_room_id, policy)
            .await
            .map_err(|_| failure(ModerationFailureKind::DependencyUnavailable))
    }
}
