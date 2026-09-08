use agent_room_agent_client::reception::{ReceptionCheckpoint, ReceptionPolicy};
use agent_room_bridge_ipc::{IpcListPreviewsRequest, IpcMethod, IpcOpenHostSessionRequest};
use serde::{Deserialize, Serialize};

use crate::{HostBinding, ReceptionFailure, ReceptionResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiverBinding {
    pub session: IpcOpenHostSessionRequest,
    pub policy: ReceptionPolicy,
    pub automation_grant_id: String,
    pub host: HostBinding,
    pub start: ReceiverStart,
}

impl ReceiverBinding {
    /// Validate the wire contract without requiring installed executables.
    /// # Errors
    /// Invalid identities, cursors or scopes cannot become durable bindings.
    pub fn validate(&self) -> ReceptionResult<()> {
        validate_task_id(&self.host.task_id)?;
        IpcMethod::OpenHostSession(self.session.clone())
            .validate()
            .map_err(|e| ReceptionFailure::validation(e.code()))?;
        let grant = uuid::Uuid::parse_str(&self.automation_grant_id)
            .map_err(|_| ReceptionFailure::validation("receiver.automation_grant_invalid"))?;
        if grant.get_version() != Some(uuid::Version::SortRand)
            || grant.to_string() != self.automation_grant_id
        {
            return Err(ReceptionFailure::validation(
                "receiver.automation_grant_invalid",
            ));
        }
        validate_task_id(&self.policy.allowed_principal_id)
            .map_err(|_| ReceptionFailure::validation("receiver.principal_invalid"))?;
        let cursor = match &self.start {
            ReceiverStart::After { event_id } => Some(event_id.clone()),
            _ => None,
        };
        IpcMethod::ReadInbox(self.inbox_request(cursor))
            .validate()
            .map_err(|e| ReceptionFailure::validation(e.code()))
    }

    pub(crate) fn inbox_request(&self, after_event_id: Option<String>) -> IpcListPreviewsRequest {
        IpcListPreviewsRequest {
            room_id: Some(self.policy.room_id.clone()),
            after_event_id,
            before_event_id: None,
            limit: 50,
        }
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.session.session_key == other.session.session_key
            && self.session.display_name == other.session.display_name
            && self.host.task_id == other.host.task_id
            && self.host.host_type == other.host.host_type
            && self.policy == other.policy
            && self.start == other.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiverStart {
    Now,
    Beginning,
    After {
        #[serde(rename = "eventId")]
        event_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiverState {
    pub binding: ReceiverBinding,
    pub bridge_service: String,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub room_catalog_id: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
    pub checkpoint: ReceptionCheckpoint,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_delivery: Option<DeliveryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryRecord {
    pub event_id: String,
    pub message_id: String,
    pub submission_id: String,
    pub stage: DeliveryStage,
    pub reply_event_id: Option<String>,
    pub failure: Option<ReceptionFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    Received,
    Running,
    Verifying,
    Replied,
    NeedsReview,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReceiverEvent {
    Ready {
        agent_id: String,
        host_task_id: String,
        room_id: String,
    },
    Delivery {
        record: DeliveryRecord,
    },
    Reconnecting {
        error: ReceptionFailure,
        retry_in_seconds: u64,
    },
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Retry,
    Skip,
}

pub(crate) fn validate_task_id(value: &str) -> ReceptionResult<()> {
    let id = uuid::Uuid::parse_str(value)
        .map_err(|_| ReceptionFailure::validation("receiver.task_id_invalid"))?;
    if id.is_nil() || id.to_string() != value {
        return Err(ReceptionFailure::validation("receiver.task_id_invalid"));
    }
    Ok(())
}
