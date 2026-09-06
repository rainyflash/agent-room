use agent_room_application::ports::PortFuture;
pub use agent_room_application::ports::{MatrixDeviceId, MatrixRoomId, MatrixUserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixSecurityCommand {
    Inspect,
    EstablishIdentity,
    Devices {
        room_id: MatrixRoomId,
        user_id: MatrixUserId,
    },
    Start {
        room_id: MatrixRoomId,
        user_id: MatrixUserId,
        device_id: MatrixDeviceId,
    },
    Verification {
        target: MatrixVerificationTarget,
        action: MatrixVerificationAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixVerificationTarget {
    pub room_id: MatrixRoomId,
    pub user_id: MatrixUserId,
    pub flow_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixVerificationAction {
    Accept,
    Poll,
    Confirm {
        decimals: [u16; 3],
        human_confirmed: bool,
    },
    Mismatch,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixIdentityState {
    Missing,
    RecoveryRequired,
    Ready,
}

/// 已有公开身份但私钥不可用时必须恢复或验证，不能用一次初始化覆盖它。
pub const fn identity_state(
    published: bool,
    private_keys_complete: bool,
    device_signed: bool,
) -> MatrixIdentityState {
    if !published {
        MatrixIdentityState::Missing
    } else if private_keys_complete && device_signed {
        MatrixIdentityState::Ready
    } else {
        MatrixIdentityState::RecoveryRequired
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSecurityDevice {
    pub user_id: String,
    pub device_id: String,
    pub owner_signed: bool,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixVerificationStage {
    Requested,
    Waiting,
    Comparing,
    Confirming,
    Verified,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixVerificationSnapshot {
    pub target: MatrixVerificationTarget,
    pub device_id: Option<String>,
    pub stage: MatrixVerificationStage,
    pub decimals: Option<[u16; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixSecurityResult {
    Identity {
        user_id: String,
        device_id: String,
        state: MatrixIdentityState,
    },
    Devices(Vec<MatrixSecurityDevice>),
    Verification(MatrixVerificationSnapshot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixSecurityFailure {
    RecoveryAlreadyConfigured,
    RecoveryUnavailable,
    RecoveryRejected,
    IdentityNotReady,
    PeerVerificationRequired,
    Unavailable,
    InvalidRequest,
    NotJoined,
    RecoveryRequired,
    VerificationUnavailable,
    ConfirmationRequired,
    SasMismatch,
}

pub trait MatrixSecurityGateway: Send + Sync {
    fn recover(
        &self,
        command: crate::matrix_recovery::MatrixRecoveryCommand,
    ) -> PortFuture<'_, Result<crate::matrix_recovery::MatrixRecoveryResult, MatrixSecurityFailure>>;
    fn ensure_room_ready<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<(), MatrixSecurityFailure>>;
    fn execute(
        &self,
        command: MatrixSecurityCommand,
    ) -> PortFuture<'_, Result<MatrixSecurityResult, MatrixSecurityFailure>>;
}

/// 公开安全码必须来自对端显示并由人类核对；收到远端文本不构成信任授权。
///
/// # Errors
/// 未获确认、协议尚未展示数字或数字不一致时返回对应拒绝原因。
pub fn validate_sas_confirmation(
    displayed: Option<[u16; 3]>,
    supplied: [u16; 3],
    human_confirmed: bool,
) -> Result<(), MatrixSecurityFailure> {
    if !human_confirmed {
        return Err(MatrixSecurityFailure::ConfirmationRequired);
    }
    let displayed = displayed.ok_or(MatrixSecurityFailure::VerificationUnavailable)?;
    if displayed != supplied {
        return Err(MatrixSecurityFailure::SasMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 已有身份丢失私钥或设备签名时要求恢复() {
        assert_eq!(
            identity_state(false, false, false),
            MatrixIdentityState::Missing
        );
        assert_eq!(identity_state(true, true, true), MatrixIdentityState::Ready);
        assert_eq!(
            identity_state(true, false, true),
            MatrixIdentityState::RecoveryRequired
        );
        assert_eq!(
            identity_state(true, true, false),
            MatrixIdentityState::RecoveryRequired
        );
    }

    #[test]
    fn 安全码确认要求明确授权和完整一致的已展示数字() {
        let codes = [1234, 5678, 9012];
        assert_eq!(validate_sas_confirmation(Some(codes), codes, true), Ok(()));
        assert_eq!(
            validate_sas_confirmation(Some(codes), codes, false),
            Err(MatrixSecurityFailure::ConfirmationRequired)
        );
        assert_eq!(
            validate_sas_confirmation(None, codes, true),
            Err(MatrixSecurityFailure::VerificationUnavailable)
        );
        assert_eq!(
            validate_sas_confirmation(Some(codes), [1234, 5678, 9013], true),
            Err(MatrixSecurityFailure::SasMismatch)
        );
    }
}
