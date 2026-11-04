//! 房主改房间简介（`specs/public-rooms/design.md`）：公开房间的简介写在发现页和分享页上，私密的
//! 房间也一样能改。只有房主，和改名一样；房间要在使用中。
//!
//! 简介也是 Matrix 房间的话题。先写 Matrix：它可以重复设置，目录写失败时房主再点一次就再走一遍。

use std::sync::Arc;

use agent_room_domain::{DomainError, ids::RoomCatalogId, private_rooms::PrivateRoomMember};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixRoomId, PortFuture,
        PrivateRoomDescriptionStore, PrivateRoomStore, PrivateRoomTopicGateway,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionFailureKind {
    /// 太长，或者带着不许的控制字符。
    InvalidRequest,
    /// 登录过期，或者不是房主。
    Forbidden,
    /// 没有这个房间，或者他不在房间里。
    NotFound,
    /// 房间已经归档。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptionFailure {
    operation: &'static str,
    kind: DescriptionFailureKind,
}

impl DescriptionFailure {
    pub const fn new(operation: &'static str, kind: DescriptionFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> DescriptionFailureKind {
        self.kind
    }
}

pub type DescriptionResult<T> = Result<T, DescriptionFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribePrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    /// 原样的简介，去掉首尾空白以后存；空的就是没有简介。
    pub description: String,
}

pub trait PrivateRoomDescriptionUseCases: Send + Sync {
    /// 改好以后的简介。
    fn describe(&self, request: DescribePrivateRoom) -> PortFuture<'_, DescriptionResult<String>>;
}

pub struct PrivateRoomDescriptionDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub descriptions: Arc<dyn PrivateRoomDescriptionStore>,
    pub matrix: Arc<dyn PrivateRoomTopicGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct PrivateRoomDescriptionService {
    rooms: Arc<dyn PrivateRoomStore>,
    descriptions: Arc<dyn PrivateRoomDescriptionStore>,
    matrix: Arc<dyn PrivateRoomTopicGateway>,
    clock: Arc<dyn Clock>,
}

impl PrivateRoomDescriptionService {
    pub fn new(dependencies: PrivateRoomDescriptionDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            descriptions: dependencies.descriptions,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
        }
    }

    async fn describe_internal(&self, request: DescribePrivateRoom) -> DescriptionResult<String> {
        const OPERATION: &str = "private_room.describe";
        let now = self.clock.now();
        if now >= request.actor.expires_at {
            return Err(DescriptionFailure::new(
                OPERATION,
                DescriptionFailureKind::Forbidden,
            ));
        }
        let actor = request.actor.principal_id;
        let snapshot = self
            .rooms
            .find_by_catalog(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .filter(|snapshot| {
                snapshot
                    .room()
                    .member(actor)
                    .is_some_and(PrivateRoomMember::has_joined)
            })
            .ok_or(DescriptionFailure::new(
                OPERATION,
                DescriptionFailureKind::NotFound,
            ))?;
        snapshot
            .room()
            .authorize_description_change(actor)
            .map_err(|error| domain(OPERATION, &error))?;
        let description = request.description.trim().to_owned();
        if description == snapshot.catalog().description() {
            return Ok(description);
        }
        let matrix_room = MatrixRoomId::new(
            snapshot.instance().matrix_room_id().as_str().to_owned(),
        )
        .map_err(|_| DescriptionFailure::new(OPERATION, DescriptionFailureKind::Internal))?;
        let described = snapshot
            .describing(description)
            .map_err(|error| domain(OPERATION, &error))?;
        let description = described.catalog().description();
        self.matrix
            .set_topic(&matrix_room, description)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.descriptions
            .describe(request.catalog_id, description, now)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(description.to_owned())
    }
}

impl PrivateRoomDescriptionUseCases for PrivateRoomDescriptionService {
    fn describe(&self, request: DescribePrivateRoom) -> PortFuture<'_, DescriptionResult<String>> {
        Box::pin(self.describe_internal(request))
    }
}

const fn domain(operation: &'static str, error: &DomainError) -> DescriptionFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => DescriptionFailureKind::Forbidden,
        DomainError::Validation { .. } => DescriptionFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => DescriptionFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => DescriptionFailureKind::Internal,
    };
    DescriptionFailure::new(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> DescriptionFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => DescriptionFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => DescriptionFailureKind::NotFound,
        RepositoryErrorKind::Conflict => DescriptionFailureKind::Conflict,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => DescriptionFailureKind::Internal,
    };
    DescriptionFailure::new(operation, kind)
}

/// Matrix 暂时写不进去的，房主过一会儿再点；别的是配置或数据不对。
const fn matrix(operation: &'static str, error: MatrixFailure) -> DescriptionFailure {
    let kind = match error.kind() {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => DescriptionFailureKind::DependencyUnavailable,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => DescriptionFailureKind::Internal,
    };
    DescriptionFailure::new(operation, kind)
}
