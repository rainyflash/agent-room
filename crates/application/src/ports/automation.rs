use agent_room_domain::{
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, DeviceId, MessageSubmissionId, PrincipalId,
        RoomCatalogId,
    },
    policy::{
        AutomationGrant, AutomationGrantAttempt, AutomationGrantDenial, AutomationUsageSnapshot,
    },
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::{MatrixRoomId, MatrixUserId, PortFuture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationGrantRecord {
    pub grant: AutomationGrant,
    pub usage: AutomationUsageSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationGrantRevocationOutcome {
    Revoked(AutomationGrantRecord),
    AlreadyRevoked(AutomationGrantRecord),
    AlreadyInactive(AutomationGrantRecord),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationConsumptionRequest {
    pub grant_id: AutomationGrantId,
    pub submission_id: MessageSubmissionId,
    pub matrix_room_id: MatrixRoomId,
    pub attempt: AutomationGrantAttempt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationConsumptionOutcome {
    Consumed {
        record: AutomationGrantRecord,
        reused: bool,
    },
    Denied(AutomationGrantDenial),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationDecisionRecord {
    pub grant_id: AutomationGrantId,
    pub submission_id: MessageSubmissionId,
    pub principal_id: PrincipalId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub room_catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomId,
    pub decision_code: &'static str,
    pub decided_at: UtcMillis,
}

/// 自动发言授权及其消费账本的权威事务边界。
///
/// `consume` 必须在同一数据库事务中锁定授权、重算频率与总量、写入幂等消费记录；
/// 不允许先读后写造成并发超发。
pub trait AutomationGrantRepository: Send + Sync {
    fn create<'a>(
        &'a self,
        grant: &'a AutomationGrant,
    ) -> PortFuture<'a, RepositoryResult<AutomationGrantRecord>>;

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<AutomationGrantRecord>>>;

    fn find(
        &self,
        grant_id: AutomationGrantId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AutomationGrantRecord>>>;

    fn revoke(
        &self,
        principal_id: PrincipalId,
        grant_id: AutomationGrantId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<AutomationGrantRevocationOutcome>>;

    fn consume<'a>(
        &'a self,
        request: &'a AutomationConsumptionRequest,
    ) -> PortFuture<'a, RepositoryResult<AutomationConsumptionOutcome>>;

    fn record_decision<'a>(
        &'a self,
        record: &'a AutomationDecisionRecord,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationScopeAuthorityRequest {
    pub principal_id: PrincipalId,
    pub agent_id: AgentId,
    pub agent_instance_id: Option<AgentInstanceId>,
    pub room_catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationSendAuthorityRequest {
    pub principal_id: PrincipalId,
    pub device_id: DeviceId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub room_catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationSendAuthority {
    pub agent_matrix_user_id: MatrixUserId,
    pub contains_unknown_recipients: bool,
}

/// 每次创建授权或发送前重新读取产品权限与实例归属；缓存实现不得实现此端口。
pub trait AutomationScopeAuthority: Send + Sync {
    fn may_create<'a>(
        &'a self,
        request: &'a AutomationScopeAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    fn inspect_send<'a>(
        &'a self,
        request: &'a AutomationSendAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<Option<AutomationSendAuthority>>>;
}
