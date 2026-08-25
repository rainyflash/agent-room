use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    moderation::{ApplyModerationAction, SubmitModerationReport},
};
use agent_room_domain::{
    ids::{ModerationActionId, ModerationCaseId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditEvent, ModerationCase,
        ModerationEvidence, ModerationReason, ModerationTarget, ModerationTargetKind,
    },
    time::UtcMillis,
};
use serde::{Deserialize, Serialize};

use crate::features::resource_ids::parse_uuid_v7;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SubmitReportBody {
    target_kind: ModerationTargetKindBody,
    target_reference: String,
    reason: ModerationReasonBody,
    #[serde(default)]
    description: String,
    evidence: ModerationEvidenceBody,
}

impl SubmitReportBody {
    pub(super) fn into_request(
        self,
        actor: AuthenticatedPrincipal,
        case_id: ModerationCaseId,
    ) -> Option<SubmitModerationReport> {
        Some(SubmitModerationReport {
            actor,
            case_id,
            target: ModerationTarget::new(self.target_kind.into_domain(), self.target_reference)
                .ok()?,
            reason: self.reason.into_domain(),
            description: self.description,
            evidence: self.evidence.into_domain()?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModerationEvidenceBody {
    #[serde(default)]
    room_catalog_id: Option<String>,
    #[serde(default)]
    matrix_event_id: Option<String>,
    #[serde(default)]
    reporter_submitted_excerpt: Option<String>,
    end_to_end_encrypted: bool,
}

impl ModerationEvidenceBody {
    fn into_domain(self) -> Option<ModerationEvidence> {
        let room_catalog_id = self
            .room_catalog_id
            .map(|value| parse_uuid_v7(&value).map(RoomCatalogId::from_uuid))
            .transpose()
            .ok()?;
        ModerationEvidence::new(
            room_catalog_id,
            self.matrix_event_id,
            self.reporter_submitted_excerpt,
            self.end_to_end_encrypted,
        )
        .ok()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ApplyActionBody {
    #[serde(default)]
    case_id: Option<String>,
    kind: ModerationActionKindBody,
    target_kind: ModerationTargetKindBody,
    target_reference: String,
    reason: ModerationReasonBody,
    #[serde(default)]
    expires_at_unix_ms: Option<i64>,
    impact_acknowledged: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReverseActionBody {
    impact_acknowledged: bool,
}

impl ReverseActionBody {
    pub(super) const fn impact_acknowledged(&self) -> bool {
        self.impact_acknowledged
    }
}

impl ApplyActionBody {
    pub(super) fn into_request(
        self,
        actor: AuthenticatedPrincipal,
        action_id: ModerationActionId,
        room_catalog_id: RoomCatalogId,
    ) -> Option<ApplyModerationAction> {
        Some(ApplyModerationAction {
            actor,
            action_id,
            case_id: self
                .case_id
                .map(|value| parse_uuid_v7(&value).map(ModerationCaseId::from_uuid))
                .transpose()
                .ok()?,
            room_catalog_id,
            kind: self.kind.into_domain(),
            target: ModerationTarget::new(self.target_kind.into_domain(), self.target_reference)
                .ok()?,
            reason: self.reason.into_domain(),
            expires_at: self
                .expires_at_unix_ms
                .map(UtcMillis::new)
                .transpose()
                .ok()?,
            impact_acknowledged: self.impact_acknowledged,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModerationTargetKindBody {
    Principal,
    Agent,
    Room,
    Event,
    FederationPeer,
}

impl ModerationTargetKindBody {
    const fn into_domain(self) -> ModerationTargetKind {
        match self {
            Self::Principal => ModerationTargetKind::Principal,
            Self::Agent => ModerationTargetKind::Agent,
            Self::Room => ModerationTargetKind::Room,
            Self::Event => ModerationTargetKind::Event,
            Self::FederationPeer => ModerationTargetKind::FederationPeer,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModerationReasonBody {
    Spam,
    Harassment,
    Impersonation,
    MaliciousContent,
    PrivacyViolation,
    UnsafeAutomation,
    Other,
}

impl ModerationReasonBody {
    const fn into_domain(self) -> ModerationReason {
        match self {
            Self::Spam => ModerationReason::Spam,
            Self::Harassment => ModerationReason::Harassment,
            Self::Impersonation => ModerationReason::Impersonation,
            Self::MaliciousContent => ModerationReason::MaliciousContent,
            Self::PrivacyViolation => ModerationReason::PrivacyViolation,
            Self::UnsafeAutomation => ModerationReason::UnsafeAutomation,
            Self::Other => ModerationReason::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModerationActionKindBody {
    Hide,
    Mute,
    Kick,
    Ban,
}

impl ModerationActionKindBody {
    const fn into_domain(self) -> ModerationActionKind {
        match self {
            Self::Hide => ModerationActionKind::Hide,
            Self::Mute => ModerationActionKind::Mute,
            Self::Kick => ModerationActionKind::Kick,
            Self::Ban => ModerationActionKind::Ban,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModerationCaseListResponse {
    cases: Vec<ModerationCaseResponse>,
}

impl From<Vec<ModerationCase>> for ModerationCaseListResponse {
    fn from(cases: Vec<ModerationCase>) -> Self {
        Self {
            cases: cases
                .into_iter()
                .map(ModerationCaseResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModerationCaseResponse {
    case_id: String,
    target_kind: &'static str,
    target_reference: String,
    reason: &'static str,
    description: String,
    evidence: ModerationEvidenceResponse,
    state: &'static str,
    created_at_unix_ms: i64,
    resolved_at_unix_ms: Option<i64>,
}

impl From<ModerationCase> for ModerationCaseResponse {
    fn from(case: ModerationCase) -> Self {
        Self {
            case_id: case.id().to_string(),
            target_kind: case.target().kind().as_str(),
            target_reference: case.target().reference().to_owned(),
            reason: case.reason().as_str(),
            description: case.description().to_owned(),
            evidence: ModerationEvidenceResponse::from(case.evidence()),
            state: case.state().as_str(),
            created_at_unix_ms: case.created_at().value(),
            resolved_at_unix_ms: case.resolved_at().map(UtcMillis::value),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModerationEvidenceResponse {
    room_catalog_id: Option<String>,
    matrix_event_id: Option<String>,
    reporter_submitted_excerpt: Option<String>,
    end_to_end_encrypted: bool,
}

impl From<&ModerationEvidence> for ModerationEvidenceResponse {
    fn from(evidence: &ModerationEvidence) -> Self {
        Self {
            room_catalog_id: evidence.room_catalog_id().map(|id| id.to_string()),
            matrix_event_id: evidence.matrix_event_id().map(str::to_owned),
            reporter_submitted_excerpt: evidence.reporter_submitted_excerpt().map(str::to_owned),
            end_to_end_encrypted: evidence.end_to_end_encrypted(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModerationActionListResponse {
    actions: Vec<ModerationActionResponse>,
}

impl From<Vec<ModerationAction>> for ModerationActionListResponse {
    fn from(actions: Vec<ModerationAction>) -> Self {
        Self {
            actions: actions
                .into_iter()
                .map(ModerationActionResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModerationActionResponse {
    action_id: String,
    case_id: Option<String>,
    actor_principal_id: String,
    room_catalog_id: String,
    kind: &'static str,
    target_kind: &'static str,
    target_reference: String,
    reason: &'static str,
    starts_at_unix_ms: i64,
    expires_at_unix_ms: Option<i64>,
    status: &'static str,
    failure_code: Option<String>,
    reversed_at_unix_ms: Option<i64>,
}

impl From<ModerationAction> for ModerationActionResponse {
    fn from(action: ModerationAction) -> Self {
        Self {
            action_id: action.id().to_string(),
            case_id: action.case_id().map(|id| id.to_string()),
            actor_principal_id: action.actor_principal_id().to_string(),
            room_catalog_id: action.room_catalog_id().to_string(),
            kind: action.kind().as_str(),
            target_kind: action.target().kind().as_str(),
            target_reference: action.target().reference().to_owned(),
            reason: action.reason().as_str(),
            starts_at_unix_ms: action.starts_at().value(),
            expires_at_unix_ms: action.expires_at().map(UtcMillis::value),
            status: action.status().as_str(),
            failure_code: action.failure_code().map(str::to_owned),
            reversed_at_unix_ms: action.reversed_at().map(UtcMillis::value),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AuditQuery {
    #[serde(default)]
    room_catalog_id: Option<String>,
    #[serde(default)]
    limit: Option<u16>,
}

impl AuditQuery {
    pub(super) fn room_catalog_id(&self) -> Result<Option<RoomCatalogId>, ()> {
        match self.room_catalog_id.as_deref() {
            None => Ok(None),
            Some(value) => parse_uuid_v7(value).map(RoomCatalogId::from_uuid).map(Some),
        }
    }

    pub(super) fn limit(&self) -> u16 {
        self.limit.unwrap_or(100)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModerationAuditListResponse {
    events: Vec<ModerationAuditResponse>,
}

impl From<Vec<ModerationAuditEvent>> for ModerationAuditListResponse {
    fn from(events: Vec<ModerationAuditEvent>) -> Self {
        Self {
            events: events
                .into_iter()
                .map(ModerationAuditResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModerationAuditResponse {
    event_id: String,
    occurred_at_unix_ms: i64,
    actor_principal_id: String,
    action: String,
    target_kind: &'static str,
    target_reference: String,
    outcome: &'static str,
    reason: Option<&'static str>,
    correlation_id: String,
    room_catalog_id: Option<String>,
}

impl From<ModerationAuditEvent> for ModerationAuditResponse {
    fn from(event: ModerationAuditEvent) -> Self {
        Self {
            event_id: event.id.to_string(),
            occurred_at_unix_ms: event.occurred_at.value(),
            actor_principal_id: event.actor_principal_id.to_string(),
            action: event.action,
            target_kind: event.target.kind().as_str(),
            target_reference: event.target.reference().to_owned(),
            outcome: event.outcome.as_str(),
            reason: event.reason.map(ModerationReason::as_str),
            correlation_id: event.correlation_id.to_string(),
            room_catalog_id: event.room_catalog_id.map(|id| id.to_string()),
        }
    }
}

pub(super) fn case_id(value: &str) -> Option<ModerationCaseId> {
    parse_uuid_v7(value).map(ModerationCaseId::from_uuid).ok()
}

pub(super) fn action_id(value: &str) -> Option<ModerationActionId> {
    parse_uuid_v7(value).map(ModerationActionId::from_uuid).ok()
}

pub(super) fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}
