use agent_room_domain::{
    agents::{AgentInstanceStatus, AgentRole},
    devices::DevicePlatform,
    handoff::{HandoffFailureCode, TargetedHandoff},
    ids::{AgentId, AgentInstanceId, ContentId, DeviceId, HandoffId, PrincipalId},
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffInstanceAccessRecord {
    pub instance_id: AgentInstanceId,
    pub agent_id: AgentId,
    pub device_id: DeviceId,
    pub matrix_user_id: String,
    pub matrix_device_id: String,
    pub role: Option<AgentRole>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffAuthorizationSnapshot {
    pub requester: HandoffInstanceAccessRecord,
    pub target: HandoffInstanceAccessRecord,
}

/// 为一次性交接提供同一数据库快照中的实例和成员权限事实。
///
/// 仓库只返回事实，不决定业务授权；允许或拒绝仍由应用用例负责。
pub trait HandoffAccessRepository: Send + Sync {
    fn inspect_authorization(
        &self,
        principal_id: PrincipalId,
        requester_instance_id: AgentInstanceId,
        target_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffAuthorizationSnapshot>>>;

    fn find_instance_access(
        &self,
        principal_id: PrincipalId,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffInstanceAccessRecord>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedHandoffTargetRecord {
    pub instance_id: AgentInstanceId,
    pub agent_id: AgentId,
    pub agent_display_name: String,
    pub agent_avatar_content_id: Option<ContentId>,
    pub device_id: DeviceId,
    pub device_label: String,
    pub device_platform: DevicePlatform,
    pub instance_status: AgentInstanceStatus,
    pub lease_expires_at: Option<UtcMillis>,
    pub last_seen_at: Option<UtcMillis>,
    pub adapter_type: String,
    pub capability_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetedHandoffRequestFingerprint([u8; 32]);

impl TargetedHandoffRequestFingerprint {
    pub const fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTargetedHandoff<'a> {
    pub handoff: &'a TargetedHandoff,
    pub request_fingerprint: TargetedHandoffRequestFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueTargetedHandoffOutcome {
    Created(TargetedHandoff),
    Existing(TargetedHandoff),
}

impl QueueTargetedHandoffOutcome {
    pub const fn handoff(&self) -> &TargetedHandoff {
        match self {
            Self::Created(handoff) | Self::Existing(handoff) => handoff,
        }
    }

    pub const fn was_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimTargetedHandoff {
    pub principal_id: PrincipalId,
    pub device_id: DeviceId,
    pub target_instance_id: AgentInstanceId,
    pub claimed_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedHandoffReceiptOutcome {
    Consumed,
    Declined(HandoffFailureCode),
    Failed(HandoffFailureCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTargetedHandoffReceipt {
    pub principal_id: PrincipalId,
    pub device_id: DeviceId,
    pub target_instance_id: AgentInstanceId,
    pub handoff_id: HandoffId,
    pub outcome: TargetedHandoffReceiptOutcome,
    pub recorded_at: UtcMillis,
}

/// 云端定向交接的唯一事实源。
///
/// 实现必须在数据库事务中复核目标实例、设备、操作权限和能力标记；应用层预检查只负责
/// 生成清晰错误，不能作为防止 TOCTOU 的最终安全边界。
pub trait TargetedHandoffRepository: Send + Sync {
    fn list_targets(
        &self,
        principal_id: PrincipalId,
        observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<TargetedHandoffTargetRecord>>>;

    fn queue<'a>(
        &'a self,
        request: QueueTargetedHandoff<'a>,
    ) -> PortFuture<'a, RepositoryResult<QueueTargetedHandoffOutcome>>;

    fn find_for_principal(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>>;

    fn revoke(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>>;

    fn claim_next(
        &self,
        request: ClaimTargetedHandoff,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>>;

    fn record_receipt(
        &self,
        request: RecordTargetedHandoffReceipt,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>>;
}
