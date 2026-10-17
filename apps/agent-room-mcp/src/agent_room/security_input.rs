use agent_room_bridge_ipc::{IpcMatrixSecurityRequest, IpcMatrixVerificationStep};
use rmcp::schemars;
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatrixSecurityInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = 36))]
    pub session_id: String,
    pub request: MatrixSecurityRequestInput,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MatrixSecurityRequestInput {
    /// Read-only identity state: missing, ready or `recovery_required`.
    Inspect {},
    /// Only creates an encryption identity that doesn't exist yet; if an existing identity's keys are missing it asks for recovery and never resets.
    EstablishIdentity {},
    /// Lists the public device state of a participant in the same room.
    Devices {
        #[schemars(length(min = 1, max = 255))]
        room_id: String,
        #[schemars(length(min = 1, max = 255))]
        user_id: String,
    },
    /// Explicitly starts SAS verification with a participant's device; keep the returned flowId and poll it instead of calling start again.
    Start {
        #[schemars(length(min = 1, max = 255))]
        room_id: String,
        #[schemars(length(min = 1, max = 255))]
        user_id: String,
        #[schemars(length(min = 1, max = 255))]
        device_id: String,
    },
    Verification {
        #[schemars(length(min = 1, max = 255))]
        room_id: String,
        #[schemars(length(min = 1, max = 255))]
        user_id: String,
        #[schemars(length(min = 1, max = 255))]
        flow_id: String,
        step: MatrixVerificationStepInput,
    },
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MatrixVerificationStepInput {
    Accept {},
    /// Advances the protocol and reads its current state without trusting the device; poll again as needed while waiting.
    Poll {},
    /// Confirm only after the user has checked every number on a separate trusted screen; never confirm automatically by copying this tool's numbers.
    Confirm {
        decimals: [u16; 3],
        human_confirmed: bool,
    },
    Mismatch {},
    Cancel {},
}

impl From<MatrixSecurityRequestInput> for IpcMatrixSecurityRequest {
    fn from(input: MatrixSecurityRequestInput) -> Self {
        match input {
            MatrixSecurityRequestInput::Inspect {} => Self::Inspect {},
            MatrixSecurityRequestInput::EstablishIdentity {} => Self::EstablishIdentity {},
            MatrixSecurityRequestInput::Devices { room_id, user_id } => {
                Self::Devices { room_id, user_id }
            }
            MatrixSecurityRequestInput::Start {
                room_id,
                user_id,
                device_id,
            } => Self::Start {
                room_id,
                user_id,
                device_id,
            },
            MatrixSecurityRequestInput::Verification {
                room_id,
                user_id,
                flow_id,
                step,
            } => Self::Verification {
                room_id,
                user_id,
                flow_id,
                step: match step {
                    MatrixVerificationStepInput::Accept {} => IpcMatrixVerificationStep::Accept {},
                    MatrixVerificationStepInput::Poll {} => IpcMatrixVerificationStep::Poll {},
                    MatrixVerificationStepInput::Confirm {
                        decimals,
                        human_confirmed,
                    } => IpcMatrixVerificationStep::Confirm {
                        decimals,
                        human_confirmed,
                    },
                    MatrixVerificationStepInput::Mismatch {} => {
                        IpcMatrixVerificationStep::Mismatch {}
                    }
                    MatrixVerificationStepInput::Cancel {} => IpcMatrixVerificationStep::Cancel {},
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 工具输入拒绝额外字段而不接受重置或恢复密钥() {
        for action in ["inspect", "establish_identity"] {
            assert!(
                serde_json::from_value::<MatrixSecurityRequestInput>(json!({"action":action}))
                    .is_ok()
            );
            assert!(
                serde_json::from_value::<MatrixSecurityRequestInput>(
                    json!({"action":action,"reset":true})
                )
                .is_err()
            );
        }
        for action in ["accept", "poll", "mismatch", "cancel"] {
            assert!(
                serde_json::from_value::<MatrixVerificationStepInput>(json!({"action":action}))
                    .is_ok()
            );
            assert!(
                serde_json::from_value::<MatrixVerificationStepInput>(
                    json!({"action":action,"recoveryKey":"not-accepted"})
                )
                .is_err()
            );
        }
    }
}
