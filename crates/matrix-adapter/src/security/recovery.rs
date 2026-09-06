use agent_room_bridge_core::{
    matrix_recovery::{
        MatrixRecoveryCommand, MatrixRecoveryResult, MatrixRecoverySecret, MatrixRecoverySnapshot,
    },
    matrix_security::{MatrixIdentityState, MatrixSecurityFailure, MatrixSecurityResult},
};
use matrix_sdk::encryption::{recovery::RecoveryError, secret_storage::SecretStorageError};

use super::MatrixSdkSecurityGateway;

impl MatrixSdkSecurityGateway {
    async fn recovery_snapshot(&self) -> Result<MatrixRecoverySnapshot, MatrixSecurityFailure> {
        let MatrixSecurityResult::Identity {
            user_id,
            device_id,
            state,
        } = self.identity().await?
        else {
            return Err(MatrixSecurityFailure::Unavailable);
        };
        let crypto = self.client.encryption();
        let recovery_available = crypto
            .secret_storage()
            .fetch_default_key_id()
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?
            .is_some();
        Ok(MatrixRecoverySnapshot {
            user_id,
            device_id,
            identity: state,
            recovery_available,
            backup_enabled: crypto.backups().are_enabled().await,
        })
    }

    pub(super) async fn execute_recovery(
        &self,
        command: MatrixRecoveryCommand,
    ) -> Result<MatrixRecoveryResult, MatrixSecurityFailure> {
        command.validate()?;
        let _operation = self.operation.lock().await;
        let current = self.recovery_snapshot().await?;
        let crypto = self.client.encryption();
        let recovery_key = match command {
            MatrixRecoveryCommand::Inspect => {
                return Ok(MatrixRecoveryResult {
                    state: current,
                    recovery_key: None,
                });
            }
            MatrixRecoveryCommand::Enable { passphrase } => {
                if current.recovery_available {
                    return Err(MatrixSecurityFailure::RecoveryAlreadyConfigured);
                }
                self.establish().await?;
                // 禁止覆盖已有 SSSS 或重建旧身份；SDK 也拒绝覆盖无法解锁的服务器备份。
                let key = crypto
                    .recovery()
                    .enable()
                    .with_passphrase(passphrase.expose())
                    .await
                    .map_err(|failure| recovery_failure(&failure))?;
                Some(MatrixRecoverySecret::new(key))
            }
            MatrixRecoveryCommand::Restore { credential } => {
                if !current.recovery_available {
                    return Err(MatrixSecurityFailure::RecoveryUnavailable);
                }
                crypto
                    .recovery()
                    .recover(credential.expose())
                    .await
                    .map_err(|failure| recovery_failure(&failure))?;
                // 导入官方加密备份，完成前不把“已恢复身份”冒充为“已恢复历史”。
                for room in self.client.joined_rooms() {
                    if room.encryption_state().is_encrypted() {
                        crypto
                            .backups()
                            .download_room_keys_for_room(room.room_id())
                            .await
                            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                    }
                }
                None
            }
        };
        let state = self.recovery_snapshot().await?;
        if state.identity != MatrixIdentityState::Ready || !state.backup_enabled {
            return Err(MatrixSecurityFailure::RecoveryRequired);
        }
        Ok(MatrixRecoveryResult {
            state,
            recovery_key,
        })
    }
}

fn recovery_failure(failure: &RecoveryError) -> MatrixSecurityFailure {
    match failure {
        RecoveryError::BackupExistsOnServer => MatrixSecurityFailure::RecoveryAlreadyConfigured,
        RecoveryError::SecretStorage(SecretStorageError::MissingKeyInfo { .. }) => {
            MatrixSecurityFailure::RecoveryUnavailable
        }
        RecoveryError::SecretStorage(
            SecretStorageError::SecretStorageKey(_)
            | SecretStorageError::Decryption(_)
            | SecretStorageError::ImportError { .. },
        ) => MatrixSecurityFailure::RecoveryRejected,
        _ => MatrixSecurityFailure::Unavailable,
    }
}
