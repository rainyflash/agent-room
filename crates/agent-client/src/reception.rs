//! Ordered delivery decisions, independent of process execution and storage.
use agent_room_bridge_ipc::{IpcActorSummary, IpcMessagePreviewSummary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionPolicy {
    pub room_id: String,
    pub allowed_principal_id: String,
}

impl ReceptionPolicy {
    /// Only direct mentions from the configured human sender can start a host turn.
    pub fn accepts(&self, message: &IpcMessagePreviewSummary, agent_matrix_user_id: &str) -> bool {
        message.room_id == self.room_id
            && matches!(&message.actor, IpcActorSummary::Human { principal_id, .. } if principal_id == &self.allowed_principal_id)
            && message
                .conversation
                .as_ref()
                .is_some_and(|chat| chat.mentions.iter().any(|id| id == agent_matrix_user_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceptionCheckpoint {
    #[serde(rename_all = "camelCase")]
    Ready { after_event_id: Option<String> },
    #[serde(rename_all = "camelCase")]
    Pending {
        after_event_id: Option<String>,
        event_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryDecision {
    Skip,
    Deliver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointFailure {
    PendingReviewRequired,
    EventMismatch,
}

impl ReceptionCheckpoint {
    pub fn cursor(&self) -> Option<&str> {
        match self {
            Self::Ready { after_event_id } | Self::Pending { after_event_id, .. } => {
                after_event_id.as_deref()
            }
        }
    }

    /// Persist the resulting checkpoint before invoking a host, even for skipped messages.
    ///
    /// # Errors
    /// An uncertain previous delivery must be resolved explicitly before reading more messages.
    pub fn prepare(
        &mut self,
        message: &IpcMessagePreviewSummary,
        policy: &ReceptionPolicy,
        agent_matrix_user_id: &str,
    ) -> Result<DeliveryDecision, CheckpointFailure> {
        let Self::Ready { after_event_id } = self else {
            return Err(CheckpointFailure::PendingReviewRequired);
        };
        if after_event_id.as_deref() == Some(&message.event_id) {
            return Ok(DeliveryDecision::Skip);
        }
        if policy.accepts(message, agent_matrix_user_id) {
            *self = Self::Pending {
                after_event_id: after_event_id.clone(),
                event_id: message.event_id.clone(),
            };
            Ok(DeliveryDecision::Deliver)
        } else {
            *after_event_id = Some(message.event_id.clone());
            Ok(DeliveryDecision::Skip)
        }
    }

    /// Confirm exactly the dispatched event after the host reports successful completion.
    ///
    /// # Errors
    /// A different event or a checkpoint without pending work cannot be acknowledged.
    pub fn complete(&mut self, completed_event_id: &str) -> Result<(), CheckpointFailure> {
        match self {
            Self::Pending { event_id, .. } if event_id == completed_event_id => {
                *self = Self::Ready {
                    after_event_id: Some(event_id.clone()),
                };
                Ok(())
            }
            _ => Err(CheckpointFailure::EventMismatch),
        }
    }
}
