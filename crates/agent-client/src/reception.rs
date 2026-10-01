//! Ordered delivery decisions, independent of process execution and storage.
use std::{collections::HashSet, time::Duration};

use agent_room_bridge_ipc::{
    IpcActorSummary, IpcMessagePreviewSummary,
    wake::{MAX_DIGEST, WaitOptions, WaitRules, WakeContext, wakes},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceptionPolicy {
    pub room_id: String,
    pub allowed_principal_id: String,
    /// 没叫醒它的消息最多攒几分钟就让它看一眼（1 到 1440）；不设就不看。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_minutes: Option<u64>,
}

impl ReceptionPolicy {
    /// 后台回复叫醒它的消息（`specs/agent-reading/waiting.md`「后台回复」）：
    /// - 主人说的、跟它有关的话：点了它或回复它，或者没点别人、也没回复别人；
    /// - 私人房间里别人点名或回复它；
    /// - 别的 Agent 叫不醒它，免得两个 Agent 的后台互相叫醒停不下来。
    pub fn wakes(&self, message: &IpcMessagePreviewSummary, private_room: bool) -> bool {
        if message.room_id != self.room_id || message.from_me {
            return false;
        }
        let IpcActorSummary::Human { principal_id, .. } = &message.actor else {
            return false;
        };
        if *principal_id == self.allowed_principal_id {
            let direct_rooms = HashSet::new();
            return wakes(
                message,
                &WaitOptions::default(),
                WakeContext {
                    owner: None,
                    direct_rooms: &direct_rooms,
                },
            );
        }
        private_room && message.mentions_me
    }

    /// 后台等消息的规则：防抖照默认，几条连着的合成一次；定时看一眼按主人的设置。
    pub fn wait_rules(&self) -> WaitRules {
        WaitRules {
            options: WaitOptions {
                room_id: Some(self.room_id.clone()),
                digest: self
                    .digest_minutes
                    .map(|minutes| Duration::from_mins(minutes.min(1_440)).min(MAX_DIGEST)),
                ..WaitOptions::default()
            },
            wait_for_mentioned: false,
        }
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

    /// 一批要交给宿主了：整批记成待定，锚在交出去的最后一条上。宿主跑之前就要存下来。
    ///
    /// # Errors
    /// An uncertain previous delivery must be resolved explicitly before reading more messages.
    pub fn begin(&mut self, anchor_event_id: &str) -> Result<(), CheckpointFailure> {
        let Self::Ready { after_event_id } = self else {
            return Err(CheckpointFailure::PendingReviewRequired);
        };
        *self = Self::Pending {
            after_event_id: after_event_id.clone(),
            event_id: anchor_event_id.to_owned(),
        };
        Ok(())
    }

    /// Confirm exactly the dispatched batch after the host reports successful completion.
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
