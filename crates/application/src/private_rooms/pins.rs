//! 房主和管理员置顶房间里的一句、取消置顶（`specs/redesign/brief-and-pins.md`“置顶”）：查“管理”这一位、
//! 房间在使用中、那一句是这间房里的消息，然后以应用服务改 `m.room.pinned_events`。不另记库：读的一边
//! 读房间状态，对话里“谁置顶了一句”那几行就是记录。
//!
//! 和须知一样每写一次是一条新的状态事件，次数和须知合起来算（[`RoomStateWriteLimit`]）。

use std::sync::Arc;

use agent_room_domain::{
    DomainError, ids::RoomCatalogId, private_rooms::PrivateRoomMember, room_pins::RoomPinEventId,
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixRoomId, MatrixUserId, PinTarget, PortFuture,
        PrivateRoomPinsGateway, PrivateRoomStore, RoomPinsChange, RoomPinsWriteFailure,
    },
};

use super::RoomStateWriteLimit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinsFailureKind {
    /// 事件 ID 的样子不对。
    InvalidRequest,
    /// 那一句不是这间房里的消息（不在这间房里、是状态事件、已经删了）。
    InvalidEvent,
    /// 已经满了，要先取消一条。
    Full,
    /// 登录过期，或者不是房主、管理员。
    Forbidden,
    /// 没有这个房间，或者他不在房间里。
    NotFound,
    /// 房间已经归档。
    Conflict,
    /// 这一小时改得太多了（须知和置顶合起来算）：`retry_at` 之后再来。
    RateLimited {
        retry_at: UtcMillis,
    },
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinsFailure {
    operation: &'static str,
    kind: PinsFailureKind,
}

impl PinsFailure {
    pub const fn new(operation: &'static str, kind: PinsFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PinsFailureKind {
        self.kind
    }
}

pub type PinsResult<T> = Result<T, PinsFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRoomPins {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    /// 那一句在 Matrix 上的事件 ID。
    pub event_id: String,
    /// `true` 置顶，`false` 取消置顶。
    pub pinned: bool,
}

/// 改完以后置顶的一句：哪一句、谁置顶的（Matrix 上的用户 ID）、什么时候。新的在最前面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPinView {
    pub event_id: String,
    pub pinned_by: String,
    pub pinned_at: UtcMillis,
}

pub trait PrivateRoomPinsUseCases: Send + Sync {
    fn update_pins(&self, request: UpdateRoomPins) -> PortFuture<'_, PinsResult<Vec<RoomPinView>>>;
}

pub struct PrivateRoomPinsDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub matrix: Arc<dyn PrivateRoomPinsGateway>,
    pub clock: Arc<dyn Clock>,
    /// 和须知共用的那一份次数上限。
    pub writes: Arc<RoomStateWriteLimit>,
}

pub struct PrivateRoomPinsService {
    rooms: Arc<dyn PrivateRoomStore>,
    matrix: Arc<dyn PrivateRoomPinsGateway>,
    clock: Arc<dyn Clock>,
    writes: Arc<RoomStateWriteLimit>,
}

const OPERATION: &str = "private_room.update_pins";

impl PrivateRoomPinsService {
    pub fn new(dependencies: PrivateRoomPinsDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
            writes: dependencies.writes,
        }
    }

    async fn update_internal(&self, request: UpdateRoomPins) -> PinsResult<Vec<RoomPinView>> {
        let now = self.clock.now();
        if now >= request.actor.expires_at {
            return Err(PinsFailure::new(OPERATION, PinsFailureKind::Forbidden));
        }
        let matrix_room = self.authorized_room(&request).await?;
        let event_id = RoomPinEventId::parse(&request.event_id).map_err(|error| domain(&error))?;
        let actor = MatrixUserId::new(request.actor.matrix_user_id.clone())
            .map_err(|_| PinsFailure::new(OPERATION, PinsFailureKind::Internal))?;
        self.writes
            .take(request.actor.principal_id, now)
            .map_err(|reached| {
                PinsFailure::new(
                    OPERATION,
                    PinsFailureKind::RateLimited {
                        retry_at: reached.retry_at,
                    },
                )
            })?;
        // 取消置顶不问那一句：删了的那句也要能从列表里拿掉。
        if request.pinned
            && self
                .matrix
                .pin_target(&matrix_room, &event_id)
                .await
                .map_err(matrix)?
                == PinTarget::NotMessage
        {
            return Err(PinsFailure::new(OPERATION, PinsFailureKind::InvalidEvent));
        }
        let change = RoomPinsChange {
            event_id,
            pinned: request.pinned,
            actor,
            at: now,
        };
        let pins = self
            .matrix
            .change_pins(&matrix_room, &change)
            .await
            .map_err(|failure| match failure {
                RoomPinsWriteFailure::Full => PinsFailure::new(OPERATION, PinsFailureKind::Full),
                RoomPinsWriteFailure::Matrix(error) => matrix(error),
            })?;
        Ok(pins
            .entries()
            .iter()
            .map(|pin| RoomPinView {
                event_id: pin.event_id.as_str().to_owned(),
                pinned_by: pin.pinned_by.clone(),
                pinned_at: pin.pinned_at,
            })
            .collect())
    }

    /// 他在这间房里、有“管理”这一位、房间在使用中：回这间房在 Matrix 上的房间号。
    async fn authorized_room(&self, request: &UpdateRoomPins) -> PinsResult<MatrixRoomId> {
        let actor = request.actor.principal_id;
        let snapshot = self
            .rooms
            .find_by_catalog(request.catalog_id)
            .await
            .map_err(|error| repository(&error))?
            .filter(|snapshot| {
                snapshot
                    .room()
                    .member(actor)
                    .is_some_and(PrivateRoomMember::has_joined)
            })
            .ok_or(PinsFailure::new(OPERATION, PinsFailureKind::NotFound))?;
        snapshot
            .room()
            .authorize_pins_change(actor)
            .map_err(|error| domain(&error))?;
        MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
            .map_err(|_| PinsFailure::new(OPERATION, PinsFailureKind::Internal))
    }
}

impl PrivateRoomPinsUseCases for PrivateRoomPinsService {
    fn update_pins(&self, request: UpdateRoomPins) -> PortFuture<'_, PinsResult<Vec<RoomPinView>>> {
        Box::pin(self.update_internal(request))
    }
}

const fn domain(error: &DomainError) -> PinsFailure {
    let kind = match error {
        DomainError::Forbidden { .. } | DomainError::OwnerOnly { .. } => PinsFailureKind::Forbidden,
        DomainError::Validation { .. } => PinsFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => PinsFailureKind::Conflict,
        DomainError::CapacityExceeded { .. } => PinsFailureKind::Full,
        DomainError::InvariantViolation { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => PinsFailureKind::Internal,
    };
    PinsFailure::new(OPERATION, kind)
}

const fn repository(error: &RepositoryError) -> PinsFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => PinsFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => PinsFailureKind::NotFound,
        RepositoryErrorKind::Conflict => PinsFailureKind::Conflict,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => PinsFailureKind::Internal,
    };
    PinsFailure::new(OPERATION, kind)
}

/// Matrix 暂时读写不了的，过一会儿再点；别的是配置或数据不对。
const fn matrix(error: MatrixFailure) -> PinsFailure {
    let kind = match error.kind() {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => PinsFailureKind::DependencyUnavailable,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => PinsFailureKind::Internal,
    };
    PinsFailure::new(OPERATION, kind)
}
