//! Validated transport and persistence contract for supervised background reception.
use crate::{persistence::RepositoryResult, ports::PortFuture};
use agent_room_domain::{
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId},
    rooms::MatrixRoomReference,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionPending {
    pub event_id: String,
    pub message_id: Uuid,
    pub submission_id: Uuid,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionProgress {
    pub after_event_id: Option<String>,
    pub pending: Option<ReceptionPending>,
    /// An explicit retry keeps its original submission identity across computers.
    #[serde(default)]
    pub retry: Option<ReceptionPending>,
}
impl ReceptionProgress {
    pub fn valid(&self) -> bool {
        let event =
            |id: &str| id.starts_with('$') && id.len() <= 2048 && !id.chars().any(char::is_control);
        self.after_event_id.as_deref().is_none_or(event)
            && !(self.pending.is_some() && self.retry.is_some())
            && self.pending.iter().chain(self.retry.iter()).all(|pending| {
                event(&pending.event_id) && v7(pending.message_id) && v7(pending.submission_id)
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceptionCommand {
    Claim {
        #[serde(rename = "sessionKey")]
        session_key: Uuid,
        #[serde(rename = "displayName")]
        display_name: String,
        initial: ReceptionProgress,
    },
    Heartbeat,
    Save {
        revision: i64,
        progress: ReceptionProgress,
    },
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionRequest {
    pub agent_id: Uuid,
    pub instance_id: Uuid,
    pub catalog_id: Uuid,
    pub room_id: String,
    pub run_id: Uuid,
    pub command: ReceptionCommand,
}
impl ReceptionRequest {
    pub fn valid(&self) -> bool {
        [
            self.agent_id,
            self.instance_id,
            self.catalog_id,
            self.run_id,
        ]
        .into_iter()
        .all(v7)
            && MatrixRoomReference::new(self.room_id.clone()).is_ok()
            && match &self.command {
                ReceptionCommand::Claim {
                    session_key,
                    display_name,
                    initial,
                } => {
                    v7(*session_key)
                        && !display_name.trim().is_empty()
                        && display_name.chars().count() <= 128
                        && display_name.trim() == display_name
                        && !display_name.chars().any(char::is_control)
                        && initial.valid()
                }
                ReceptionCommand::Save { revision, progress } => *revision >= 0 && progress.valid(),
                ReceptionCommand::Heartbeat | ReceptionCommand::Release => true,
            }
    }
}
fn v7(id: Uuid) -> bool {
    id.get_version() == Some(uuid::Version::SortRand)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceptionStatus {
    Active,
    Draining,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionRecord {
    pub agent_id: Uuid,
    pub catalog_id: Uuid,
    pub room_id: String,
    pub session_key: Uuid,
    pub display_name: String,
    pub instance_id: Uuid,
    pub device_id: Uuid,
    pub device_label: String,
    pub run_id: Uuid,
    pub status: ReceptionStatus,
    pub next_device_id: Option<Uuid>,
    pub last_seen_unix_ms: i64,
    pub revision: i64,
    pub progress: ReceptionProgress,
}

/// The repository checks current ownership/device authority and serializes every
/// transition on the logical Agent and room, including concurrent first claims.
pub trait ReceptionRepository: Send + Sync {
    fn execute<'a>(
        &'a self,
        principal: PrincipalId,
        device: DeviceId,
        request: &'a ReceptionRequest,
    ) -> PortFuture<'a, RepositoryResult<ReceptionRecord>>;
    fn list(
        &self,
        principal: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReceptionRecord>>>;
    fn drain(
        &self,
        principal: PrincipalId,
        agent: AgentId,
        catalog: RoomCatalogId,
        next_device: Option<DeviceId>,
    ) -> PortFuture<'_, RepositoryResult<ReceptionRecord>>;
}

#[derive(Debug, Clone, Copy)]
pub struct ReceptionControlFailure {
    pub code: &'static str,
    pub retryable: bool,
}
pub trait ReceptionControlGateway: Send + Sync {
    fn execute(
        &self,
        request: ReceptionRequest,
    ) -> PortFuture<'_, Result<ReceptionRecord, ReceptionControlFailure>>;
}
