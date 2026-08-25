use agent_room_domain::{ids::AgentInstanceId, time::UtcMillis};

use crate::ports::{
    AgentInstanceMatrixCleanupStore, MatrixAgentDeviceSessionRevoker,
    MatrixAgentDeviceSessionTarget, MatrixDeviceId, MatrixFailureKind, MatrixUserId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatrixDeviceCleanupFailure {
    InvalidStoredIdentity,
    Matrix(MatrixFailureKind),
    StatePersistenceUnavailable,
}

pub(crate) async fn revoke_agent_matrix_device(
    matrix: &dyn MatrixAgentDeviceSessionRevoker,
    cleanup: &dyn AgentInstanceMatrixCleanupStore,
    instance_id: AgentInstanceId,
    matrix_user_id: &str,
    matrix_device_id: &str,
    revoked_at: UtcMillis,
) -> Result<(), MatrixDeviceCleanupFailure> {
    let user_id = MatrixUserId::new(matrix_user_id.to_owned())
        .map_err(|_| MatrixDeviceCleanupFailure::InvalidStoredIdentity)?;
    let device_id = MatrixDeviceId::new(matrix_device_id.to_owned())
        .map_err(|_| MatrixDeviceCleanupFailure::InvalidStoredIdentity)?;
    let target = MatrixAgentDeviceSessionTarget::new(user_id, device_id);
    matrix
        .revoke_device_session(&target)
        .await
        .map_err(|failure| MatrixDeviceCleanupFailure::Matrix(failure.kind()))?;
    cleanup
        .mark_matrix_device_revoked(instance_id, revoked_at)
        .await
        .map_err(|_| MatrixDeviceCleanupFailure::StatePersistenceUnavailable)
}
