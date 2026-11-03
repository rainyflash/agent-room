//! 房主和管理员改房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：查“管理”这一位、
//! 房间在使用中，然后以应用服务把 `room.notice.v1` 写进 Matrix。不另记库：读的一边读房间状态。
//!
//! 每写一次就是一条新的状态事件，聊天服务器不删，应用服务又不受它的限速，所以每个人一小时最多写
//! [`NOTICES_PER_HOUR`] 次。

use std::sync::Arc;

use agent_room_domain::{
    DomainError, ids::RoomCatalogId, private_rooms::PrivateRoomMember,
    room_notices::RoomNoticeText, time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    hourly_limit::HourlyLimit,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixRoomId, MatrixUserId, PortFuture,
        PrivateRoomNoticeGateway, PrivateRoomStore, RoomNoticeWrite,
    },
};

/// 每个人一小时里最多改几次公告。日常改几个字够用，又挡住拿它刷状态事件。
pub const NOTICES_PER_HOUR: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeFailureKind {
    /// 太长，或者带着不许的控制字符。
    InvalidRequest,
    /// 登录过期，或者不是房主、管理员。
    Forbidden,
    /// 没有这个房间，或者他不在房间里。
    NotFound,
    /// 房间已经归档。
    Conflict,
    /// 这一小时改得太多了：`retry_at` 之后再来。
    RateLimited {
        retry_at: UtcMillis,
    },
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoticeFailure {
    operation: &'static str,
    kind: NoticeFailureKind,
}

impl NoticeFailure {
    pub const fn new(operation: &'static str, kind: NoticeFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> NoticeFailureKind {
        self.kind
    }
}

pub type NoticeResult<T> = Result<T, NoticeFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRoomNotice {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    /// 原样的公告，去掉首尾空白以后写；空的就是撤掉。
    pub text: String,
}

/// 写进去的那一版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomNotice {
    pub text: String,
    pub updated_at: UtcMillis,
}

pub trait PrivateRoomNoticeUseCases: Send + Sync {
    fn update_notice(&self, request: UpdateRoomNotice) -> PortFuture<'_, NoticeResult<RoomNotice>>;
}

pub struct PrivateRoomNoticeDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub matrix: Arc<dyn PrivateRoomNoticeGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct PrivateRoomNoticeService {
    rooms: Arc<dyn PrivateRoomStore>,
    matrix: Arc<dyn PrivateRoomNoticeGateway>,
    clock: Arc<dyn Clock>,
    written: HourlyLimit,
}

impl PrivateRoomNoticeService {
    pub fn new(dependencies: PrivateRoomNoticeDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
            written: HourlyLimit::new(NOTICES_PER_HOUR),
        }
    }

    async fn update_internal(&self, request: UpdateRoomNotice) -> NoticeResult<RoomNotice> {
        const OPERATION: &str = "private_room.update_notice";
        let now = self.clock.now();
        if now >= request.actor.expires_at {
            return Err(NoticeFailure::new(OPERATION, NoticeFailureKind::Forbidden));
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
            .ok_or(NoticeFailure::new(OPERATION, NoticeFailureKind::NotFound))?;
        snapshot
            .room()
            .authorize_notice_change(actor)
            .map_err(|error| domain(OPERATION, &error))?;
        let text =
            RoomNoticeText::parse(&request.text).map_err(|error| domain(OPERATION, &error))?;
        let internal = || NoticeFailure::new(OPERATION, NoticeFailureKind::Internal);
        let matrix_room =
            MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
                .map_err(|_| internal())?;
        let updated_by =
            MatrixUserId::new(request.actor.matrix_user_id.clone()).map_err(|_| internal())?;
        self.written.take(actor, now).map_err(|reached| {
            NoticeFailure::new(
                OPERATION,
                NoticeFailureKind::RateLimited {
                    retry_at: reached.retry_at,
                },
            )
        })?;
        let notice = RoomNoticeWrite {
            text,
            updated_by,
            updated_at: now,
        };
        self.matrix
            .set_notice(&matrix_room, &notice)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        Ok(RoomNotice {
            text: notice.text.as_str().to_owned(),
            updated_at: now,
        })
    }
}

impl PrivateRoomNoticeUseCases for PrivateRoomNoticeService {
    fn update_notice(&self, request: UpdateRoomNotice) -> PortFuture<'_, NoticeResult<RoomNotice>> {
        Box::pin(self.update_internal(request))
    }
}

const fn domain(operation: &'static str, error: &DomainError) -> NoticeFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => NoticeFailureKind::Forbidden,
        DomainError::Validation { .. } => NoticeFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => NoticeFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => NoticeFailureKind::Internal,
    };
    NoticeFailure::new(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> NoticeFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => NoticeFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => NoticeFailureKind::NotFound,
        RepositoryErrorKind::Conflict => NoticeFailureKind::Conflict,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => NoticeFailureKind::Internal,
    };
    NoticeFailure::new(operation, kind)
}

/// Matrix 暂时写不进去的，过一会儿再点；别的是配置或数据不对。
const fn matrix(operation: &'static str, error: MatrixFailure) -> NoticeFailure {
    let kind = match error.kind() {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => NoticeFailureKind::DependencyUnavailable,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => NoticeFailureKind::Internal,
    };
    NoticeFailure::new(operation, kind)
}
