use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, AutomationGrantId, MessageSubmissionId, RoomCatalogId},
    policy::{AutomationGrantScope, AutomationRiskScanOutcome},
    time::DurationMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    ports::{AutomationGrantRecord, MatrixRoomId},
};

use super::AutomationSendDenial;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAutomationGrant {
    pub actor: AuthenticatedPrincipal,
    pub grant_id: AutomationGrantId,
    pub scope: AutomationGrantScope,
    pub max_messages_per_minute: u16,
    pub max_total_messages: Option<u32>,
    pub lifetime: DurationMillis,
    pub impact_acknowledged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAutomationGrants {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeAutomationGrant {
    pub actor: AuthenticatedPrincipal,
    pub grant_id: AutomationGrantId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeAutomationSend {
    pub actor: AuthenticatedDevice,
    pub grant_id: AutomationGrantId,
    pub submission_id: MessageSubmissionId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub room_catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomId,
    pub is_reply: bool,
    pub risk_scan: AutomationRiskScanOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationAuthorizationReceipt {
    pub grant_id: AutomationGrantId,
    pub submission_id: MessageSubmissionId,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationAuthorizationOutcome {
    Authorized(AutomationAuthorizationReceipt),
    Denied(AutomationSendDenial),
}

pub type AutomationGrantList = Vec<AutomationGrantRecord>;
