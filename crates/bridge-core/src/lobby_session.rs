use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomLanguage, RoomRegion},
    time::UtcMillis,
};

use crate::agent_identity::BridgeAgentIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLobbySessionConfig {
    catalog_id: RoomCatalogId,
    preferred_language: Option<RoomLanguage>,
    preferred_region: Option<RoomRegion>,
}

impl AgentLobbySessionConfig {
    pub const fn new(
        catalog_id: RoomCatalogId,
        preferred_language: Option<RoomLanguage>,
        preferred_region: Option<RoomRegion>,
    ) -> Self {
        Self {
            catalog_id,
            preferred_language,
            preferred_region,
        }
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn preferred_language(&self) -> Option<&RoomLanguage> {
        self.preferred_language.as_ref()
    }

    pub const fn preferred_region(&self) -> Option<&RoomRegion> {
        self.preferred_region.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLobbyEntryIntent {
    agent_id: AgentId,
    agent_instance_id: AgentInstanceId,
    catalog_id: RoomCatalogId,
    preferred_language: Option<RoomLanguage>,
    preferred_region: Option<RoomRegion>,
}

impl AgentLobbyEntryIntent {
    fn from_identity(identity: &BridgeAgentIdentity, config: &AgentLobbySessionConfig) -> Self {
        Self {
            agent_id: identity.agent_id(),
            agent_instance_id: identity.agent_instance_id(),
            catalog_id: config.catalog_id,
            preferred_language: config.preferred_language.clone(),
            preferred_region: config.preferred_region.clone(),
        }
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn agent_instance_id(&self) -> AgentInstanceId {
        self.agent_instance_id
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn preferred_language(&self) -> Option<&RoomLanguage> {
        self.preferred_language.as_ref()
    }

    pub const fn preferred_region(&self) -> Option<&RoomRegion> {
        self.preferred_region.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyAssignmentKind {
    New,
    Recovered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedAgentLobby {
    catalog_id: RoomCatalogId,
    room_instance_id: RoomInstanceId,
    matrix_room_id: MatrixRoomReference,
    reservation_id: RoomReservationId,
    assignment: LobbyAssignmentKind,
}

impl JoinedAgentLobby {
    pub const fn new(
        catalog_id: RoomCatalogId,
        room_instance_id: RoomInstanceId,
        matrix_room_id: MatrixRoomReference,
        reservation_id: RoomReservationId,
        assignment: LobbyAssignmentKind,
    ) -> Self {
        Self {
            catalog_id,
            room_instance_id,
            matrix_room_id,
            reservation_id,
            assignment,
        }
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn room_instance_id(&self) -> RoomInstanceId {
        self.room_instance_id
    }

    pub const fn matrix_room_id(&self) -> &MatrixRoomReference {
        &self.matrix_room_id
    }

    pub const fn reservation_id(&self) -> RoomReservationId {
        self.reservation_id
    }

    pub const fn assignment(&self) -> LobbyAssignmentKind {
        self.assignment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlPlaneLobbyEntryOutcome {
    Joined(JoinedAgentLobby),
    ProvisioningBusy { retry_at: UtcMillis },
    CapacityChanged { catalog_id: RoomCatalogId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneLobbyEntryFailureKind {
    InvalidRequest,
    AuthenticationRejected,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    UnknownCommit,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlaneLobbyEntryFailure {
    kind: ControlPlaneLobbyEntryFailureKind,
}

impl ControlPlaneLobbyEntryFailure {
    pub const fn new(kind: ControlPlaneLobbyEntryFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ControlPlaneLobbyEntryFailureKind {
        self.kind
    }
}

pub type ControlPlaneLobbyEntryResult<T> = Result<T, ControlPlaneLobbyEntryFailure>;

pub trait ControlPlaneLobbyEntryGateway: Send + Sync {
    fn enter<'a>(
        &'a self,
        intent: &'a AgentLobbyEntryIntent,
    ) -> PortFuture<'a, ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>>;
}

pub struct AgentLobbySessionService {
    control_plane: Arc<dyn ControlPlaneLobbyEntryGateway>,
}

impl AgentLobbySessionService {
    pub const fn new(control_plane: Arc<dyn ControlPlaneLobbyEntryGateway>) -> Self {
        Self { control_plane }
    }

    /// 让已登记 Agent 实例进入配置的公共大厅，并拒绝控制面返回的错配目录。
    ///
    /// # Errors
    ///
    /// 控制面拒绝、不可用、提交状态未知或返回内容不可信时返回稳定错误。
    pub async fn enter(
        &self,
        identity: &BridgeAgentIdentity,
        config: &AgentLobbySessionConfig,
    ) -> AgentLobbySessionResult<ControlPlaneLobbyEntryOutcome> {
        let intent = AgentLobbyEntryIntent::from_identity(identity, config);
        let outcome = self
            .control_plane
            .enter(&intent)
            .await
            .map_err(map_control_plane_failure)?;
        if !outcome_matches_catalog(&outcome, config.catalog_id) {
            return Err(failure(
                "bridge.lobby.validate_response",
                AgentLobbySessionFailureKind::InvalidControlPlaneResponse,
            ));
        }
        Ok(outcome)
    }
}

fn outcome_matches_catalog(
    outcome: &ControlPlaneLobbyEntryOutcome,
    expected_catalog_id: RoomCatalogId,
) -> bool {
    match outcome {
        ControlPlaneLobbyEntryOutcome::Joined(room) => room.catalog_id() == expected_catalog_id,
        ControlPlaneLobbyEntryOutcome::ProvisioningBusy { .. } => true,
        ControlPlaneLobbyEntryOutcome::CapacityChanged { catalog_id } => {
            *catalog_id == expected_catalog_id
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLobbySessionFailureKind {
    InvalidRequest,
    NotAuthorized,
    Forbidden,
    NotFound,
    Conflict,
    ControlPlaneUnavailable,
    EntryOutcomeUnknown,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLobbySessionFailure {
    operation: &'static str,
    kind: AgentLobbySessionFailureKind,
}

impl AgentLobbySessionFailure {
    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentLobbySessionFailureKind {
        self.kind
    }
}

pub type AgentLobbySessionResult<T> = Result<T, AgentLobbySessionFailure>;

fn map_control_plane_failure(
    control_plane_failure: ControlPlaneLobbyEntryFailure,
) -> AgentLobbySessionFailure {
    let kind = match control_plane_failure.kind() {
        ControlPlaneLobbyEntryFailureKind::InvalidRequest => {
            AgentLobbySessionFailureKind::InvalidRequest
        }
        ControlPlaneLobbyEntryFailureKind::AuthenticationRejected => {
            AgentLobbySessionFailureKind::NotAuthorized
        }
        ControlPlaneLobbyEntryFailureKind::Forbidden => AgentLobbySessionFailureKind::Forbidden,
        ControlPlaneLobbyEntryFailureKind::NotFound => AgentLobbySessionFailureKind::NotFound,
        ControlPlaneLobbyEntryFailureKind::Conflict => AgentLobbySessionFailureKind::Conflict,
        ControlPlaneLobbyEntryFailureKind::Unavailable => {
            AgentLobbySessionFailureKind::ControlPlaneUnavailable
        }
        ControlPlaneLobbyEntryFailureKind::UnknownCommit => {
            AgentLobbySessionFailureKind::EntryOutcomeUnknown
        }
        ControlPlaneLobbyEntryFailureKind::InvalidResponse => {
            AgentLobbySessionFailureKind::InvalidControlPlaneResponse
        }
        ControlPlaneLobbyEntryFailureKind::Internal => AgentLobbySessionFailureKind::Internal,
    };
    failure("bridge.lobby.enter", kind)
}

const fn failure(
    operation: &'static str,
    kind: AgentLobbySessionFailureKind,
) -> AgentLobbySessionFailure {
    AgentLobbySessionFailure { operation, kind }
}
