use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    automation::{AuthorizeAutomationSend, AutomationAuthorizationOutcome, CreateAutomationGrant},
    devices::AuthenticatedDevice,
    ports::{AutomationGrantRecord, MatrixRoomId},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, AutomationGrantId, MessageSubmissionId, RoomCatalogId},
    policy::{
        AutomationAudience, AutomationGrantScope, AutomationMessageKind, AutomationMessageKinds,
        AutomationRiskScanOutcome,
    },
    time::{DurationMillis, UtcMillis},
};
use serde::{Deserialize, Serialize};

use crate::features::resource_ids::parse_uuid_v7;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateAutomationGrantBody {
    agent_id: String,
    #[serde(default)]
    agent_instance_id: Option<String>,
    room_catalog_id: String,
    message_kinds: Vec<AutomationMessageKindBody>,
    audience: AutomationAudienceBody,
    requires_risk_scan: bool,
    max_messages_per_minute: u16,
    #[serde(default)]
    max_total_messages: Option<u32>,
    lifetime_seconds: u32,
    impact_acknowledged: bool,
}

impl CreateAutomationGrantBody {
    pub(super) fn into_request(
        self,
        actor: AuthenticatedPrincipal,
        grant_id: AutomationGrantId,
    ) -> Option<CreateAutomationGrant> {
        let agent_id = parse_uuid_v7(&self.agent_id).map(AgentId::from_uuid).ok()?;
        let agent_instance_id = self
            .agent_instance_id
            .map(|value| parse_uuid_v7(&value).map(AgentInstanceId::from_uuid))
            .transpose()
            .ok()?;
        let room_catalog_id = parse_uuid_v7(&self.room_catalog_id)
            .map(RoomCatalogId::from_uuid)
            .ok()?;
        let message_kinds = AutomationMessageKinds::new(
            self.message_kinds
                .into_iter()
                .map(AutomationMessageKindBody::into_domain),
        )
        .ok()?;
        let scope = AutomationGrantScope::new(
            agent_id,
            agent_instance_id,
            room_catalog_id,
            message_kinds,
            self.audience.into_domain(),
            self.requires_risk_scan,
        )
        .ok()?;
        let lifetime_millis = u64::from(self.lifetime_seconds).checked_mul(1_000)?;
        Some(CreateAutomationGrant {
            actor,
            grant_id,
            scope,
            max_messages_per_minute: self.max_messages_per_minute,
            max_total_messages: self.max_total_messages,
            lifetime: DurationMillis::new(lifetime_millis).ok()?,
            impact_acknowledged: self.impact_acknowledged,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutomationMessageKindBody {
    RoomMessage,
    Reply,
}

impl AutomationMessageKindBody {
    const fn into_domain(self) -> AutomationMessageKind {
        match self {
            Self::RoomMessage => AutomationMessageKind::RoomMessage,
            Self::Reply => AutomationMessageKind::Reply,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutomationAudienceBody {
    KnownRoomMembers,
    AnyRoomMember,
}

impl AutomationAudienceBody {
    const fn into_domain(self) -> AutomationAudience {
        match self {
            Self::KnownRoomMembers => AutomationAudience::KnownRoomMembers,
            Self::AnyRoomMember => AutomationAudience::AnyRoomMember,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AuthorizeAutomationSendBody {
    submission_id: String,
    agent_id: String,
    agent_instance_id: String,
    room_catalog_id: String,
    matrix_room_id: String,
    message_kind: AutomationMessageKindBody,
    risk_scan: AutomationRiskScanBody,
}

impl AuthorizeAutomationSendBody {
    pub(super) fn into_request(
        self,
        actor: AuthenticatedDevice,
        grant_id: AutomationGrantId,
    ) -> Option<AuthorizeAutomationSend> {
        Some(AuthorizeAutomationSend {
            actor,
            grant_id,
            submission_id: parse_uuid_v7(&self.submission_id)
                .map(MessageSubmissionId::from_uuid)
                .ok()?,
            agent_id: parse_uuid_v7(&self.agent_id).map(AgentId::from_uuid).ok()?,
            agent_instance_id: parse_uuid_v7(&self.agent_instance_id)
                .map(AgentInstanceId::from_uuid)
                .ok()?,
            room_catalog_id: parse_uuid_v7(&self.room_catalog_id)
                .map(RoomCatalogId::from_uuid)
                .ok()?,
            matrix_room_id: MatrixRoomId::new(self.matrix_room_id).ok()?,
            is_reply: matches!(self.message_kind, AutomationMessageKindBody::Reply),
            risk_scan: self.risk_scan.into_domain(),
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutomationRiskScanBody {
    Passed,
    Rejected,
    Unavailable,
    NotRequested,
}

impl AutomationRiskScanBody {
    const fn into_domain(self) -> AutomationRiskScanOutcome {
        match self {
            Self::Passed => AutomationRiskScanOutcome::Passed,
            Self::Rejected => AutomationRiskScanOutcome::Rejected,
            Self::Unavailable => AutomationRiskScanOutcome::Unavailable,
            Self::NotRequested => AutomationRiskScanOutcome::NotRequested,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AutomationGrantListResponse {
    grants: Vec<AutomationGrantResponse>,
}

impl From<Vec<AutomationGrantRecord>> for AutomationGrantListResponse {
    fn from(records: Vec<AutomationGrantRecord>) -> Self {
        Self {
            grants: records
                .into_iter()
                .map(AutomationGrantResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AutomationGrantResponse {
    grant_id: String,
    agent_id: String,
    agent_instance_id: Option<String>,
    room_catalog_id: String,
    message_kinds: Vec<&'static str>,
    audience: &'static str,
    requires_risk_scan: bool,
    max_messages_per_minute: u16,
    max_total_messages: Option<u32>,
    starts_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    status: &'static str,
    revoked_at_unix_ms: Option<i64>,
    total_messages: u32,
    messages_in_current_minute: u32,
}

impl From<AutomationGrantRecord> for AutomationGrantResponse {
    fn from(record: AutomationGrantRecord) -> Self {
        let scope = record.grant.scope();
        let limits = record.grant.limits();
        Self {
            grant_id: record.grant.id().to_string(),
            agent_id: scope.agent_id().to_string(),
            agent_instance_id: scope.agent_instance_id().map(|id| id.to_string()),
            room_catalog_id: scope.room_catalog_id().to_string(),
            message_kinds: scope
                .message_kinds()
                .iter()
                .map(AutomationMessageKind::as_str)
                .collect(),
            audience: scope.audience().as_str(),
            requires_risk_scan: scope.requires_risk_scan(),
            max_messages_per_minute: limits.max_messages_per_minute(),
            max_total_messages: limits.max_total_messages(),
            starts_at_unix_ms: limits.starts_at().value(),
            expires_at_unix_ms: limits.expires_at().value(),
            status: record.grant.status().as_str(),
            revoked_at_unix_ms: record.grant.revoked_at().map(UtcMillis::value),
            total_messages: record.usage.total_messages,
            messages_in_current_minute: record.usage.messages_in_current_minute,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AutomationAuthorizationResponse {
    decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    reused: bool,
}

impl From<AutomationAuthorizationOutcome> for AutomationAuthorizationResponse {
    fn from(outcome: AutomationAuthorizationOutcome) -> Self {
        match outcome {
            AutomationAuthorizationOutcome::Authorized(receipt) => Self {
                decision: "authorized",
                reason: None,
                reused: receipt.reused,
            },
            AutomationAuthorizationOutcome::Denied(reason) => Self {
                decision: "denied",
                reason: Some(reason.as_str()),
                reused: false,
            },
        }
    }
}

pub(super) fn grant_id(value: &str) -> Option<AutomationGrantId> {
    parse_uuid_v7(value).map(AutomationGrantId::from_uuid).ok()
}
