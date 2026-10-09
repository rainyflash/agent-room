use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{RoomInstanceId, RoomProvisioningJobId, RoomProvisioningLeaseId},
    moderation::{ModerationAction, ModerationActionKind},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogKind, RoomCatalogVisibility,
        RoomInstance, RoomInstanceFields, RoomInstanceState, RoomRegion,
    },
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, MatrixCreateRoom, MatrixEventType, MatrixFailure, MatrixFailureKind,
        MatrixRoomAliasLocalpart, MatrixRoomId, MatrixRoomKind, MatrixRoomPreset,
        MatrixRoomVisibility, ModerationEffectGateway, ModerationEffectTarget,
        RoomProvisioningClaim, RoomProvisioningClaimOutcome, RoomProvisioningFailureCode,
        RoomProvisioningGateway, RoomProvisioningJob, RoomProvisioningKind, RoomProvisioningStore,
        RoomProvisioningTarget, StandingModerationSource,
    },
};

const MAXIMUM_PROVISIONING_LEASE_MILLIS: u64 = 300_000;
const DEFAULT_PUBLIC_RETENTION_DAYS: u16 = 30;
const AGENT_STATUS_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.agent.status.v1";

pub trait LobbyProvisioningIdentifierFactory: Send + Sync {
    fn room_provisioning_job_id(&self) -> RoomProvisioningJobId;
    fn room_provisioning_lease_id(&self) -> RoomProvisioningLeaseId;
    fn room_instance_id(&self) -> RoomInstanceId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyProvisioningPolicy {
    lease_lifetime: DurationMillis,
}

impl LobbyProvisioningPolicy {
    /// 创建建房租约策略。
    ///
    /// # Errors
    ///
    /// 租约超过五分钟时返回校验错误。
    pub fn new(lease_lifetime: DurationMillis) -> Result<Self, DomainError> {
        if lease_lifetime.value() > MAXIMUM_PROVISIONING_LEASE_MILLIS {
            return Err(DomainError::Validation {
                field: "room_provisioning_lease_lifetime",
                reason: "不能超过五分钟",
            });
        }
        Ok(Self { lease_lifetime })
    }

