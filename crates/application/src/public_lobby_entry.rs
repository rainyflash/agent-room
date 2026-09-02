use std::sync::Arc;

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{PortFuture, PublicLobbyObservationRoom, RoomDirectory},
    rooms::{
        LobbyProvisioningFailure, LobbyProvisioningOperation, LobbyProvisioningOutcome,
        LobbyProvisioningRequest,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicLobbyEntryFailureKind {
    NotFound,
    ProvisioningBusy,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicLobbyEntryFailure {
    NotFound,
    ProvisioningBusy { retry_at: UtcMillis },
    Directory(RepositoryError),
    Provisioning(LobbyProvisioningFailure),
    ProvisioningResultMismatch,
}

impl PublicLobbyEntryFailure {
    pub const fn kind(&self) -> PublicLobbyEntryFailureKind {
        match self {
            Self::NotFound => PublicLobbyEntryFailureKind::NotFound,
            Self::ProvisioningBusy { .. } => PublicLobbyEntryFailureKind::ProvisioningBusy,
            Self::Directory(error) => repository_failure_kind(error.kind()),
            Self::Provisioning(failure) => provisioning_failure_kind(failure),
            Self::ProvisioningResultMismatch => PublicLobbyEntryFailureKind::Internal,
        }
    }

    pub const fn retry_at(&self) -> Option<UtcMillis> {
        match self {
            Self::ProvisioningBusy { retry_at } => Some(*retry_at),
            Self::NotFound
            | Self::Directory(_)
            | Self::Provisioning(_)
            | Self::ProvisioningResultMismatch => None,
        }
    }
}

pub type PublicLobbyEntryResult<T> = Result<T, PublicLobbyEntryFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnterPublicLobby {
    pub catalog_id: RoomCatalogId,
}

pub trait PublicLobbyEntryUseCases: Send + Sync {
    fn enter(
        &self,
        request: EnterPublicLobby,
    ) -> PortFuture<'_, PublicLobbyEntryResult<PublicLobbyObservationRoom>>;
}

pub struct PublicLobbyEntryDependencies {
    pub directory: Arc<dyn RoomDirectory>,
    pub provisioning: Arc<dyn LobbyProvisioningOperation>,
}

/// 公开大厅属于云端基础设施；没有 Agent 在线时也必须能由控制面幂等供给。
pub struct PublicLobbyEntryService {
    directory: Arc<dyn RoomDirectory>,
    provisioning: Arc<dyn LobbyProvisioningOperation>,
}

impl PublicLobbyEntryService {
    pub fn new(dependencies: PublicLobbyEntryDependencies) -> Self {
        Self {
            directory: dependencies.directory,
            provisioning: dependencies.provisioning,
        }
    }

    async fn enter_internal(
        &self,
        request: EnterPublicLobby,
    ) -> PublicLobbyEntryResult<PublicLobbyObservationRoom> {
        if let Some(room) = self
            .directory
            .find_public_observation_room(request.catalog_id)
            .await
            .map_err(PublicLobbyEntryFailure::Directory)?
        {
            return Ok(room);
        }

        let catalog = self
            .directory
            .find_catalog(request.catalog_id)
            .await
            .map_err(PublicLobbyEntryFailure::Directory)?
            .ok_or(PublicLobbyEntryFailure::NotFound)?;
        let outcome = self
            .provisioning
            .provision(LobbyProvisioningRequest {
                catalog,
                preferred_region: None,
            })
            .await
            .map_err(PublicLobbyEntryFailure::Provisioning)?;
        match outcome {
            LobbyProvisioningOutcome::Busy { retry_at } => {
                Err(PublicLobbyEntryFailure::ProvisioningBusy { retry_at })
            }
            LobbyProvisioningOutcome::Ready(ready) => {
                if ready.catalog.id() != request.catalog_id
                    || ready.room.catalog_id() != request.catalog_id
                {
                    return Err(PublicLobbyEntryFailure::ProvisioningResultMismatch);
                }
                Ok(PublicLobbyObservationRoom {
                    catalog_id: request.catalog_id,
                    room_instance_id: ready.room.id(),
                    matrix_room_id: ready.room.matrix_room_id().clone(),
                })
            }
        }
    }
}

impl PublicLobbyEntryUseCases for PublicLobbyEntryService {
    fn enter(
        &self,
        request: EnterPublicLobby,
    ) -> PortFuture<'_, PublicLobbyEntryResult<PublicLobbyObservationRoom>> {
        Box::pin(self.enter_internal(request))
    }
}

const fn repository_failure_kind(kind: RepositoryErrorKind) -> PublicLobbyEntryFailureKind {
    match kind {
        RepositoryErrorKind::Unavailable => PublicLobbyEntryFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => PublicLobbyEntryFailureKind::NotFound,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => PublicLobbyEntryFailureKind::Internal,
    }
}

const fn provisioning_failure_kind(
    failure: &LobbyProvisioningFailure,
) -> PublicLobbyEntryFailureKind {
    match failure {
        LobbyProvisioningFailure::Store { source, .. } => repository_failure_kind(source.kind()),
        LobbyProvisioningFailure::Matrix { source, .. } => matrix_failure_kind(source.kind()),
        LobbyProvisioningFailure::MatrixReleaseFailed {
            source, release, ..
        } => {
            if matches!(
                matrix_failure_kind(source.kind()),
                PublicLobbyEntryFailureKind::DependencyUnavailable
            ) || matches!(
                repository_failure_kind(release.kind()),
                PublicLobbyEntryFailureKind::DependencyUnavailable
            ) {
                PublicLobbyEntryFailureKind::DependencyUnavailable
            } else {
                PublicLobbyEntryFailureKind::Internal
            }
        }
        LobbyProvisioningFailure::Invalid(_) | LobbyProvisioningFailure::TimeOverflow => {
            PublicLobbyEntryFailureKind::Internal
        }
    }
}

const fn matrix_failure_kind(kind: crate::ports::MatrixFailureKind) -> PublicLobbyEntryFailureKind {
    match kind {
        crate::ports::MatrixFailureKind::Timeout
        | crate::ports::MatrixFailureKind::DependencyUnavailable
        | crate::ports::MatrixFailureKind::RateLimited => {
            PublicLobbyEntryFailureKind::DependencyUnavailable
        }
        crate::ports::MatrixFailureKind::Unauthenticated
        | crate::ports::MatrixFailureKind::AuthenticationRejected
        | crate::ports::MatrixFailureKind::Forbidden
        | crate::ports::MatrixFailureKind::NotFound
        | crate::ports::MatrixFailureKind::Conflict
        | crate::ports::MatrixFailureKind::UnknownCommit
        | crate::ports::MatrixFailureKind::InvalidConfiguration
        | crate::ports::MatrixFailureKind::CryptographicIdentityConflict
        | crate::ports::MatrixFailureKind::InvalidResponse
        | crate::ports::MatrixFailureKind::StaleSyncToken
        | crate::ports::MatrixFailureKind::UnsupportedVersion => {
            PublicLobbyEntryFailureKind::Internal
        }
    }
}
