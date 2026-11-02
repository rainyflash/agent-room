//! 进公开房间（`specs/public-rooms/design.md`）：直接进的房间点一下就进来，要申请的房间留一句话
//! 等房主或管理员批准，都拿房主定的默认权限。和邀请链接同一条路：服务器以房主的名义邀请他、在
//! Matrix 上以应用服务发邀请，网页再照接受邀请那一套自己加入、确认。
//!
//! 不公开的、归档了的、没有的房间一律回答“没有这个房间”，免得说出改回私密的房间还在。已经在
//! 房间里（受邀或已加入）的照实说，不再邀请一次。

use std::sync::Arc;

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus},
    room_join_requests::JoinRequestStatus,
    room_listing::RoomJoinMode,
    time::UtcMillis,
};

use super::{InviteByLink, PrivateRoomFailureKind, PrivateRoomLinkInvitations};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, JoinRequestSweep, PendingJoinRequest, PortFuture, PrivateRoomJoinRequestStore,
        PrivateRoomListingRecord, PrivateRoomListingStore, PrivateRoomSnapshot, PrivateRoomStore,
    },
};

mod requests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicJoinFailureKind {
    /// 申请留言不对：太长，或者带着控制字符。
    InvalidRequest,
    /// 登录过期，或者没有管理权限。
    Forbidden,
    /// 没有这个房间，或者它不公开、归档了；处理申请时是没有这个人在等的申请。
    NotFound,
    /// 被这个房间封禁了。
    Banned,
    /// 这间房要申请，管理员批准了才进得来。
    ApprovalRequired,
    /// 这间房只能由房主和管理员邀请。
    InviteOnly,
    /// 被拒还不到 24 小时，见 [`PublicJoinFailure::retry_at`]。
    RecentlyRejected,
    /// 房间刚刚变了（比如改回私密、归档），或者申请刚被别人处理了，再试一次。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicJoinFailure {
    operation: &'static str,
    kind: PublicJoinFailureKind,
    retry_at: Option<UtcMillis>,
}

impl PublicJoinFailure {
    pub const fn new(operation: &'static str, kind: PublicJoinFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_at: None,
        }
    }

    pub const fn recently_rejected(operation: &'static str, retry_at: UtcMillis) -> Self {
        Self {
            operation,
            kind: PublicJoinFailureKind::RecentlyRejected,
            retry_at: Some(retry_at),
        }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PublicJoinFailureKind {
        self.kind
    }

    /// 被拒以后什么时候能再申请。
    pub const fn retry_at(self) -> Option<UtcMillis> {
        self.retry_at
    }
}

pub type PublicJoinResult<T> = Result<T, PublicJoinFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPublicRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

/// 申请进要申请的房间。`message` 是留的那句话，可以是空的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestToJoin {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub message: String,
}

/// 房主或管理员处理一个人的申请。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerJoinRequest {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub requester: PrincipalId,
    pub approve: bool,
}

/// 进来以后他在房间里的处境。受邀的网页接着在 Matrix 上加入、确认；已加入的直接打开房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicJoinMembership {
    Invited,
    Joined,
}

/// 申请以后的处境：在等批准；或者房间刚改成直接进、他本来就在里面，不用等。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRequestState {
    Pending { requested_at: UtcMillis },
    Member(PublicJoinMembership),
}

