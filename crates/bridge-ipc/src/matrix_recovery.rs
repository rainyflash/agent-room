pub use agent_room_bridge_core::matrix_recovery::MatrixRecoverySecret;
use agent_room_bridge_core::matrix_recovery::{MatrixRecoveryCommand, MatrixRecoveryResult};
use serde::{Deserialize, Serialize};

use crate::{
    IpcHostSessionState, IpcMatrixIdentityState, IpcMethodValidationFailure, tools::failure,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcMatrixRecoveryRequest {
    Inspect {},
    Enable { passphrase: MatrixRecoverySecret },
    Restore { credential: MatrixRecoverySecret },
}

impl IpcMatrixRecoveryRequest {
    /// # Errors
    /// 恢复口令不满足边界约束时拒绝，不回显输入。
    pub fn command(&self) -> Result<MatrixRecoveryCommand, IpcMethodValidationFailure> {
        let command = match self {
            Self::Inspect {} => MatrixRecoveryCommand::Inspect,
            Self::Enable { passphrase } => MatrixRecoveryCommand::Enable {
                passphrase: passphrase.clone(),
            },
            Self::Restore { credential } => MatrixRecoveryCommand::Restore {
                credential: credential.clone(),
            },
        };
        command
            .validate()
            .map_err(|_| failure("bridge.security.invalid_request"))?;
        Ok(command)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcAgentRecoverySession {
    pub session_id: String,
    pub display_name: String,
    pub state: IpcHostSessionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMatrixRecoveryState {
    pub user_id: String,
    pub device_id: String,
    pub identity: IpcMatrixIdentityState,
    pub recovery_available: bool,
    pub backup_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcMatrixRecoveryResult {
    pub state: IpcMatrixRecoveryState,
    pub recovery_key: Option<MatrixRecoverySecret>,
}

impl From<MatrixRecoveryResult> for IpcMatrixRecoveryResult {
    fn from(result: MatrixRecoveryResult) -> Self {
        Self {
            state: IpcMatrixRecoveryState {
                user_id: result.state.user_id,
                device_id: result.state.device_id,
                identity: result.state.identity.into(),
                recovery_available: result.state.recovery_available,
                backup_enabled: result.state.backup_enabled,
            },
            recovery_key: result.recovery_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IpcMethod;
    #[test]
    fn 恢复入口闭合校验且调试输出不含恢复口令() {
        let request: IpcMatrixRecoveryRequest = serde_json::from_value(
            serde_json::json!({"action":"restore","credential":"test-recovery-credential"}),
        )
        .unwrap();
        let method = IpcMethod::MatrixRecovery(request);
        assert!(method.validate().is_ok());
        assert!(!format!("{method:?}").contains("test-recovery-credential"));
        for value in [
            serde_json::json!({"action":"inspect","reset":true}),
            serde_json::json!({"action":"enable","passphrase":"weak"}),
            serde_json::json!({"action":"restore","credential":""}),
        ] {
            assert!(
                serde_json::from_value::<IpcMatrixRecoveryRequest>(value)
                    .map_or(true, |request| request.command().is_err())
            );
        }
    }
}
