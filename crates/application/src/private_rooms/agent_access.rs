//! 私人房间的 Agent 口令：房主或管理员生成、更换、停用口令，查看并移出凭口令进来的 Agent；
//! Agent 凭口令以“Agent 成员”身份加入。口令不改变任何人的成员资格，只决定 Agent 能否入场。

use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    join_codes::{PrivateRoomAgentMemberStatus, PrivateRoomJoinCode},
    private_rooms::{PrivateRoomMembershipStatus, PrivateRoomPermissions},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentLobbyAccessRepository, Clock, JoinCodeAttemptPolicy, MatrixFailure, MatrixFailureKind,
        MatrixRoomId, PortFuture, PrivateRoomAgentAccessStore, PrivateRoomAgentMemberRecord,
        PrivateRoomJoinCodeRecord, PrivateRoomMatrixGateway, PrivateRoomSnapshot, PrivateRoomStore,
        SecretFactory,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAccessFailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAccessFailure {
    operation: &'static str,
    kind: AgentAccessFailureKind,
    retry_at: Option<UtcMillis>,
}

impl AgentAccessFailure {
    pub const fn new(operation: &'static str, kind: AgentAccessFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_at: None,
        }
    }

    /// 猜错太多被限流，到 `retry_at` 才能再试。
    pub const fn rate_limited(operation: &'static str, retry_at: UtcMillis) -> Self {
        Self {
            operation,
            kind: AgentAccessFailureKind::RateLimited,
            retry_at: Some(retry_at),
        }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentAccessFailureKind {
        self.kind
    }

    /// 限流时何时可以再试。
    pub const fn retry_at(self) -> Option<UtcMillis> {
        self.retry_at
    }
}

