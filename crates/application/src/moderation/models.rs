use agent_room_domain::{
    ids::{ModerationActionId, ModerationCaseId, RoomCatalogId},
    moderation::{ModerationActionKind, ModerationEvidence, ModerationReason, ModerationTarget},
    time::UtcMillis,
};

use crate::authentication::AuthenticatedPrincipal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitModerationReport {
    pub actor: AuthenticatedPrincipal,
    pub target: ModerationTarget,
    pub reason: ModerationReason,
    pub description: String,
    pub evidence: ModerationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMyModerationCases {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyModerationAction {
    pub actor: AuthenticatedPrincipal,
    pub case_id: Option<ModerationCaseId>,
    pub room_catalog_id: RoomCatalogId,
    pub kind: ModerationActionKind,
    pub target: ModerationTarget,
    pub reason: ModerationReason,
    pub expires_at: Option<UtcMillis>,
    pub impact_acknowledged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverseModerationAction {
    pub actor: AuthenticatedPrincipal,
    pub action_id: ModerationActionId,
    pub impact_acknowledged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRoomModeration {
    pub actor: AuthenticatedPrincipal,
    pub room_catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListModerationAudit {
    pub actor: AuthenticatedPrincipal,
    pub room_catalog_id: Option<RoomCatalogId>,
    pub limit: u16,
}
