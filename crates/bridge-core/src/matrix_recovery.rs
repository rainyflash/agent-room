use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::matrix_security::{MatrixIdentityState, MatrixSecurityFailure};

/// 只在受信桌面与原生加密端口之间传递；调试输出脱敏，释放时清零。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MatrixRecoverySecret(String);

impl MatrixRecoverySecret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for MatrixRecoverySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl Drop for MatrixRecoverySecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixRecoveryCommand {
    Inspect,
    Enable { passphrase: MatrixRecoverySecret },
    Restore { credential: MatrixRecoverySecret },
}

impl MatrixRecoveryCommand {
    /// # Errors
    /// 拒绝空、超长或含控制字符的恢复凭据，设置时要求至少 12 个字符。
    pub fn validate(&self) -> Result<(), MatrixSecurityFailure> {
        let (secret, minimum) = match self {
            Self::Inspect => return Ok(()),
            Self::Enable { passphrase } => (passphrase.expose(), 12),
            Self::Restore { credential } => (credential.expose(), 1),
        };
        if secret.trim().chars().count() < minimum
            || secret.len() > 1024
            || secret.chars().any(char::is_control)
        {
            return Err(MatrixSecurityFailure::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRecoverySnapshot {
    pub user_id: String,
    pub device_id: String,
    pub identity: MatrixIdentityState,
    pub recovery_available: bool,
    pub backup_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRecoveryResult {
    pub state: MatrixRecoverySnapshot,
    pub recovery_key: Option<MatrixRecoverySecret>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 恢复凭据有长度边界且不进入调试输出() {
        let secret = MatrixRecoverySecret::new("sensitive-test-passphrase".into());
        let command = MatrixRecoveryCommand::Enable { passphrase: secret };
        assert_eq!(command.validate(), Ok(()));
        assert!(!format!("{command:?}").contains("sensitive-test-passphrase"));
        for value in [
            "short".to_owned(),
            "x".repeat(1025),
            "long-but\n-invalid".to_owned(),
        ] {
            assert!(
                MatrixRecoveryCommand::Enable {
                    passphrase: MatrixRecoverySecret::new(value)
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            MatrixRecoveryCommand::Restore {
                credential: MatrixRecoverySecret::new(" ".into())
            }
            .validate()
            .is_err()
        );
    }
}