pub type AgentAccessResult<T> = Result<T, AgentAccessFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectAgentAccess {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManageJoinCode {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveAgentMember {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub agent_id: AgentId,
}

/// 本机 Agent 凭口令加入：设备、Agent 与实例必须对得上，和入场同一条规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemJoinCode {
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAccessView {
    pub join_code: Option<PrivateRoomJoinCodeRecord>,
    pub agents: Vec<PrivateRoomAgentMemberRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedJoinCode {
    pub code: PrivateRoomJoinCode,
    pub record: PrivateRoomJoinCodeRecord,
}

/// 口令兑换出的房间，Agent 随后按它入场。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemedRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub name: String,
}

pub trait PrivateRoomAgentAccessUseCases: Send + Sync {
    fn inspect(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<AgentAccessView>>;

    fn generate_code(
        &self,
        request: ManageJoinCode,
    ) -> PortFuture<'_, AgentAccessResult<GeneratedJoinCode>>;

    fn disable_code(&self, request: ManageJoinCode) -> PortFuture<'_, AgentAccessResult<()>>;

    fn remove_agent(&self, request: RemoveAgentMember) -> PortFuture<'_, AgentAccessResult<()>>;

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>>;
}

pub struct PrivateRoomAgentAccessDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub access: Arc<dyn PrivateRoomAgentAccessStore>,
    pub lobby_access: Arc<dyn AgentLobbyAccessRepository>,
    pub matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
    pub attempts: JoinCodeAttemptPolicy,
}

pub struct PrivateRoomAgentAccessService {
    rooms: Arc<dyn PrivateRoomStore>,
    access: Arc<dyn PrivateRoomAgentAccessStore>,
    lobby_access: Arc<dyn AgentLobbyAccessRepository>,
    matrix: Arc<dyn PrivateRoomMatrixGateway>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
    attempts: JoinCodeAttemptPolicy,
}

impl PrivateRoomAgentAccessService {
    pub fn new(dependencies: PrivateRoomAgentAccessDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            access: dependencies.access,
            lobby_access: dependencies.lobby_access,
            matrix: dependencies.matrix,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            attempts: dependencies.attempts,
        }
    }

    async fn inspect_internal(
        &self,
        request: InspectAgentAccess,
    ) -> AgentAccessResult<AgentAccessView> {
        const OPERATION: &str = "private_room.agent_access.inspect";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let join_code = self
            .access
            .join_code(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        let agents = self
            .access
            .agent_members(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(AgentAccessView { join_code, agents })
    }

    async fn generate_internal(
        &self,
        request: ManageJoinCode,
    ) -> AgentAccessResult<GeneratedJoinCode> {
        const OPERATION: &str = "private_room.join_code.generate";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        // 口令取一份 256 位随机值摘要的前 60 位。
        let secret = self.secrets.generate().map_err(|_| {
            AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::DependencyUnavailable)
        })?;
        let mut entropy = [0_u8; 8];
        entropy.copy_from_slice(&self.secrets.digest(secret.expose()).as_bytes()[..8]);
        let code = PrivateRoomJoinCode::from_entropy(entropy);
        let record = PrivateRoomJoinCodeRecord {
            catalog_id: request.catalog_id,
            permissions: PrivateRoomPermissions::AGENT_MEMBER,
            created_by: request.actor.principal_id,
            created_at: self.clock.now(),
        };
        self.access
            .replace_join_code(&record, &self.secrets.digest(code.normalized()))
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(GeneratedJoinCode { code, record })
    }

    async fn disable_internal(&self, request: ManageJoinCode) -> AgentAccessResult<()> {
        const OPERATION: &str = "private_room.join_code.disable";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        self.access
            .clear_join_code(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    async fn remove_internal(&self, request: RemoveAgentMember) -> AgentAccessResult<()> {
        const OPERATION: &str = "private_room.agent_member.remove";
        let snapshot = self
            .managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let member = self
            .access
            .agent_member(request.catalog_id, request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::NotFound))?;
        if member.status != PrivateRoomAgentMemberStatus::Joined {
            return Ok(());
        }
        // 先在 Matrix 上收紧再记账：记账失败时重试仍能完成，不会出现账上已移出、人还在房里。
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .set_speaking(&matrix_room, &member.matrix_user_id, false)
            .await
            .map_err(|error| matrix(OPERATION, &error))?;
        self.matrix
            .kick(&matrix_room, &member.matrix_user_id)
            .await
            .map_err(|error| matrix(OPERATION, &error))?;
        self.access
            .remove_agent(request.catalog_id, request.agent_id, self.clock.now())
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    async fn redeem_internal(&self, request: RedeemJoinCode) -> AgentAccessResult<RedeemedRoom> {
        const OPERATION: &str = "private_room.join_code.redeem";
        let now = self.clock.now();
        if request.actor.access_token_expires_at <= now {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Forbidden,
            ));
        }
        let access = self
            .lobby_access
            .find_lobby_access(request.agent_instance_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::NotFound))?;
        if !access.active
            || access.agent_id != request.agent_id
            || access.device_id != request.actor.device_id
        {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Forbidden,
            ));
        }
        let caller = format!("device:{}", request.actor.device_id);
        if let Some(retry_at) = self
            .access
            .join_code_retry_at(&caller, now, self.attempts)
            .await
            .map_err(|error| repository(OPERATION, &error))?
        {
            return Err(AgentAccessFailure::rate_limited(OPERATION, retry_at));
        }
        // 格式不对多半是抄错，不算一次猜测；格式对但不存在才计入失败次数。
        let code = PrivateRoomJoinCode::parse(&request.code).map_err(|_| {
            AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::InvalidRequest)
        })?;
        let digest = self.secrets.digest(code.normalized());
        let Some(record) = self
            .access
            .find_join_code(&digest)
            .await
            .map_err(|error| repository(OPERATION, &error))?
        else {
            self.access
                .record_join_code_failure(&caller, now, self.attempts)
                .await
                .map_err(|error| repository(OPERATION, &error))?;
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::NotFound,
            ));
        };
        let snapshot = self
            .rooms
            .find_by_catalog(record.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::NotFound))?;
        if !snapshot.room().admits_agent_member(true) {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Conflict,
            ));
        }
        // 被移出的 Agent 只能用移出之后生成的新口令再进来。
        let previous = self
            .access
            .agent_member(record.catalog_id, request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        if previous.is_some_and(|member| {
            member.status == PrivateRoomAgentMemberStatus::Removed
                && record.created_at <= member.status_changed_at
        }) {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Forbidden,
            ));
        }
        self.access
            .admit_agent(record.catalog_id, request.agent_id, record.permissions, now)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(RedeemedRoom {
            catalog_id: record.catalog_id,
            matrix_room_id: snapshot.instance().matrix_room_id().clone(),
            name: snapshot.catalog().name().to_owned(),
        })
    }

    /// 操作者看得见这个房间（受邀或已加入），并且有管理 Agent 口令的权限。
    async fn managed_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> AgentAccessResult<PrivateRoomSnapshot> {
        if self.clock.now() >= actor.expires_at {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::Forbidden,
            ));
        }
        let snapshot = self
            .rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| AgentAccessFailure::new(operation, AgentAccessFailureKind::NotFound))?;
        let visible = snapshot
            .room()
            .member(actor.principal_id)
            .is_some_and(|member| {
                matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
            });
        if !visible {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::NotFound,
            ));
        }
        snapshot
            .room()
            .authorize_agent_access(actor.principal_id)
            .map_err(|error| domain(operation, &error))?;
        Ok(snapshot)
    }
}

