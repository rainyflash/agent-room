//! 进公开房间（`specs/public-rooms/design.md`）：直接进的房间点一下就进来，拿到房主定的默认权限。
//! 和邀请链接同一条路：服务器以房主的名义邀请他、在 Matrix 上以应用服务发邀请，网页再照接受邀请
//! 那一套自己加入、确认。
//!
//! 不公开的、归档了的、没有的房间一律回答“没有这个房间”，免得说出改回私密的房间还在。已经在
//! 房间里（受邀或已加入）的照实说，不再邀请一次。

use std::sync::Arc;

use agent_room_domain::{
    ids::RoomCatalogId,
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus},
    room_listing::RoomJoinMode,
};

use super::{InviteByLink, PrivateRoomFailureKind, PrivateRoomLinkInvitations};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, PortFuture, PrivateRoomListingRecord, PrivateRoomListingStore, PrivateRoomSnapshot,
        PrivateRoomStore,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicJoinFailureKind {
    /// 登录过期。
    Forbidden,
    /// 没有这个房间，或者它不公开、归档了。
    NotFound,
    /// 被这个房间封禁了。
    Banned,
    /// 这间房要申请，管理员批准了才进得来。
    ApprovalRequired,
    /// 这间房只能由房主和管理员邀请。
    InviteOnly,
    /// 房间刚刚变了（比如改回私密、归档），再试一次。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicJoinFailure {
    operation: &'static str,
    kind: PublicJoinFailureKind,
}

impl PublicJoinFailure {
    pub const fn new(operation: &'static str, kind: PublicJoinFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PublicJoinFailureKind {
        self.kind
    }
}

pub type PublicJoinResult<T> = Result<T, PublicJoinFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPublicRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

/// 进来以后他在房间里的处境。受邀的网页接着在 Matrix 上加入、确认；已加入的直接打开房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicJoinMembership {
    Invited,
    Joined,
}

pub trait PublicRoomJoinUseCases: Send + Sync {
    fn join(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<PublicJoinMembership>>;
}

pub struct PublicRoomJoinDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub listings: Arc<dyn PrivateRoomListingStore>,
    /// 以房主的名义邀请：和邀请链接以创建人的名义邀请是同一个入口。
    pub invitations: Arc<dyn PrivateRoomLinkInvitations>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomJoinService {
    rooms: Arc<dyn PrivateRoomStore>,
    listings: Arc<dyn PrivateRoomListingStore>,
    invitations: Arc<dyn PrivateRoomLinkInvitations>,
    clock: Arc<dyn Clock>,
}

impl PublicRoomJoinService {
    pub fn new(dependencies: PublicRoomJoinDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            listings: dependencies.listings,
            invitations: dependencies.invitations,
            clock: dependencies.clock,
        }
    }

    async fn join_internal(
        &self,
        request: JoinPublicRoom,
    ) -> PublicJoinResult<PublicJoinMembership> {
        const OPERATION: &str = "private_room.public_join";
        if self.clock.now() >= request.actor.expires_at {
            return Err(PublicJoinFailure::new(
                OPERATION,
                PublicJoinFailureKind::Forbidden,
            ));
        }
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let principal_id = request.actor.principal_id;
        match snapshot
            .room()
            .member(principal_id)
            .map(PrivateRoomMember::status)
        {
            Some(PrivateRoomMembershipStatus::Joined) => return Ok(PublicJoinMembership::Joined),
            Some(PrivateRoomMembershipStatus::Invited) => {
                return Ok(PublicJoinMembership::Invited);
            }
            _ => {}
        }
        let listing = self.listed(&snapshot, OPERATION).await?;
        if snapshot
            .room()
            .member(principal_id)
            .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned)
        {
            return Err(PublicJoinFailure::new(
                OPERATION,
                PublicJoinFailureKind::Banned,
            ));
        }
        match listing.settings.join_mode() {
            RoomJoinMode::Open => {}
            RoomJoinMode::Request => {
                return Err(PublicJoinFailure::new(
                    OPERATION,
                    PublicJoinFailureKind::ApprovalRequired,
                ));
            }
            RoomJoinMode::InviteOnly => {
                return Err(PublicJoinFailure::new(
                    OPERATION,
                    PublicJoinFailureKind::InviteOnly,
                ));
            }
        }
        self.invitations
            .invite_by_link(InviteByLink {
                catalog_id: request.catalog_id,
                link_creator: snapshot.room().owner_principal_id(),
                target_principal_id: principal_id,
                permissions: listing.settings.member_permissions(),
            })
            .await
            .map_err(|failure| {
                PublicJoinFailure::new(OPERATION, invitation_failure(failure.kind()))
            })?;
        Ok(PublicJoinMembership::Invited)
    }

    async fn load(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PublicJoinResult<PrivateRoomSnapshot> {
        self.rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| PublicJoinFailure::new(operation, PublicJoinFailureKind::NotFound))
    }

    /// 房间此刻公开着、还在使用中。
    async fn listed(
        &self,
        snapshot: &PrivateRoomSnapshot,
        operation: &'static str,
    ) -> PublicJoinResult<PrivateRoomListingRecord> {
        let not_found = || PublicJoinFailure::new(operation, PublicJoinFailureKind::NotFound);
        if snapshot.room().status() != PrivateRoomLifecycleStatus::Active {
            return Err(not_found());
        }
        self.listings
            .listing(snapshot.catalog().id())
            .await
            .map_err(|error| repository(operation, &error))?
            .filter(|listing| listing.settings.listed())
            .ok_or_else(not_found)
    }
}

impl PublicRoomJoinUseCases for PublicRoomJoinService {
    fn join(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<PublicJoinMembership>> {
        Box::pin(self.join_internal(request))
    }
}

/// 以房主的名义邀请没成：刚检查过以后房间归档了、房主换了人这类，算房间刚变了。
const fn invitation_failure(kind: PrivateRoomFailureKind) -> PublicJoinFailureKind {
    match kind {
        PrivateRoomFailureKind::Forbidden
        | PrivateRoomFailureKind::NotFound
        | PrivateRoomFailureKind::Conflict => PublicJoinFailureKind::Conflict,
        PrivateRoomFailureKind::DependencyUnavailable | PrivateRoomFailureKind::UnknownCommit => {
            PublicJoinFailureKind::DependencyUnavailable
        }
        PrivateRoomFailureKind::InvalidRequest
        | PrivateRoomFailureKind::PublicRoomLimitReached
        | PrivateRoomFailureKind::Internal => PublicJoinFailureKind::Internal,
    }
}

const fn repository(operation: &'static str, error: &RepositoryError) -> PublicJoinFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => PublicJoinFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => PublicJoinFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => PublicJoinFailureKind::NotFound,
        RepositoryErrorKind::Conflict => PublicJoinFailureKind::Conflict,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            PublicJoinFailureKind::Internal
        }
    };
    PublicJoinFailure::new(operation, kind)
}
