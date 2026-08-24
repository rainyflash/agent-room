use agent_room_domain::{
    agents::AgentRole,
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId},
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