impl PrivateRoomAgentAccessUseCases for PrivateRoomAgentAccessService {
    fn inspect(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<AgentAccessView>> {
        Box::pin(self.inspect_internal(request))
    }

    fn generate_code(
        &self,
        request: ManageJoinCode,
    ) -> PortFuture<'_, AgentAccessResult<GeneratedJoinCode>> {
        Box::pin(self.generate_internal(request))
    }

    fn disable_code(&self, request: ManageJoinCode) -> PortFuture<'_, AgentAccessResult<()>> {
        Box::pin(self.disable_internal(request))
    }

    fn remove_agent(&self, request: RemoveAgentMember) -> PortFuture<'_, AgentAccessResult<()>> {
        Box::pin(self.remove_internal(request))
    }

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        Box::pin(self.redeem_internal(request))
    }
}

fn matrix_room_id(
    snapshot: &PrivateRoomSnapshot,
    operation: &'static str,
) -> AgentAccessResult<MatrixRoomId> {
    MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
        .map_err(|_| AgentAccessFailure::new(operation, AgentAccessFailureKind::Internal))
}

const fn domain(operation: &'static str, error: &DomainError) -> AgentAccessFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => AgentAccessFailureKind::Forbidden,
        DomainError::Validation { .. } => AgentAccessFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => AgentAccessFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => AgentAccessFailureKind::Internal,
    };
    AgentAccessFailure::new(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> AgentAccessFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => AgentAccessFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => AgentAccessFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AgentAccessFailureKind::NotFound,
        RepositoryErrorKind::Conflict => AgentAccessFailureKind::Conflict,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            AgentAccessFailureKind::Internal
        }
    };
    AgentAccessFailure::new(operation, kind)
}

const fn matrix(operation: &'static str, error: &MatrixFailure) -> AgentAccessFailure {
    let kind = match error.kind() {
        MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden => AgentAccessFailureKind::Forbidden,
        MatrixFailureKind::NotFound => AgentAccessFailureKind::NotFound,
        MatrixFailureKind::Conflict => AgentAccessFailureKind::Conflict,
        MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::RateLimited => AgentAccessFailureKind::DependencyUnavailable,
        MatrixFailureKind::UnknownCommit => AgentAccessFailureKind::UnknownCommit,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => AgentAccessFailureKind::Internal,
    };
    AgentAccessFailure::new(operation, kind)
}
