use crate::{IpcMethodValidationFailure, tools::failure};
use agent_room_bridge_core::matrix_security::{
    MatrixDeviceId, MatrixIdentityState, MatrixRoomId, MatrixSecurityCommand, MatrixSecurityResult,
    MatrixUserId, MatrixVerificationAction, MatrixVerificationStage, MatrixVerificationTarget,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IpcMatrixSecurityRequest {
    Inspect {},
    EstablishIdentity {},
    Devices {
        room_id: String,
        user_id: String,
    },
    Start {
        room_id: String,
        user_id: String,
        device_id: String,
    },
    Verification {
        room_id: String,
        user_id: String,
        flow_id: String,
        step: IpcMatrixVerificationStep,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IpcMatrixVerificationStep {
    Accept {},
    Poll {},
    Confirm {
        decimals: [u16; 3],
        human_confirmed: bool,
    },
    Mismatch {},
    Cancel {},
}

impl IpcMatrixSecurityRequest {
    /// 校验传输字段后进入原生 Matrix 安全端口。
    ///
    /// # Errors
    /// 标识越界、非法或确认字段无效时拒绝请求。
    pub fn command(&self) -> Result<MatrixSecurityCommand, IpcMethodValidationFailure> {
        let invalid = || failure("bridge.security.invalid_request");
        let room = |value: &str| MatrixRoomId::new(value.to_owned()).map_err(|_| invalid());
        let user = |value: &str| MatrixUserId::new(value.to_owned()).map_err(|_| invalid());
        Ok(match self {
            Self::Inspect {} => MatrixSecurityCommand::Inspect,
            Self::EstablishIdentity {} => MatrixSecurityCommand::EstablishIdentity,
            Self::Devices { room_id, user_id } => MatrixSecurityCommand::Devices {
                room_id: room(room_id)?,
                user_id: user(user_id)?,
            },
            Self::Start {
                room_id,
                user_id,
                device_id,
            } => MatrixSecurityCommand::Start {
                room_id: room(room_id)?,
                user_id: user(user_id)?,
                device_id: MatrixDeviceId::new(device_id.clone()).map_err(|_| invalid())?,
            },
            Self::Verification {
                room_id,
                user_id,
                flow_id,
                step,
            } => {
                if flow_id.is_empty()
                    || flow_id.len() > 255
                    || flow_id.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
                let action = match step {
                    IpcMatrixVerificationStep::Accept {} => MatrixVerificationAction::Accept,
                    IpcMatrixVerificationStep::Poll {} => MatrixVerificationAction::Poll,
                    IpcMatrixVerificationStep::Mismatch {} => MatrixVerificationAction::Mismatch,
                    IpcMatrixVerificationStep::Cancel {} => MatrixVerificationAction::Cancel,
                    IpcMatrixVerificationStep::Confirm {
                        decimals,
                        human_confirmed,
                    } => {
                        if decimals.iter().any(|code| !(1000..=9191).contains(code)) {
                            return Err(invalid());
                        }
                        MatrixVerificationAction::Confirm {
                            decimals: *decimals,
                            human_confirmed: *human_confirmed,
                        }
                    }
                };
                MatrixSecurityCommand::Verification {
                    target: MatrixVerificationTarget {
                        room_id: room(room_id)?,
                        user_id: user(user_id)?,
                        flow_id: flow_id.clone(),
                    },
                    action,
                }
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IpcMatrixSecurityResult {
    Identity {
        user_id: String,
        device_id: String,
        state: IpcMatrixIdentityState,
    },
    Devices {
        devices: Vec<IpcMatrixSecurityDevice>,
    },
    Verification {
        room_id: String,
        user_id: String,
        device_id: Option<String>,
        flow_id: String,
        stage: IpcMatrixVerificationStage,
        decimals: Option<[u16; 3]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMatrixIdentityState {
    Missing,
    RecoveryRequired,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcMatrixVerificationStage {
    Requested,
    Waiting,
    Comparing,
    Confirming,
    Verified,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMatrixSecurityDevice {
    pub user_id: String,
    pub device_id: String,
    pub owner_signed: bool,
    pub verified: bool,
}

impl From<MatrixSecurityResult> for IpcMatrixSecurityResult {
    fn from(result: MatrixSecurityResult) -> Self {
        match result {
            MatrixSecurityResult::Identity {
                user_id,
                device_id,
                state,
            } => Self::Identity {
                user_id,
                device_id,
                state: match state {
                    MatrixIdentityState::Missing => IpcMatrixIdentityState::Missing,
                    MatrixIdentityState::RecoveryRequired => {
                        IpcMatrixIdentityState::RecoveryRequired
                    }
                    MatrixIdentityState::Ready => IpcMatrixIdentityState::Ready,
                },
            },
            MatrixSecurityResult::Devices(devices) => Self::Devices {
                devices: devices
                    .into_iter()
                    .map(|device| IpcMatrixSecurityDevice {
                        user_id: device.user_id,
                        device_id: device.device_id,
                        owner_signed: device.owner_signed,
                        verified: device.verified,
                    })
                    .collect(),
            },
            MatrixSecurityResult::Verification(snapshot) => Self::Verification {
                room_id: snapshot.target.room_id.as_str().to_owned(),
                user_id: snapshot.target.user_id.as_str().to_owned(),
                device_id: snapshot.device_id,
                flow_id: snapshot.target.flow_id,
                decimals: snapshot.decimals,
                stage: match snapshot.stage {
                    MatrixVerificationStage::Requested => IpcMatrixVerificationStage::Requested,
                    MatrixVerificationStage::Waiting => IpcMatrixVerificationStage::Waiting,
                    MatrixVerificationStage::Comparing => IpcMatrixVerificationStage::Comparing,
                    MatrixVerificationStage::Confirming => IpcMatrixVerificationStage::Confirming,
                    MatrixVerificationStage::Verified => IpcMatrixVerificationStage::Verified,
                    MatrixVerificationStage::Cancelled => IpcMatrixVerificationStage::Cancelled,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 安全请求拒绝无效标识和过长事务并保留人类确认位() {
        assert!(
            IpcMatrixSecurityRequest::Start {
                room_id: "not-a-room".into(),
                user_id: "@a:test".into(),
                device_id: "DEVICE".into()
            }
            .command()
            .is_err()
        );
        let mut request = IpcMatrixSecurityRequest::Verification {
            room_id: "!room:test".into(),
            user_id: "@a:test".into(),
            flow_id: "txn".into(),
            step: IpcMatrixVerificationStep::Confirm {
                decimals: [1234, 5678, 9012],
                human_confirmed: false,
            },
        };
        assert!(matches!(
            request.command(),
            Ok(MatrixSecurityCommand::Verification {
                action: MatrixVerificationAction::Confirm {
                    human_confirmed: false,
                    ..
                },
                ..
            })
        ));
        if let IpcMatrixSecurityRequest::Verification { flow_id, .. } = &mut request {
            *flow_id = "x".repeat(256);
        }
        assert!(request.command().is_err());
    }
    #[test]
    fn 无参数动作也拒绝隐藏的密钥或重置字段() {
        for action in ["inspect", "establish_identity"] {
            assert!(
                serde_json::from_value::<IpcMatrixSecurityRequest>(
                    serde_json::json!({"action":action})
                )
                .is_ok()
            );
            assert!(
                serde_json::from_value::<IpcMatrixSecurityRequest>(
                    serde_json::json!({"action":action,"reset":true})
                )
                .is_err()
            );
        }
        for action in ["accept", "poll", "mismatch", "cancel"] {
            assert!(
                serde_json::from_value::<IpcMatrixVerificationStep>(
                    serde_json::json!({"action":action})
                )
                .is_ok()
            );
            assert!(
                serde_json::from_value::<IpcMatrixVerificationStep>(
                    serde_json::json!({"action":action,"recoveryKey":"not-accepted"})
                )
                .is_err()
            );
        }
    }
}