pub trait PublicRoomJoinUseCases: Send + Sync {
    fn join(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<PublicJoinMembership>>;

    /// 申请加入。已经有一个在等的就原样回答那一个，留言不改。
    fn request(&self, request: RequestToJoin)
    -> PortFuture<'_, PublicJoinResult<JoinRequestState>>;

    /// 撤回自己在等的申请，没有也算成功。
    fn withdraw(&self, request: JoinPublicRoom) -> PortFuture<'_, PublicJoinResult<()>>;

    /// 房主和管理员看这间房在等的申请。
    fn pending_requests(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<Vec<PendingJoinRequest>>>;

    /// 批准就以房主的名义邀请他，拒绝的 24 小时内不能再申请。
    fn answer(
        &self,
        request: AnswerJoinRequest,
    ) -> PortFuture<'_, PublicJoinResult<JoinRequestStatus>>;
}

/// 控制面定时清理申请：不公开、归档了的房间里在等的作废，处理完 30 天的删掉。
pub trait JoinRequestCleanupUseCases: Send + Sync {
    fn sweep(&self) -> PortFuture<'_, PublicJoinResult<JoinRequestSweep>>;
}

pub struct PublicRoomJoinDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub listings: Arc<dyn PrivateRoomListingStore>,
    pub requests: Arc<dyn PrivateRoomJoinRequestStore>,
    /// 以房主的名义邀请：和邀请链接以创建人的名义邀请是同一个入口。
    pub invitations: Arc<dyn PrivateRoomLinkInvitations>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomJoinService {
    rooms: Arc<dyn PrivateRoomStore>,
    listings: Arc<dyn PrivateRoomListingStore>,
    requests: Arc<dyn PrivateRoomJoinRequestStore>,
    invitations: Arc<dyn PrivateRoomLinkInvitations>,
    clock: Arc<dyn Clock>,
}

impl PublicRoomJoinService {
    pub fn new(dependencies: PublicRoomJoinDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            listings: dependencies.listings,
            requests: dependencies.requests,
            invitations: dependencies.invitations,
            clock: dependencies.clock,
        }
    }

    async fn join_internal(
        &self,
        request: JoinPublicRoom,
    ) -> PublicJoinResult<PublicJoinMembership> {
        const OPERATION: &str = "private_room.public_join";
        self.ensure_session(&request.actor, OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let principal_id = request.actor.principal_id;
        if let Some(membership) = current_membership(&snapshot, principal_id) {
            return Ok(membership);
        }
        let listing = self.listed(&snapshot, OPERATION).await?;
        ensure_not_banned(&snapshot, principal_id, OPERATION)?;
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
        self.invite_as_owner(&snapshot, &listing, principal_id, OPERATION)
            .await
    }

    /// 以房主的名义邀请他，权限用公开设置里的默认权限。
    async fn invite_as_owner(
        &self,
        snapshot: &PrivateRoomSnapshot,
        listing: &PrivateRoomListingRecord,
        principal_id: PrincipalId,
        operation: &'static str,
    ) -> PublicJoinResult<PublicJoinMembership> {
        self.invitations
            .invite_by_link(InviteByLink {
                catalog_id: snapshot.catalog().id(),
                link_creator: snapshot.room().owner_principal_id(),
                target_principal_id: principal_id,
                permissions: listing.settings.member_permissions(),
            })
            .await
            .map_err(|failure| {
                PublicJoinFailure::new(operation, invitation_failure(failure.kind()))
            })?;
        Ok(PublicJoinMembership::Invited)
    }

    fn ensure_session(
        &self,
        actor: &AuthenticatedPrincipal,
        operation: &'static str,
    ) -> PublicJoinResult<()> {
        if self.clock.now() >= actor.expires_at {
            Err(PublicJoinFailure::new(
                operation,
                PublicJoinFailureKind::Forbidden,
            ))
        } else {
            Ok(())
        }
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

    fn request(
        &self,
        request: RequestToJoin,
    ) -> PortFuture<'_, PublicJoinResult<JoinRequestState>> {
        Box::pin(self.request_internal(request))
    }

    fn withdraw(&self, request: JoinPublicRoom) -> PortFuture<'_, PublicJoinResult<()>> {
        Box::pin(self.withdraw_internal(request))
    }

    fn pending_requests(
        &self,
        request: JoinPublicRoom,
    ) -> PortFuture<'_, PublicJoinResult<Vec<PendingJoinRequest>>> {
        Box::pin(self.pending_requests_internal(request))
    }

    fn answer(
        &self,
        request: AnswerJoinRequest,
    ) -> PortFuture<'_, PublicJoinResult<JoinRequestStatus>> {
        Box::pin(self.answer_internal(request))
    }
}

impl JoinRequestCleanupUseCases for PublicRoomJoinService {
    fn sweep(&self) -> PortFuture<'_, PublicJoinResult<JoinRequestSweep>> {
        Box::pin(self.sweep_internal())
    }
}

/// 他已经在房间里了（受邀或已加入）。
fn current_membership(
    snapshot: &PrivateRoomSnapshot,
    principal_id: PrincipalId,
) -> Option<PublicJoinMembership> {
    match snapshot
        .room()
        .member(principal_id)
        .map(PrivateRoomMember::status)
    {
        Some(PrivateRoomMembershipStatus::Joined) => Some(PublicJoinMembership::Joined),
        Some(PrivateRoomMembershipStatus::Invited) => Some(PublicJoinMembership::Invited),
        _ => None,
    }
}

fn ensure_not_banned(
    snapshot: &PrivateRoomSnapshot,
    principal_id: PrincipalId,
    operation: &'static str,
) -> PublicJoinResult<()> {
    let banned = snapshot
        .room()
        .member(principal_id)
        .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned);
    if banned {
        Err(PublicJoinFailure::new(
            operation,
            PublicJoinFailureKind::Banned,
        ))
    } else {
        Ok(())
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
