use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomReservationId},
    rooms::{
        RoomCatalog, RoomInstance, RoomLanguage, RoomRegion, RoomReservation, RoomReservationState,
    },
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, MatrixFailure, RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore,
        RoomMembershipGateway, RoomReservationClaim, RoomReservationOutcome,
    },
};

const MAXIMUM_RESERVATION_LIFETIME_MILLIS: u64 = 300_000;

pub trait RoomReservationIdentifierFactory: Send + Sync {
    fn room_reservation_id(&self) -> RoomReservationId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyJoinPolicy {
    reservation_lifetime: DurationMillis,
}

impl LobbyJoinPolicy {
    /// 创建容量预约策略。
    ///
    /// # Errors
    ///
    /// 预约时间超过五分钟时返回错误；零时长已经由 `DurationMillis` 拒绝。
    pub fn new(reservation_lifetime: DurationMillis) -> Result<Self, DomainError> {
        if reservation_lifetime.value() > MAXIMUM_RESERVATION_LIFETIME_MILLIS {
            return Err(DomainError::Validation {
                field: "room_reservation_lifetime",
                reason: "不能超过五分钟",
            });
        }
        Ok(Self {
            reservation_lifetime,
        })
    }

    pub const fn reservation_lifetime(self) -> DurationMillis {
        self.reservation_lifetime
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinLobbyRequest {
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub catalog_id: RoomCatalogId,
    pub mode: RoomAllocationMode,
    pub preferred_language: Option<RoomLanguage>,
    pub preferred_region: Option<RoomRegion>,
    pub evidence: RoomAllocationEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyJoinKind {
    NewAssignment,
    RecoveredAssignment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinLobbyOutcome {
    Joined {
        reservation: RoomReservation,
        room: RoomInstance,
        kind: LobbyJoinKind,
    },
    ProvisioningRequired {
        catalog: RoomCatalog,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyJoinRollbackFailure {
    Matrix(MatrixFailure),
    Reservation(RepositoryError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinLobbyFailure {
    TimeOverflow,
    Allocation(RepositoryError),
    MatrixJoin(MatrixFailure),
    MatrixJoinCompensation {
        join: MatrixFailure,
        release: RepositoryError,
    },
    ConfirmationRolledBack(RepositoryError),
    ConfirmationRollbackFailed {
        confirmation: RepositoryError,
        rollback: LobbyJoinRollbackFailure,
    },
}

pub type JoinLobbyResult<T> = Result<T, JoinLobbyFailure>;

pub struct JoinLobbyService {
    allocations: Arc<dyn RoomAllocationStore>,
    membership: Arc<dyn RoomMembershipGateway>,
    identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: LobbyJoinPolicy,
}

pub struct JoinLobbyDependencies {
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub membership: Arc<dyn RoomMembershipGateway>,
    pub identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl JoinLobbyService {
    pub fn new(dependencies: JoinLobbyDependencies, policy: LobbyJoinPolicy) -> Self {
        Self {
            allocations: dependencies.allocations,
            membership: dependencies.membership,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            policy,
        }
    }

    /// 预约容量、加入 Matrix 并确认当前分配。
    ///
    /// # Errors
    ///
    /// 预约、Matrix 加入、确认或补偿失败时返回携带精确阶段的闭合错误。
    pub async fn join(&self, request: JoinLobbyRequest) -> JoinLobbyResult<JoinLobbyOutcome> {
        let now = self.clock.now();
        let expires_at = now
            .checked_add(self.policy.reservation_lifetime())
            .map_err(|_| JoinLobbyFailure::TimeOverflow)?;
        let claim = RoomReservationClaim {
            reservation_id: self.identifiers.room_reservation_id(),
            agent_id: request.agent_id,
            agent_instance_id: request.agent_instance_id,
            catalog_id: request.catalog_id,
            mode: request.mode,
            preferred_language: request.preferred_language,
            preferred_region: request.preferred_region,
            evidence: request.evidence,
            reserved_at: now,
            expires_at,
        };
        match self
            .allocations
            .reserve(&claim)
            .await
            .map_err(JoinLobbyFailure::Allocation)?
        {
            RoomReservationOutcome::ProvisioningRequired { catalog } => {
                Ok(JoinLobbyOutcome::ProvisioningRequired { catalog })
            }
            RoomReservationOutcome::ExistingAssignment { reservation, room } => {
                self.membership
                    .join(room.matrix_room_id())
                    .await
                    .map_err(JoinLobbyFailure::MatrixJoin)?;
                Ok(JoinLobbyOutcome::Joined {
                    reservation,
                    room,
                    kind: LobbyJoinKind::RecoveredAssignment,
                })
            }
            RoomReservationOutcome::Reserved { reservation, room } => {
                self.join_reserved(reservation, room, now).await
            }
        }
    }

    async fn join_reserved(
        &self,
        reservation: RoomReservation,
        room: RoomInstance,
        now: UtcMillis,
    ) -> JoinLobbyResult<JoinLobbyOutcome> {
        if let Err(join) = self.membership.join(room.matrix_room_id()).await {
            return match self
                .allocations
                .transition(
                    reservation.id(),
                    RoomReservationState::Reserved,
                    RoomReservationState::Released,
                    now,
                )
                .await
            {
                Ok(_) => Err(JoinLobbyFailure::MatrixJoin(join)),
                Err(release) => Err(JoinLobbyFailure::MatrixJoinCompensation { join, release }),
            };
        }

        match self
            .allocations
            .transition(
                reservation.id(),
                RoomReservationState::Reserved,
                RoomReservationState::Committed,
                now,
            )
            .await
        {
            Ok(committed) => Ok(JoinLobbyOutcome::Joined {
                reservation: committed,
                room,
                kind: LobbyJoinKind::NewAssignment,
            }),
            Err(confirmation) => {
                let rollback = self.rollback_join(&reservation, &room, now).await;
                match rollback {
                    Ok(()) => Err(JoinLobbyFailure::ConfirmationRolledBack(confirmation)),
                    Err(rollback) => Err(JoinLobbyFailure::ConfirmationRollbackFailed {
                        confirmation,
                        rollback,
                    }),
                }
            }
        }
    }

    async fn rollback_join(
        &self,
        reservation: &RoomReservation,
        room: &RoomInstance,
        now: UtcMillis,
    ) -> Result<(), LobbyJoinRollbackFailure> {
        self.membership
            .leave(room.matrix_room_id())
            .await
            .map_err(LobbyJoinRollbackFailure::Matrix)?;
        self.allocations
            .transition(
                reservation.id(),
                RoomReservationState::Reserved,
                RoomReservationState::Released,
                now,
            )
            .await
            .map_err(LobbyJoinRollbackFailure::Reservation)?;
        Ok(())
    }
}
