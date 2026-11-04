//! 平台把公开房间从目录里拿掉（`specs/public-rooms/design.md` 的 3b-4b）：平台不审消息，但出了违法的
//! 事得有人能处理。只有平台治理的人（`moderation_operator` 里的 `moderator`），还要最近登录过，和别的
//! 治理一样。
//!
//! 房主的“公开”设置不动，拿掉期间房主改设置也回不到目录里；发现页、详情、分享页都不列，也不能直接
//! 进、申请，在等的申请一起作废。放回去以后照房主的设置来。没公开的房间也能先拿掉，之后公开了也不进
//! 目录。

use std::sync::Arc;

use agent_room_domain::{
    ids::RoomCatalogId, moderation::ModerationRole, private_rooms::PrivateRoomLifecycleStatus,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, ModerationAuthority, PortFuture, PrivateRoomJoinRequestStore, PrivateRoomStore,
        PublicRoomPlatformHold, PublicRoomPlatformHoldStore,
    },
};

/// 原因最多几个字。
pub const MAX_PLATFORM_HOLD_REASON_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformHoldFailureKind {
    /// 原因太长，或者带着换行以外的控制字符。
    InvalidRequest,
    /// 登录过期，不是平台治理的人。
    Forbidden,
    /// 要最近登录过。
    ReauthenticationRequired,
    /// 没有这个房间，或者房间已经归档。
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformHoldFailure {
    operation: &'static str,
    kind: PlatformHoldFailureKind,
}

impl PlatformHoldFailure {
    pub const fn new(operation: &'static str, kind: PlatformHoldFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PlatformHoldFailureKind {
        self.kind
    }
}

pub type PlatformHoldResult<T> = Result<T, PlatformHoldFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldPublicRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    /// 写给房主看的原因，去掉首尾空白以后存；可以空着。
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePublicRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

pub trait PublicRoomPlatformHoldUseCases: Send + Sync {
    fn hold(
        &self,
        request: HoldPublicRoom,
    ) -> PortFuture<'_, PlatformHoldResult<PublicRoomPlatformHold>>;

    /// 放回目录。本来就没拿掉的也算放回去了。
    fn release(&self, request: ReleasePublicRoom) -> PortFuture<'_, PlatformHoldResult<()>>;
}

pub struct PublicRoomPlatformHoldDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub holds: Arc<dyn PublicRoomPlatformHoldStore>,
    pub authority: Arc<dyn ModerationAuthority>,
    /// 拿掉时作废在等的申请。
    pub requests: Arc<dyn PrivateRoomJoinRequestStore>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomPlatformHoldService {
    rooms: Arc<dyn PrivateRoomStore>,
    holds: Arc<dyn PublicRoomPlatformHoldStore>,
    authority: Arc<dyn ModerationAuthority>,
    requests: Arc<dyn PrivateRoomJoinRequestStore>,
    clock: Arc<dyn Clock>,
}

impl PublicRoomPlatformHoldService {
    pub fn new(dependencies: PublicRoomPlatformHoldDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            holds: dependencies.holds,
            authority: dependencies.authority,
            requests: dependencies.requests,
            clock: dependencies.clock,
        }
    }

    async fn hold_internal(
        &self,
        request: HoldPublicRoom,
    ) -> PlatformHoldResult<PublicRoomPlatformHold> {
        const OPERATION: &str = "public_room.platform_hold.place";
        let reason = request.reason.trim();
        if reason.chars().count() > MAX_PLATFORM_HOLD_REASON_CHARS
            || reason
                .chars()
                .any(|character| character.is_control() && character != '\n')
        {
            return Err(PlatformHoldFailure::new(
                OPERATION,
                PlatformHoldFailureKind::InvalidRequest,
            ));
        }
        self.authorize(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let now = self.clock.now();
        let hold = PublicRoomPlatformHold {
            catalog_id: request.catalog_id,
            held_by: request.actor.principal_id,
            reason: reason.to_owned(),
            held_at: now,
        };
        self.holds
            .place_platform_hold(&hold)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        self.requests
            .void_pending_join_requests(request.catalog_id, now)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(hold)
    }

    async fn release_internal(&self, request: ReleasePublicRoom) -> PlatformHoldResult<()> {
        const OPERATION: &str = "public_room.platform_hold.release";
        self.authorize(&request.actor, request.catalog_id, OPERATION)
            .await?;
        self.holds
            .release_platform_hold(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    /// 平台治理的人、最近登录过；房间还在使用中。
    async fn authorize(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PlatformHoldResult<()> {
        if self.clock.now() >= actor.expires_at {
            return Err(PlatformHoldFailure::new(
                operation,
                PlatformHoldFailureKind::Forbidden,
            ));
        }
        if !actor.recently_authenticated {
            return Err(PlatformHoldFailure::new(
                operation,
                PlatformHoldFailureKind::ReauthenticationRequired,
            ));
        }
        let role = self
            .authority
            .platform_role(actor.principal_id)
            .await
            .map_err(|error| repository(operation, &error))?;
        if role != ModerationRole::PlatformModerator {
            return Err(PlatformHoldFailure::new(
                operation,
                PlatformHoldFailureKind::Forbidden,
            ));
        }
        self.rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .filter(|snapshot| snapshot.room().status() == PrivateRoomLifecycleStatus::Active)
            .map(|_| ())
            .ok_or(PlatformHoldFailure::new(
                operation,
                PlatformHoldFailureKind::NotFound,
            ))
    }
}

impl PublicRoomPlatformHoldUseCases for PublicRoomPlatformHoldService {
    fn hold(
        &self,
        request: HoldPublicRoom,
    ) -> PortFuture<'_, PlatformHoldResult<PublicRoomPlatformHold>> {
        Box::pin(self.hold_internal(request))
    }

    fn release(&self, request: ReleasePublicRoom) -> PortFuture<'_, PlatformHoldResult<()>> {
        Box::pin(self.release_internal(request))
    }
}

const fn repository(operation: &'static str, error: &RepositoryError) -> PlatformHoldFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => PlatformHoldFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => PlatformHoldFailureKind::NotFound,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => PlatformHoldFailureKind::Internal,
    };
    PlatformHoldFailure::new(operation, kind)
}