    pub const fn lease_lifetime(self) -> DurationMillis {
        self.lease_lifetime
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyProvisioningRequest {
    pub catalog: RoomCatalog,
    pub preferred_region: Option<RoomRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedLobby {
    pub catalog: RoomCatalog,
    pub room: RoomInstance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyProvisioningOutcome {
    Ready(Box<ProvisionedLobby>),
    Busy { retry_at: UtcMillis },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyProvisioningFailureStage {
    ClaimSpace,
    CreateSpace,
    ResolveSpace,
    CheckpointSpace,
    CompleteSpace,
    ClaimInstance,
    CreateInstance,
    ResolveInstance,
    CheckpointInstance,
    AttachInstance,
    /// 开始接人之前补上这个大厅生效的禁言、封禁：读不到要补的是 `Store`，补不上是 `Matrix`。
    CarryOverModeration,
    CompleteInstance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyProvisioningFailure {
    Invalid(DomainError),
    TimeOverflow,
    Store {
        stage: LobbyProvisioningFailureStage,
        source: RepositoryError,
    },
    Matrix {
        stage: LobbyProvisioningFailureStage,
        source: MatrixFailure,
    },
    MatrixReleaseFailed {
        stage: LobbyProvisioningFailureStage,
        source: MatrixFailure,
        release: RepositoryError,
    },
}

pub type LobbyProvisioningResult<T> = Result<T, LobbyProvisioningFailure>;

pub struct LobbyProvisioningDependencies {
    pub store: Arc<dyn RoomProvisioningStore>,
    pub matrix: Arc<dyn RoomProvisioningGateway>,
    /// 新分片要补上的禁言、封禁从哪里读。
    pub moderation: Arc<dyn StandingModerationSource>,
    /// 补的时候和管理员落治理用同一套 Matrix 效果。
    pub moderation_effects: Arc<dyn ModerationEffectGateway>,
    pub identifiers: Arc<dyn LobbyProvisioningIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct LobbyProvisioningService {
    store: Arc<dyn RoomProvisioningStore>,
    matrix: Arc<dyn RoomProvisioningGateway>,
    moderation: Arc<dyn StandingModerationSource>,
    moderation_effects: Arc<dyn ModerationEffectGateway>,
    identifiers: Arc<dyn LobbyProvisioningIdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: LobbyProvisioningPolicy,
}

impl LobbyProvisioningService {
    pub fn new(
        dependencies: LobbyProvisioningDependencies,
        policy: LobbyProvisioningPolicy,
    ) -> Self {
        Self {
            store: dependencies.store,
            matrix: dependencies.matrix,
            moderation: dependencies.moderation,
            moderation_effects: dependencies.moderation_effects,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            policy,
        }
    }

    /// 确保公共大厅已有 Matrix Space 和至少一个可分配房间实例。
    ///
    /// # Errors
    ///
    /// 目录不合法、租约、Matrix 或持久化任一阶段失败时返回精确阶段错误。
    pub async fn provision(
        &self,
        request: LobbyProvisioningRequest,
    ) -> LobbyProvisioningResult<LobbyProvisioningOutcome> {
        validate_catalog(&request.catalog).map_err(LobbyProvisioningFailure::Invalid)?;
        let catalog = match self.ensure_space(request.catalog).await? {
            EnsureSpaceOutcome::Ready(catalog) => catalog,
            EnsureSpaceOutcome::Busy(retry_at) => {
                return Ok(LobbyProvisioningOutcome::Busy { retry_at });
            }
        };
        self.ensure_instance(catalog, request.preferred_region)
            .await
    }

    async fn ensure_space(
        &self,
        catalog: RoomCatalog,
    ) -> LobbyProvisioningResult<EnsureSpaceOutcome> {
        if catalog.matrix_space_id().is_some() {
            return Ok(EnsureSpaceOutcome::Ready(catalog));
        }
        let alias = space_alias(&catalog).map_err(LobbyProvisioningFailure::Invalid)?;
        let claim = self.claim(catalog.id(), RoomProvisioningTarget::Space, alias)?;
        let outcome =
            self.store.claim(&claim).await.map_err(|source| {
                store_failure(LobbyProvisioningFailureStage::ClaimSpace, source)
            })?;
        let job = match outcome {
            RoomProvisioningClaimOutcome::Claimed(job) => {
                ensure_job_kind(&job, RoomProvisioningKind::Space)
                    .map_err(LobbyProvisioningFailure::Invalid)?;
                job
            }
            RoomProvisioningClaimOutcome::Busy { retry_at } => {
                return Ok(EnsureSpaceOutcome::Busy(retry_at));
            }
            RoomProvisioningClaimOutcome::SpaceReady { catalog } => {
                return Ok(EnsureSpaceOutcome::Ready(catalog));
            }
            RoomProvisioningClaimOutcome::InstanceReady { .. } => {
                return Err(invalid_store_outcome("Space 租约返回了房间实例"));
            }
        };
        let matrix_space_id = self
            .ensure_matrix_room(
                &job,
                MatrixRoomKind::Space,
                LobbyProvisioningFailureStage::CreateSpace,
                LobbyProvisioningFailureStage::ResolveSpace,
                LobbyProvisioningFailureStage::CheckpointSpace,
            )
            .await?;
        let catalog = self
            .store
            .complete_space(&job, &matrix_space_id, self.clock.now())
            .await
            .map_err(|source| {
                store_failure(LobbyProvisioningFailureStage::CompleteSpace, source)
            })?;
        Ok(EnsureSpaceOutcome::Ready(catalog))
    }

    async fn ensure_instance(
        &self,
        catalog: RoomCatalog,
        preferred_region: Option<RoomRegion>,
    ) -> LobbyProvisioningResult<LobbyProvisioningOutcome> {
        let space_id = catalog
            .matrix_space_id()
            .cloned()
            .ok_or_else(|| invalid_store_outcome("完成 Space 后目录仍缺少 Matrix Space 标识"))?;
        let proposed_instance_id = self.identifiers.room_instance_id();
        let target = RoomProvisioningTarget::Instance {
            room_instance_id: proposed_instance_id,
            region: preferred_region,
        };
        let alias = instance_alias(&catalog, proposed_instance_id)
            .map_err(LobbyProvisioningFailure::Invalid)?;
        let claim = self.claim(catalog.id(), target, alias)?;
        let outcome = self.store.claim(&claim).await.map_err(|source| {
            store_failure(LobbyProvisioningFailureStage::ClaimInstance, source)
        })?;
        let job = match outcome {
            RoomProvisioningClaimOutcome::Claimed(job) => {
                ensure_job_kind(&job, RoomProvisioningKind::Instance)
                    .map_err(LobbyProvisioningFailure::Invalid)?;
                job
            }
            RoomProvisioningClaimOutcome::Busy { retry_at } => {
                return Ok(LobbyProvisioningOutcome::Busy { retry_at });
            }
            RoomProvisioningClaimOutcome::InstanceReady { room } => {
                return Ok(LobbyProvisioningOutcome::Ready(Box::new(
                    ProvisionedLobby { catalog, room },
                )));
            }
            RoomProvisioningClaimOutcome::SpaceReady { .. } => {
                return Err(invalid_store_outcome("房间实例租约返回了 Space"));
            }
        };
        let matrix_room_id = self
            .ensure_matrix_room(
                &job,
                MatrixRoomKind::Conversation,
                LobbyProvisioningFailureStage::CreateInstance,
                LobbyProvisioningFailureStage::ResolveInstance,
                LobbyProvisioningFailureStage::CheckpointInstance,
            )
            .await?;
        self.attach_instance(&job, &space_id, &matrix_room_id)
            .await?;
        self.carry_over_moderation(&job, &matrix_room_id).await?;
        let room =
            build_room_instance(&job, matrix_room_id).map_err(LobbyProvisioningFailure::Invalid)?;
        let room = self
            .store
            .complete_instance(&job, &room, self.clock.now())
            .await
            .map_err(|source| {
                store_failure(LobbyProvisioningFailureStage::CompleteInstance, source)
            })?;
        Ok(LobbyProvisioningOutcome::Ready(Box::new(
            ProvisionedLobby { catalog, room },
        )))
    }

    fn claim(
        &self,
        catalog_id: agent_room_domain::ids::RoomCatalogId,
        target: RoomProvisioningTarget,
        alias: MatrixRoomAliasLocalpart,
    ) -> LobbyProvisioningResult<RoomProvisioningClaim> {
        let now = self.clock.now();
        let expires_at = now
            .checked_add(self.policy.lease_lifetime())
            .map_err(|_| LobbyProvisioningFailure::TimeOverflow)?;
        RoomProvisioningClaim::new(
            self.identifiers.room_provisioning_job_id(),
            self.identifiers.room_provisioning_lease_id(),
            catalog_id,
            target,
            alias,
            now,
            expires_at,
        )
        .map_err(LobbyProvisioningFailure::Invalid)
    }

    async fn ensure_matrix_room(
        &self,
        job: &RoomProvisioningJob,
        kind: MatrixRoomKind,
        create_stage: LobbyProvisioningFailureStage,
        resolve_stage: LobbyProvisioningFailureStage,
        checkpoint_stage: LobbyProvisioningFailureStage,
    ) -> LobbyProvisioningResult<MatrixRoomReference> {
        if let Some(matrix_room_id) = job.matrix_room_id() {
            return Ok(matrix_room_id.clone());
        }
        let request = create_room_request(job, kind).map_err(LobbyProvisioningFailure::Invalid)?;
        let matrix_room_id = match self.matrix.create_room(&request).await {
            Ok(matrix_room_id) => matrix_room_id,
            Err(source)
                if matches!(
                    source.kind(),
                    MatrixFailureKind::Conflict | MatrixFailureKind::UnknownCommit
                ) =>
            {
                match self.matrix.resolve_room_alias(job.alias_localpart()).await {
                    Ok(matrix_room_id) => matrix_room_id,
                    Err(resolve) => {
                        return Err(self
                            .release_matrix_failure(
                                job,
                                RoomProvisioningFailureCode::MatrixResolve,
                                resolve_stage,
                                resolve,
                            )
                            .await);
                    }
                }
            }
            Err(source) => {
                return Err(self
                    .release_matrix_failure(
                        job,
                        RoomProvisioningFailureCode::MatrixCreate,
                        create_stage,
                        source,
                    )
                    .await);
            }
        };
        let matrix_room_reference = MatrixRoomReference::new(matrix_room_id.as_str().to_owned())
            .map_err(LobbyProvisioningFailure::Invalid)?;
        self.store
            .checkpoint_matrix_room(job, &matrix_room_reference, self.clock.now())
            .await
            .map_err(|source| store_failure(checkpoint_stage, source))?;
        Ok(matrix_room_reference)
    }

    async fn attach_instance(
        &self,
        job: &RoomProvisioningJob,
        space_id: &MatrixRoomReference,
        child_id: &MatrixRoomReference,
    ) -> LobbyProvisioningResult<()> {
        let space_id = MatrixRoomId::new(space_id.as_str().to_owned())
            .map_err(|_| invalid_matrix_reference())?;
        let child_id = MatrixRoomId::new(child_id.as_str().to_owned())
            .map_err(|_| invalid_matrix_reference())?;
        if let Err(source) = self.matrix.attach_child(&space_id, &child_id).await {
            return Err(self
                .release_matrix_failure(
                    job,
                    RoomProvisioningFailureCode::SpaceAttach,
                    LobbyProvisioningFailureStage::AttachInstance,
                    source,
                )
                .await);
        }
        Ok(())
    }

    /// 新分片开始接人之前，补上这个大厅此刻仍生效的禁言和封禁：人下次进大厅可能被分到这里，不补
    /// 的话他在这儿照常说话、照常进来。补在发布实例之前，补不上就和建房间失败一样放掉租约，分片
    /// 不接人；下一个进大厅的人接着建，Matrix 房间已经记下不再重建，补过的再补一遍什么也不变。
    async fn carry_over_moderation(
        &self,
        job: &RoomProvisioningJob,
        matrix_room_id: &MatrixRoomReference,
    ) -> LobbyProvisioningResult<()> {
        const STAGE: LobbyProvisioningFailureStage =
            LobbyProvisioningFailureStage::CarryOverModeration;
        let now = self.clock.now();
        let standing = self
            .moderation
            .standing_person_actions(job.catalog().id(), now)
            .await
            .map_err(|source| store_failure(STAGE, source))?;
        let room = MatrixRoomId::new(matrix_room_id.as_str().to_owned())
            .map_err(|_| invalid_matrix_reference())?;
        for item in standing
            .iter()
            .filter(|item| carries_over(&item.action, now))
        {
            let target = ModerationEffectTarget {
                matrix_room_id: room.clone(),
                room_kind: job.catalog().kind(),
                target: item.action.target().clone(),
                target_matrix_user_id: Some(item.target_matrix_user_id.clone()),
            };
            if let Err(source) = self.moderation_effects.apply(&item.action, &target).await {
                return Err(self
                    .release_matrix_failure(
                        job,
                        RoomProvisioningFailureCode::ModerationCarryOver,
                        STAGE,
                        source,
                    )
                    .await);
            }
        }
        Ok(())
    }

    async fn release_matrix_failure(
        &self,
        job: &RoomProvisioningJob,
        code: RoomProvisioningFailureCode,
        stage: LobbyProvisioningFailureStage,
        source: MatrixFailure,
    ) -> LobbyProvisioningFailure {
        match self.store.release(job, code, self.clock.now()).await {
            Ok(()) => LobbyProvisioningFailure::Matrix { stage, source },
            Err(release) => LobbyProvisioningFailure::MatrixReleaseFailed {
                stage,
                source,
                release,
            },
        }
    }
}

enum EnsureSpaceOutcome {
    Ready(RoomCatalog),
    Busy(UtcMillis),
}

fn validate_catalog(catalog: &RoomCatalog) -> Result<(), DomainError> {
    if catalog.kind() != RoomCatalogKind::PublicLobby {
        return Err(DomainError::InvariantViolation {
            entity: "room_catalog",
            rule: "自动分片只接受公共大厅目录",
        });
    }
    if !catalog.is_joinable() {
        return Err(DomainError::InvalidTransition {
            entity: "room_catalog",
            from: catalog.status().as_str(),
            to: "provisioning",
        });
    }
    Ok(())
}

/// 只补管人、一直生效到撤销或到期的两种：禁言和封禁。踢出是一次性的，隐藏只管消息所在的分片。
fn carries_over(action: &ModerationAction, now: UtcMillis) -> bool {
    matches!(
        action.kind(),
        ModerationActionKind::Mute | ModerationActionKind::Ban
    ) && action.is_effective_at(now)
}

fn ensure_job_kind(
    job: &RoomProvisioningJob,
    expected: RoomProvisioningKind,
) -> Result<(), DomainError> {
    if job.target().kind() != expected {
        return Err(DomainError::InvariantViolation {
            entity: "room_provisioning_job",
            rule: "持久化返回的建房目标类型与请求不一致",
        });
    }
    Ok(())
}

fn space_alias(catalog: &RoomCatalog) -> Result<MatrixRoomAliasLocalpart, DomainError> {
    let slug = catalog.slug().ok_or(DomainError::InvariantViolation {
        entity: "room_catalog",
        rule: "公共大厅必须有稳定短名",
    })?;
    MatrixRoomAliasLocalpart::new(format!("agent-room-space-{}", slug.as_str()))
        .map_err(|_| invalid_alias())
}

fn instance_alias(
    catalog: &RoomCatalog,
    room_instance_id: RoomInstanceId,
) -> Result<MatrixRoomAliasLocalpart, DomainError> {
    let slug = catalog.slug().ok_or(DomainError::InvariantViolation {
        entity: "room_catalog",
        rule: "公共大厅必须有稳定短名",
    })?;
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-{}-{}",
        slug.as_str(),
        room_instance_id.as_uuid().simple()
    ))
    .map_err(|_| invalid_alias())
}

fn create_room_request(
    job: &RoomProvisioningJob,
    kind: MatrixRoomKind,
) -> Result<MatrixCreateRoom, DomainError> {
    let catalog = job.catalog();
    let visibility = match catalog.visibility() {
        RoomCatalogVisibility::Public => MatrixRoomVisibility::Public,
        RoomCatalogVisibility::Unlisted | RoomCatalogVisibility::Private => {
            MatrixRoomVisibility::Private
        }
    };
    let request = MatrixCreateRoom::new(
        Some(catalog.name().to_owned()),
        Some(catalog.description().to_owned()),
        visibility,
        MatrixRoomPreset::PublicChat,
        false,
        Vec::new(),
    )?
    .with_kind(kind)
    .with_alias_localpart(job.alias_localpart().clone());
    match kind {
        MatrixRoomKind::Conversation => {
            let request = request.with_retention_days(
                catalog
                    .retention_days()
                    .unwrap_or(DEFAULT_PUBLIC_RETENTION_DAYS),
            )?;
            let event_type = MatrixEventType::new(AGENT_STATUS_EVENT_TYPE).map_err(|_| {
                DomainError::InvariantViolation {
                    entity: "room_provisioning_policy",
                    rule: "Agent 状态事件类型必须符合 Matrix 事件命名规则",
                }
            })?;
            Ok(request.with_member_writable_state_event_type(event_type))
        }
        MatrixRoomKind::Space => Ok(request),
    }
}

fn build_room_instance(
    job: &RoomProvisioningJob,
    matrix_room_id: MatrixRoomReference,
) -> Result<RoomInstance, DomainError> {
    let room_instance_id =
        job.target()
            .room_instance_id()
            .ok_or(DomainError::InvariantViolation {
                entity: "room_provisioning_job",
                rule: "房间实例任务缺少实例标识",
            })?;
    RoomInstance::restore(
        room_instance_id,
        RoomInstanceFields {
            catalog_id: job.catalog().id(),
            matrix_room_id,
            region: job.target().region().cloned(),
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
}

const fn invalid_alias() -> DomainError {
    DomainError::Validation {
        field: "matrix_room_alias_localpart",
        reason: "无法从目录生成安全的 Matrix 房间别名",
    }
}

const fn invalid_matrix_reference() -> LobbyProvisioningFailure {
    LobbyProvisioningFailure::Invalid(DomainError::InvariantViolation {
        entity: "room_provisioning_job",
        rule: "持久化中的 Matrix 房间标识无法进入协议端口",
    })
}

const fn invalid_store_outcome(rule: &'static str) -> LobbyProvisioningFailure {
    LobbyProvisioningFailure::Invalid(DomainError::InvariantViolation {
        entity: "room_provisioning_store",
        rule,
    })
}

const fn store_failure(
    stage: LobbyProvisioningFailureStage,
    source: RepositoryError,
) -> LobbyProvisioningFailure {
    LobbyProvisioningFailure::Store { stage, source }
}
