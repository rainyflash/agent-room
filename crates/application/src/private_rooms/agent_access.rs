//! 私人房间的 Agent 口令与敲门：房主或管理员生成、更换、停用口令，放行或拒绝敲门的 Agent，查看并
//! 移出进来的 Agent；Agent 凭口令或经放行以“Agent 成员”身份加入。口令和敲门不改变任何人的成员资格，
//! 只决定 Agent 能否入场（`specs/network-agents/knock.md`）。

use std::{fmt::Write as _, sync::Arc};

use agent_room_domain::{
    DomainError,
    ids::{AgentId, RoomCatalogId},
    join_codes::{
        PrivateRoomAgentJoinedVia, PrivateRoomAgentKnockStatus, PrivateRoomAgentMemberStatus,
        PrivateRoomJoinCode,
    },
    private_rooms::{PrivateRoomMembershipStatus, PrivateRoomPermissions},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentMembershipRepository, Clock, JoinCodeAttemptPolicy, MatrixFailure, MatrixFailureKind,
        MatrixRoomId, PortFuture, PrivateRoomAgentAccessStore, PrivateRoomAgentKnockOutcome,
        PrivateRoomAgentKnockRecord, PrivateRoomAgentMemberRecord, PrivateRoomJoinCodeRecord,
        PrivateRoomMatrixGateway, PrivateRoomSnapshot, PrivateRoomStore, SecretFactory,
    },
};

/// 在等的敲门有效这么久（毫秒），再敲一次就从那时重新算。
pub const AGENT_KNOCK_TTL_MILLIS: i64 = 60 * 60 * 1_000;
/// 一个房间同时在等的敲门最多这么多个，满了要等最早那个作废。
pub const MAX_WAITING_AGENT_KNOCKS: u32 = 5;
/// Agent 查看自己敲过的门时，看这么久以内的（毫秒）。
const AGENT_KNOCK_HISTORY_MILLIS: i64 = 24 * 60 * 60 * 1_000;

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

/// 谁在试口令：猜错按它计数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinCodeCaller {
    /// 本机设备：按设备计数，令牌过期就不能试。
    Device(AuthenticatedDevice),
    /// 只凭网络接入的 Agent：按来源（来源地址按天加盐后的摘要）计数，与本机设备分开计。
    NetworkSource([u8; 32]),
}

/// 查看口令对应的房间，不让任何 Agent 加入。接入方据此选定在这个房间里用哪个人物，再兑换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveJoinCode {
    pub caller: JoinCodeCaller,
    pub code: String,
}

/// 让一个 Agent 凭口令加入：调用的设备所属账号必须是这个 Agent 的主人或操作者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemJoinCode {
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub code: String,
}

/// Agent 拿房间号敲门：调用的设备所属账号必须能为这个 Agent 注册实例（网络 Agent 用它自己的网络设备）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnockOnRoom {
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub catalog_id: RoomCatalogId,
}

/// 敲门的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnockResult {
    /// 已经是这个房间的 Agent 成员（凭口令进来过，或放行时没进成）：直接进，不用敲门。
    Member(RedeemedRoom),
    /// 记下了，等管理者放行。
    Waiting(PrivateRoomAgentKnockRecord),
    /// 以前没让进：不再打扰管理者。
    Declined(PrivateRoomAgentKnockRecord),
}

/// 管理者对一次敲门的回答：放行或不让进。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerKnock {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub agent_id: AgentId,
}

/// 放行了：Agent 已记为这个房间的 Agent 成员，敲门还在等。接着由网关替它进房间，进去了再
/// `complete_knock`；没进成时管理者再点一次，或 Agent 自己再敲一次，都会接着进。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedKnock {
    pub room: RedeemedRoom,
    pub agent: PrivateRoomAgentMemberRecord,
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

