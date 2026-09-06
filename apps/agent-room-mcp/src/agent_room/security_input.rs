use agent_room_bridge_ipc::{IpcMatrixSecurityRequest, IpcMatrixVerificationStep};
use rmcp::schemars;
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatrixSecurityInput {
    /// 本任务 `open_session` 返回的 `sessionId`。
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
    /// 只读身份状态：missing、ready 或 `recovery_required`。
    Inspect {},
    /// 只建立尚不存在的加密身份；已有身份的密钥缺失时明确要求恢复，不执行重置。
    EstablishIdentity {},
    /// 查询同一房间内参与者的公开设备状态。
    Devices {
        #[schemars(length(min = 1, max = 255))]
        room_id: String,
        #[schemars(length(min = 1, max = 255))]
        user_id: String,
    },
    /// 显式向指定参与者设备发起 SAS 验证；保存返回的 flowId，不能重复 start 代替查询。
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
    /// 推进协议协商并读取当前状态，不确认设备信任；等待时按需再次查询。
    Poll {},
    /// 只有用户在独立可信界面核对完整数字后才能确认，禁止照抄本工具数字自动确认。
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
