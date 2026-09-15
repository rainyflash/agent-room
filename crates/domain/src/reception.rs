use crate::ids::{AgentInstanceId, DeviceId};
use uuid::Uuid;

/// Background execution is exclusive until the previous executor acknowledges
/// that its host and all outbound calls have drained. Expiry is availability
/// information, never permission for another computer to take over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceptionOwner {
    pub instance_id: AgentInstanceId,
    pub device_id: DeviceId,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceptionMode {
    Idle,
    Active,
    Draining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceptionExecution {
    pub owner: Option<ReceptionOwner>,
    pub mode: ReceptionMode,
    pub next_device: Option<DeviceId>,
}

impl Default for ReceptionExecution {
    fn default() -> Self {
        Self {
            owner: None,
            mode: ReceptionMode::Idle,
            next_device: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceptionTransition {
    Claim(ReceptionOwner),
    Drain(Option<DeviceId>),
    Release(ReceptionOwner),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceptionConflict {
    Occupied,
    WrongDevice,
    StaleExecution,
}

impl ReceptionExecution {
    /// # Errors
    /// Rejects overlapping executors, stale acknowledgements and unapproved transfers.
    pub fn apply(self, command: ReceptionTransition) -> Result<Self, ReceptionConflict> {
        match command {
            ReceptionTransition::Claim(owner) => {
                if self.mode == ReceptionMode::Active && self.owner == Some(owner) {
                    return Ok(self);
                }
                if self.mode != ReceptionMode::Idle {
                    return Err(ReceptionConflict::Occupied);
                }
                if self
                    .next_device
                    .is_some_and(|device| device != owner.device_id)
                {
                    return Err(ReceptionConflict::WrongDevice);
                }
                if self
                    .owner
                    .is_some_and(|previous| previous.run_id == owner.run_id)
                {
                    return Err(ReceptionConflict::StaleExecution);
                }
                Ok(Self {
                    owner: Some(owner),
                    mode: ReceptionMode::Active,
                    next_device: None,
                })
            }
            ReceptionTransition::Drain(next_device) => Ok(Self {
                mode: if self.mode == ReceptionMode::Idle {
                    ReceptionMode::Idle
                } else {
                    ReceptionMode::Draining
                },
                next_device,
                ..self
            }),
            ReceptionTransition::Release(owner) => {
                if self.owner != Some(owner) {
                    return Err(ReceptionConflict::StaleExecution);
                }
                Ok(Self {
                    mode: ReceptionMode::Idle,
                    ..self
                })
            }
        }
    }

    pub fn permits(self, owner: ReceptionOwner) -> bool {
        self.mode == ReceptionMode::Active && self.owner == Some(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> ReceptionOwner {
        ReceptionOwner {
            instance_id: Uuid::now_v7().into(),
            device_id: Uuid::now_v7().into(),
            run_id: Uuid::now_v7(),
        }
    }
    #[test]
    fn transfer_requires_drain_ack_and_fences_previous_run() {
        let old = owner();
        let new = owner();
        let active = ReceptionExecution::default()
            .apply(ReceptionTransition::Claim(old))
            .unwrap();
        assert_eq!(
            active.apply(ReceptionTransition::Claim(new)),
            Err(ReceptionConflict::Occupied)
        );
        let draining = active
            .apply(ReceptionTransition::Drain(Some(new.device_id)))
            .unwrap();
        assert!(!draining.permits(old));
        assert!(draining.apply(ReceptionTransition::Claim(new)).is_err());
        assert!(draining.apply(ReceptionTransition::Release(new)).is_err());
        let idle = draining.apply(ReceptionTransition::Release(old)).unwrap();
        assert!(idle.apply(ReceptionTransition::Claim(old)).is_err());
        let transferred = idle.apply(ReceptionTransition::Claim(new)).unwrap();
        assert!(transferred.permits(new));
        assert!(!transferred.permits(old));
        assert!(
            transferred
                .apply(ReceptionTransition::Release(old))
                .is_err()
        );
    }
    #[test]
    fn manual_handoff_and_restart_do_not_reuse_a_retired_execution() {
        let first = owner();
        let active = ReceptionExecution::default()
            .apply(ReceptionTransition::Claim(first))
            .unwrap();
        assert_eq!(
            active.apply(ReceptionTransition::Claim(first)).unwrap(),
            active
        );
        let stopped = active
            .apply(ReceptionTransition::Drain(None))
            .unwrap()
            .apply(ReceptionTransition::Release(first))
            .unwrap();
        assert_eq!(
            stopped.apply(ReceptionTransition::Claim(first)),
            Err(ReceptionConflict::StaleExecution)
        );
        let resumed = ReceptionOwner {
            run_id: Uuid::now_v7(),
            ..first
        };
        assert!(
            stopped
                .apply(ReceptionTransition::Claim(resumed))
                .unwrap()
                .permits(resumed)
        );
    }
}