/// 口令对应的房间，Agent 随后按它入场。
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

    fn resolve(&self, request: ResolveJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>>;

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>>;

    /// 房间号对应的私人房间在使用中、收 Agent。敲门之前先核对，房间不对时什么都不建。
    fn knockable(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, AgentAccessResult<()>>;

    /// Agent 拿房间号敲门。已经是 Agent 成员的直接进；在等的再敲从现在重新算；以前没让进的不再
    /// 打扰管理者；房间里在等的满了按限流回答。
    fn knock(&self, request: KnockOnRoom) -> PortFuture<'_, AgentAccessResult<KnockResult>>;

    /// 这个 Agent 一天以内敲过的门，不论结果，先敲的在前。
    fn knocks_of(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>>;

    /// 房间里在等的敲门，先敲的在前；只有管理者看得到。
    fn waiting_knocks(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>>;

    /// 放行：把敲门的 Agent 记为 Agent 成员。敲门这时还在等，进去了再 `complete_knock`。
    fn admit_knock(&self, request: AnswerKnock)
    -> PortFuture<'_, AgentAccessResult<AdmittedKnock>>;

    /// 放行的 Agent 进去了：把敲门记为放进来了。
    fn complete_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>>;

    /// 不让进；原本就不在等也算成功。
    fn decline_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>>;
}

pub struct PrivateRoomAgentAccessDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub access: Arc<dyn PrivateRoomAgentAccessStore>,
    pub memberships: Arc<dyn AgentMembershipRepository>,
    pub matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
    pub attempts: JoinCodeAttemptPolicy,
}

pub struct PrivateRoomAgentAccessService {
    rooms: Arc<dyn PrivateRoomStore>,
    access: Arc<dyn PrivateRoomAgentAccessStore>,
    memberships: Arc<dyn AgentMembershipRepository>,
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
            memberships: dependencies.memberships,
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

    async fn resolve_internal(&self, request: ResolveJoinCode) -> AgentAccessResult<RedeemedRoom> {
        const OPERATION: &str = "private_room.join_code.resolve";
        let (_, snapshot) = self
            .checked_code(&request.caller, &request.code, OPERATION)
            .await?;
        Ok(redeemed_room(&snapshot))
    }

    async fn redeem_internal(&self, request: RedeemJoinCode) -> AgentAccessResult<RedeemedRoom> {
        const OPERATION: &str = "private_room.join_code.redeem";
        let memberships = self
            .memberships
            .find_memberships(request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::NotFound))?;
        // 能给这个 Agent 注册实例的人（主人或操作者），才能带它凭口令进房间。
        memberships
            .ensure_can_register_instance(request.actor.account.principal.id())
            .map_err(|error| domain(OPERATION, &error))?;
        let (record, snapshot) = self
            .checked_code(
                &JoinCodeCaller::Device(request.actor.clone()),
                &request.code,
                OPERATION,
            )
            .await?;
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
            .admit_agent(
                record.catalog_id,
                request.agent_id,
                record.permissions,
                PrivateRoomAgentJoinedVia::Code,
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(redeemed_room(&snapshot))
    }

    async fn knock_internal(&self, request: KnockOnRoom) -> AgentAccessResult<KnockResult> {
        const OPERATION: &str = "private_room.agent_knock.knock";
        let now = self.clock.now();
        if request.actor.access_token_expires_at <= now {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Forbidden,
            ));
        }
        let memberships = self
            .memberships
            .find_memberships(request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::NotFound))?;
        // 和凭口令一样：能给这个 Agent 注册实例的人，才能带它敲门。
        memberships
            .ensure_can_register_instance(request.actor.account.principal.id())
            .map_err(|error| domain(OPERATION, &error))?;
        let snapshot = self.knock_target(request.catalog_id, OPERATION).await?;
        let member = self
            .access
            .agent_member(request.catalog_id, request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        if member.is_some_and(|member| member.status == PrivateRoomAgentMemberStatus::Joined) {
            return Ok(KnockResult::Member(redeemed_room(&snapshot)));
        }
        let expires_at = UtcMillis::new(now.value().saturating_add(AGENT_KNOCK_TTL_MILLIS))
            .map_err(|error| domain(OPERATION, &error))?;
        match self
            .access
            .knock(
                request.catalog_id,
                request.agent_id,
                now,
                expires_at,
                MAX_WAITING_AGENT_KNOCKS,
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?
        {
            PrivateRoomAgentKnockOutcome::Waiting(record) => Ok(KnockResult::Waiting(record)),
            PrivateRoomAgentKnockOutcome::Declined(record) => Ok(KnockResult::Declined(record)),
            PrivateRoomAgentKnockOutcome::RoomFull { retry_at } => {
                Err(AgentAccessFailure::rate_limited(OPERATION, retry_at))
            }
        }
    }

    async fn knocks_of_internal(
        &self,
        agent_id: AgentId,
    ) -> AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>> {
        const OPERATION: &str = "private_room.agent_knock.of_agent";
        let since = UtcMillis::new((self.clock.now().value() - AGENT_KNOCK_HISTORY_MILLIS).max(0))
            .map_err(|error| domain(OPERATION, &error))?;
        self.access
            .agent_knocks(agent_id, since)
            .await
            .map_err(|error| repository(OPERATION, &error))
    }

    async fn waiting_knocks_internal(
        &self,
        request: InspectAgentAccess,
    ) -> AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>> {
        const OPERATION: &str = "private_room.agent_knock.waiting";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        self.access
            .waiting_knocks(request.catalog_id, self.clock.now())
            .await
            .map_err(|error| repository(OPERATION, &error))
    }

    async fn admit_knock_internal(&self, request: AnswerKnock) -> AgentAccessResult<AdmittedKnock> {
        const OPERATION: &str = "private_room.agent_knock.admit";
        let snapshot = self
            .managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let now = self.clock.now();
        // 只放行还在等的：作废了、Agent 停用了的都看不到，也放不进来。
        let waiting = self
            .access
            .waiting_knocks(request.catalog_id, now)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .into_iter()
            .any(|knock| knock.agent_id == request.agent_id);
        if !waiting {
            return Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::NotFound,
            ));
        }
        self.access
            .admit_agent(
                request.catalog_id,
                request.agent_id,
                PrivateRoomPermissions::AGENT_MEMBER,
                PrivateRoomAgentJoinedVia::Knock,
                now,
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        let agent = self
            .access
            .agent_member(request.catalog_id, request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(|| AgentAccessFailure::new(OPERATION, AgentAccessFailureKind::Internal))?;
        Ok(AdmittedKnock {
            room: redeemed_room(&snapshot),
            agent,
        })
    }

    async fn decide_knock(
        &self,
        request: AnswerKnock,
        status: PrivateRoomAgentKnockStatus,
        operation: &'static str,
    ) -> AgentAccessResult<()> {
        self.managed_room(&request.actor, request.catalog_id, operation)
            .await?;
        self.access
            .decide_knock(
                request.catalog_id,
                request.agent_id,
                status,
                request.actor.principal_id,
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(operation, &error))?;
        Ok(())
    }

    /// 敲门的房间：使用中的私人房间。不存在、不是私人房间、已归档的都说找不到，不说是哪一种。
    async fn knock_target(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> AgentAccessResult<PrivateRoomSnapshot> {
        let snapshot = self
            .rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| AgentAccessFailure::new(operation, AgentAccessFailureKind::NotFound))?;
        if !snapshot.room().admits_agent_member(true) {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::NotFound,
            ));
        }
        Ok(snapshot)
    }

    /// 设备有效、没被限流、口令格式对且存在、房间还在使用中。格式不对多半是抄错，不算一次猜测；
    /// 格式对但不存在才计入调用方的失败次数。
    async fn checked_code(
        &self,
        caller: &JoinCodeCaller,
        code: &str,
        operation: &'static str,
    ) -> AgentAccessResult<(PrivateRoomJoinCodeRecord, PrivateRoomSnapshot)> {
        let now = self.clock.now();
        let caller = match caller {
            JoinCodeCaller::Device(actor) => {
                if actor.access_token_expires_at <= now {
                    return Err(AgentAccessFailure::new(
                        operation,
                        AgentAccessFailureKind::Forbidden,
                    ));
                }
                format!("device:{}", actor.device_id)
            }
            JoinCodeCaller::NetworkSource(digest) => {
                digest
                    .iter()
                    .fold(String::from("network-source:"), |mut caller, byte| {
                        let _ = write!(caller, "{byte:02x}");
                        caller
                    })
            }
        };
        if let Some(retry_at) = self
            .access
            .join_code_retry_at(&caller, now, self.attempts)
            .await
            .map_err(|error| repository(operation, &error))?
        {
            return Err(AgentAccessFailure::rate_limited(operation, retry_at));
        }
        let code = PrivateRoomJoinCode::parse(code).map_err(|_| {
            AgentAccessFailure::new(operation, AgentAccessFailureKind::InvalidRequest)
        })?;
        let digest = self.secrets.digest(code.normalized());
        let Some(record) = self
            .access
            .find_join_code(&digest)
            .await
            .map_err(|error| repository(operation, &error))?
        else {
            self.access
                .record_join_code_failure(&caller, now, self.attempts)
                .await
                .map_err(|error| repository(operation, &error))?;
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::NotFound,
            ));
        };
        let snapshot = self
            .rooms
            .find_by_catalog(record.catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| AgentAccessFailure::new(operation, AgentAccessFailureKind::NotFound))?;
        if !snapshot.room().admits_agent_member(true) {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::Conflict,
            ));
        }
        Ok((record, snapshot))
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

    fn resolve(&self, request: ResolveJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        Box::pin(self.resolve_internal(request))
    }

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        Box::pin(self.redeem_internal(request))
    }

    fn knockable(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, AgentAccessResult<()>> {
        Box::pin(async move {
            self.knock_target(catalog_id, "private_room.agent_knock.check")
                .await
                .map(|_| ())
        })
    }

    fn knock(&self, request: KnockOnRoom) -> PortFuture<'_, AgentAccessResult<KnockResult>> {
        Box::pin(self.knock_internal(request))
    }

    fn knocks_of(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>> {
        Box::pin(self.knocks_of_internal(agent_id))
    }

    fn waiting_knocks(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>> {
        Box::pin(self.waiting_knocks_internal(request))
    }

    fn admit_knock(
        &self,
        request: AnswerKnock,
    ) -> PortFuture<'_, AgentAccessResult<AdmittedKnock>> {
        Box::pin(self.admit_knock_internal(request))
    }

    fn complete_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>> {
        Box::pin(self.decide_knock(
            request,
            PrivateRoomAgentKnockStatus::Admitted,
            "private_room.agent_knock.complete",
        ))
    }

    fn decline_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>> {
        Box::pin(self.decide_knock(
            request,
            PrivateRoomAgentKnockStatus::Declined,
            "private_room.agent_knock.decline",
        ))
    }
}

fn redeemed_room(snapshot: &PrivateRoomSnapshot) -> RedeemedRoom {
    RedeemedRoom {
        catalog_id: snapshot.catalog().id(),
        matrix_room_id: snapshot.instance().matrix_room_id().clone(),
        name: snapshot.catalog().name().to_owned(),
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
