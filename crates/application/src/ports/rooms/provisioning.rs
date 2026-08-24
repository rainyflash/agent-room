use agent_room_domain::{
    DomainError, DomainResult,
    ids::{RoomCatalogId, RoomInstanceId, RoomProvisioningJobId, RoomProvisioningLeaseId},
    rooms::{MatrixRoomReference, RoomCatalog, RoomInstance, RoomRegion},
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixCreateRoom, MatrixEventId, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomId,
        PortFuture,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomProvisioningTarget {
    Space,
    Instance {
        room_instance_id: RoomInstanceId,
        region: Option<RoomRegion>,
    },
}

impl RoomProvisioningTarget {
    pub const fn kind(&self) -> RoomProvisioningKind {
        match self {
            Self::Space => RoomProvisioningKind::Space,
            Self::Instance { .. } => RoomProvisioningKind::Instance,
        }
    }

    pub const fn room_instance_id(&self) -> Option<RoomInstanceId> {
        match self {
            Self::Space => None,
            Self::Instance {
                room_instance_id, ..
            } => Some(*room_instance_id),
        }
    }

    pub const fn region(&self) -> Option<&RoomRegion> {
        match self {
            Self::Space => None,
            Self::Instance { region, .. } => region.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomProvisioningKind {
    Space,
    Instance,
}

impl RoomProvisioningKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Instance => "instance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomProvisioningFailureCode {
    MatrixCreate,
    MatrixResolve,
    SpaceAttach,
}

impl RoomProvisioningFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MatrixCreate => "matrix_create",
            Self::MatrixResolve => "matrix_resolve",
            Self::SpaceAttach => "space_attach",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomProvisioningClaim {
    job_id: RoomProvisioningJobId,
    lease_id: RoomProvisioningLeaseId,
    catalog_id: RoomCatalogId,
    target: RoomProvisioningTarget,
    alias_localpart: MatrixRoomAliasLocalpart,
    claimed_at: UtcMillis,
    expires_at: UtcMillis,
}

impl RoomProvisioningClaim {
    /// 创建带 fencing token 的建房租约请求。
    ///
    /// # Errors
    ///
    /// 租约未晚于声明时间时返回校验错误。
    pub fn new(
        job_id: RoomProvisioningJobId,
        lease_id: RoomProvisioningLeaseId,
        catalog_id: RoomCatalogId,
        target: RoomProvisioningTarget,
        alias_localpart: MatrixRoomAliasLocalpart,
        claimed_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> DomainResult<Self> {
        if expires_at <= claimed_at {
            return Err(DomainError::Validation {
                field: "room_provisioning_lease",
                reason: "租约过期时间必须晚于声明时间",
            });
        }
        Ok(Self {
            job_id,
            lease_id,
            catalog_id,
            target,
            alias_localpart,
            claimed_at,
            expires_at,
        })
    }

    pub const fn job_id(&self) -> RoomProvisioningJobId {
        self.job_id
    }

    pub const fn lease_id(&self) -> RoomProvisioningLeaseId {
        self.lease_id
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn target(&self) -> &RoomProvisioningTarget {
        &self.target
    }

    pub const fn alias_localpart(&self) -> &MatrixRoomAliasLocalpart {
        &self.alias_localpart
    }

    pub const fn claimed_at(&self) -> UtcMillis {
        self.claimed_at
    }

    pub const fn expires_at(&self) -> UtcMillis {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomProvisioningJob {
    job_id: RoomProvisioningJobId,
    lease_id: RoomProvisioningLeaseId,
    catalog: RoomCatalog,
    target: RoomProvisioningTarget,
    alias_localpart: MatrixRoomAliasLocalpart,
    matrix_room_id: Option<MatrixRoomReference>,
    lease_expires_at: UtcMillis,
}

impl RoomProvisioningJob {
    pub const fn restore(
        job_id: RoomProvisioningJobId,
        lease_id: RoomProvisioningLeaseId,
        catalog: RoomCatalog,
        target: RoomProvisioningTarget,
        alias_localpart: MatrixRoomAliasLocalpart,
        matrix_room_id: Option<MatrixRoomReference>,
        lease_expires_at: UtcMillis,
    ) -> Self {
        Self {
            job_id,
            lease_id,
            catalog,
            target,
            alias_localpart,
            matrix_room_id,
            lease_expires_at,
        }
    }

    pub const fn job_id(&self) -> RoomProvisioningJobId {
        self.job_id
    }

    pub const fn lease_id(&self) -> RoomProvisioningLeaseId {
        self.lease_id
    }

    pub const fn catalog(&self) -> &RoomCatalog {
        &self.catalog
    }

    pub const fn target(&self) -> &RoomProvisioningTarget {
        &self.target
    }

    pub const fn alias_localpart(&self) -> &MatrixRoomAliasLocalpart {
        &self.alias_localpart
    }

    pub const fn matrix_room_id(&self) -> Option<&MatrixRoomReference> {
        self.matrix_room_id.as_ref()
    }

    pub const fn lease_expires_at(&self) -> UtcMillis {
        self.lease_expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomProvisioningClaimOutcome {
    Claimed(RoomProvisioningJob),
    Busy { retry_at: UtcMillis },
    SpaceReady { catalog: RoomCatalog },
    InstanceReady { room: RoomInstance },
}

pub trait RoomProvisioningStore: Send + Sync {
    fn claim<'a>(
        &'a self,
        claim: &'a RoomProvisioningClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomProvisioningClaimOutcome>>;

    fn checkpoint_matrix_room<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_room_id: &'a MatrixRoomReference,
        checkpointed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    fn complete_space<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_space_id: &'a MatrixRoomReference,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomCatalog>>;

    fn complete_instance<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        room: &'a RoomInstance,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomInstance>>;

    fn release<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        failure: RoomProvisioningFailureCode,
        released_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

/// 只暴露建房编排需要的 Matrix 能力，隐藏状态事件 JSON 细节。
pub trait RoomProvisioningGateway: Send + Sync {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>>;

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>>;

    fn attach_child<'a>(
        &'a self,
        space_id: &'a MatrixRoomId,
        child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>>;
}
