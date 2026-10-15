//! 私人房间的邀请链接（`specs/room-people/design.md`）：有邀请权限的成员生成、再看、停用；拿到链接的
//! 人登录后先看一眼是哪个房间、谁邀请的，再用它进来。用链接进来时以创建人的名义做一次普通的邀请，
//! 之后网页照接受邀请那一套在 Matrix 上加入再确认。

use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoomMember, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    room_invites::{PrivateRoomInviteSecret, invite_link_expires_at},
    time::UtcMillis,
};

use super::{InviteByLink, PrivateRoomFailureKind, PrivateRoomLinkInvitations};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, JoinCodeAttemptPolicy, JoinCodeSealer, PortFuture, PrivateRoomInviteLinkRecord,
        PrivateRoomInviteLinkStore, PrivateRoomSnapshot, PrivateRoomStore, RoomPeopleDirectory,
        SecretFactory,
    },
    room_people::shown_display_name,
};

/// 封存时加在秘密前面，和 Agent 口令共用一把封存密钥时，两种密文挪来挪去都解不对。
const SEALED_PREFIX: &[u8] = b"invite-link\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteLinkFailureKind {
    InvalidRequest,
    /// 登录过期，或者没有邀请权限。
    Forbidden,
    /// 没有这条链接，或者它不管用了：过期、停用、房间归档、创建人不再能邀请。
    NotFound,
    /// 拿着链接的人被这个房间封禁了。
    Banned,
    /// 房间有链接，可这台服务器没配封存密钥，看不到了。
    NotRevealable,
    RateLimited,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteLinkFailure {
    operation: &'static str,
    kind: InviteLinkFailureKind,
    retry_at: Option<UtcMillis>,
}

impl InviteLinkFailure {
    pub const fn new(operation: &'static str, kind: InviteLinkFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_at: None,
        }
    }

    pub const fn rate_limited(operation: &'static str, retry_at: UtcMillis) -> Self {
        Self {
            operation,
            kind: InviteLinkFailureKind::RateLimited,
            retry_at: Some(retry_at),
        }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> InviteLinkFailureKind {
        self.kind
    }

    pub const fn retry_at(self) -> Option<UtcMillis> {
        self.retry_at
    }
}

pub type InviteLinkResult<T> = Result<T, InviteLinkFailure>;

/// 生成或换一条链接。`permissions` 为空时给默认的（查看、发言、自动发言，不超过自己有的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateInviteLink {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub permissions: Option<PrivateRoomPermissions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManageInviteLink {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

/// 拿着链接的人：看一眼，或者用它进来。`secret` 是网页从链接 `#` 后面读出来的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseInviteLink {
    pub actor: AuthenticatedPrincipal,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub secret: PrivateRoomInviteSecret,
    pub record: PrivateRoomInviteLinkRecord,
}

/// 拿着链接的人此刻在这个房间里是什么情况。被封禁的进不来，单独报错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteeMembership {
    /// 不在房间里（没来过、拒绝过或被移出过），用链接能进来。
    Absent,
    /// 已经受邀，接受邀请就进来了。
    Invited,
    /// 已经在房间里，直接打开房间。
    Joined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitePreview {
    pub catalog_id: RoomCatalogId,
    pub room_name: String,
    pub inviter_display_name: String,
    pub permissions: PrivateRoomPermissions,
    pub expires_at: UtcMillis,
    pub membership: InviteeMembership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedeemedInvite {
    pub catalog_id: RoomCatalogId,
    pub membership: InviteeMembership,
}

pub trait PrivateRoomInviteLinkUseCases: Send + Sync {
    fn create(&self, request: CreateInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>>;

    /// 再看当前的链接，给有邀请权限的人复制。过期或不管用了的当作没有。
    fn reveal(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>>;

    fn disable(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<()>>;

    fn preview(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<InvitePreview>>;

    /// 用链接进来：还不在房间里的，以创建人的名义邀请他；已经受邀或已加入的什么都不改。
    fn redeem(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<RedeemedInvite>>;
}

pub struct PrivateRoomInviteLinkDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub links: Arc<dyn PrivateRoomInviteLinkStore>,
    pub invitations: Arc<dyn PrivateRoomLinkInvitations>,
    pub people: Arc<dyn RoomPeopleDirectory>,
    pub secrets: Arc<dyn SecretFactory>,
    pub sealer: Arc<dyn JoinCodeSealer>,
    pub clock: Arc<dyn Clock>,
    /// 猜链接和猜 Agent 口令用同一套限额，按账号计数。
    pub attempts: JoinCodeAttemptPolicy,
}

pub struct PrivateRoomInviteLinkService {
    rooms: Arc<dyn PrivateRoomStore>,
    links: Arc<dyn PrivateRoomInviteLinkStore>,
    invitations: Arc<dyn PrivateRoomLinkInvitations>,
    people: Arc<dyn RoomPeopleDirectory>,
    secrets: Arc<dyn SecretFactory>,
    sealer: Arc<dyn JoinCodeSealer>,
    clock: Arc<dyn Clock>,
    attempts: JoinCodeAttemptPolicy,
}

impl PrivateRoomInviteLinkService {
    pub fn new(dependencies: PrivateRoomInviteLinkDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            links: dependencies.links,
            invitations: dependencies.invitations,
            people: dependencies.people,
            secrets: dependencies.secrets,
            sealer: dependencies.sealer,
            clock: dependencies.clock,
            attempts: dependencies.attempts,
        }
    }

    async fn create_internal(&self, request: CreateInviteLink) -> InviteLinkResult<InviteLink> {
        const OPERATION: &str = "private_room.invite_link.create";
        let actor = request.actor.principal_id;
        let snapshot = self
            .visible_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let room = snapshot.room();
        let permissions = request
            .permissions
            .unwrap_or_else(|| room.default_invitation_for(actor));
        room.authorize_invite_link_permissions(actor, permissions)
            .map_err(|error| domain(OPERATION, &error))?;
        let random = self.secrets.generate().map_err(|_| {
            InviteLinkFailure::new(OPERATION, InviteLinkFailureKind::DependencyUnavailable)
        })?;
        let mut entropy = [0_u8; 16];
        entropy.copy_from_slice(&self.secrets.digest(random.expose()).as_bytes()[..16]);
        let secret = PrivateRoomInviteSecret::from_entropy(entropy);
        let now = self.clock.now();
        let record = PrivateRoomInviteLinkRecord {
            catalog_id: request.catalog_id,
            permissions,
            created_by: actor,
            created_at: now,
            expires_at: invite_link_expires_at(now).map_err(|error| domain(OPERATION, &error))?,
        };
        // 封存不了（没配封存密钥）时照样生成，只是以后看不到。
        let sealed = self
            .sealer
            .seal(request.catalog_id, &sealed_plaintext(&secret))
            .ok();
        self.links
            .replace_invite_link(
                &record,
                &self.secrets.digest(secret.as_str()),
                sealed.as_ref(),
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(InviteLink { secret, record })
    }

    async fn reveal_internal(&self, request: ManageInviteLink) -> InviteLinkResult<InviteLink> {
        const OPERATION: &str = "private_room.invite_link.reveal";
        let snapshot = self
            .managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let stored = self
            .links
            .invite_link(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .filter(|stored| self.honored(&snapshot, &stored.record))
            .ok_or_else(|| InviteLinkFailure::new(OPERATION, InviteLinkFailureKind::NotFound))?;
        let not_revealable =
            || InviteLinkFailure::new(OPERATION, InviteLinkFailureKind::NotRevealable);
        let sealed = stored.sealed.as_ref().ok_or_else(not_revealable)?;
        let opened = self
            .sealer
            .open(request.catalog_id, sealed)
            .map_err(|_| not_revealable())?;
        let secret = opened
            .strip_prefix(SEALED_PREFIX)
            .and_then(|text| std::str::from_utf8(text).ok())
            .and_then(|text| PrivateRoomInviteSecret::parse(text).ok())
            .filter(|secret| self.secrets.digest(secret.as_str()) == stored.digest)
            .ok_or_else(not_revealable)?;
        Ok(InviteLink {
            secret,
            record: stored.record,
        })
    }

    async fn disable_internal(&self, request: ManageInviteLink) -> InviteLinkResult<()> {
        const OPERATION: &str = "private_room.invite_link.disable";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        self.links
            .clear_invite_link(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    async fn preview_internal(&self, request: UseInviteLink) -> InviteLinkResult<InvitePreview> {
        const OPERATION: &str = "private_room.invite_link.preview";
        let (record, snapshot) = self.checked_link(&request, OPERATION).await?;
        let membership = invitee_membership(&snapshot, request.actor.principal_id, OPERATION)?;
        let inviter_display_name = self
            .people
            .member_names(record.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .into_iter()
            .find(|person| person.principal_id == record.created_by)
            .map(|person| shown_display_name(&person.display_name))
            .ok_or_else(|| InviteLinkFailure::new(OPERATION, InviteLinkFailureKind::NotFound))?;
        Ok(InvitePreview {
            catalog_id: record.catalog_id,
            room_name: snapshot.catalog().name().to_owned(),
            inviter_display_name,
            permissions: record.permissions,
            expires_at: record.expires_at,
            membership,
        })
    }

    async fn redeem_internal(&self, request: UseInviteLink) -> InviteLinkResult<RedeemedInvite> {
        const OPERATION: &str = "private_room.invite_link.redeem";
        let (record, snapshot) = self.checked_link(&request, OPERATION).await?;
        let membership = invitee_membership(&snapshot, request.actor.principal_id, OPERATION)?;
        if membership != InviteeMembership::Absent {
            return Ok(RedeemedInvite {
                catalog_id: record.catalog_id,
                membership,
            });
        }
        self.invitations
            .invite_by_link(InviteByLink {
                catalog_id: record.catalog_id,
                link_creator: record.created_by,
                target_principal_id: request.actor.principal_id,
                permissions: record.permissions,
            })
            .await
            .map_err(|failure| {
                let kind = match failure.kind() {
                    // 刚刚检查过以后创建人没了邀请权限、房间归档了：链接不管用了。
                    PrivateRoomFailureKind::Forbidden | PrivateRoomFailureKind::NotFound => {
                        InviteLinkFailureKind::NotFound
                    }
                    PrivateRoomFailureKind::Conflict => InviteLinkFailureKind::Conflict,
                    PrivateRoomFailureKind::DependencyUnavailable
                    | PrivateRoomFailureKind::UnknownCommit => {
                        InviteLinkFailureKind::DependencyUnavailable
                    }
                    PrivateRoomFailureKind::InvalidRequest | PrivateRoomFailureKind::Internal => {
                        InviteLinkFailureKind::Internal
                    }
                };
                InviteLinkFailure::new(OPERATION, kind)
            })?;
        Ok(RedeemedInvite {
            catalog_id: record.catalog_id,
            membership: InviteeMembership::Invited,
        })
    }

    /// 登录有效、没被限流、秘密格式对且有这条链接、链接还管用。格式不对多半是链接没复制全，不算
    /// 一次猜测；格式对但没有这条才计入这个账号的失败次数。
    async fn checked_link(
        &self,
        request: &UseInviteLink,
        operation: &'static str,
    ) -> InviteLinkResult<(PrivateRoomInviteLinkRecord, PrivateRoomSnapshot)> {
        let now = self.clock.now();
        if request.actor.expires_at <= now {
            return Err(InviteLinkFailure::new(
                operation,
                InviteLinkFailureKind::Forbidden,
            ));
        }
        let caller = format!("principal:{}", request.actor.principal_id);
        if let Some(retry_at) = self
            .links
            .invite_link_retry_at(&caller, now, self.attempts)
            .await
            .map_err(|error| repository(operation, &error))?
        {
            return Err(InviteLinkFailure::rate_limited(operation, retry_at));
        }
        let secret = PrivateRoomInviteSecret::parse(&request.secret).map_err(|_| {
            InviteLinkFailure::new(operation, InviteLinkFailureKind::InvalidRequest)
        })?;
        let not_found = || InviteLinkFailure::new(operation, InviteLinkFailureKind::NotFound);
        let Some(record) = self
            .links
            .find_invite_link(&self.secrets.digest(secret.as_str()))
            .await
            .map_err(|error| repository(operation, &error))?
        else {
            self.links
                .record_invite_link_failure(&caller, now, self.attempts)
                .await
                .map_err(|error| repository(operation, &error))?;
            return Err(not_found());
        };
        let snapshot = self
            .rooms
            .find_by_catalog(record.catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(not_found)?;
        if !self.honored(&snapshot, &record) {
            return Err(not_found());
        }
        Ok((record, snapshot))
    }

    /// 链接还管不管用：没过期，房间还在使用中，创建人还在房间里、能邀请，给的权限还在他的权限以内。
    fn honored(
        &self,
        snapshot: &PrivateRoomSnapshot,
        record: &PrivateRoomInviteLinkRecord,
    ) -> bool {
        self.clock.now() < record.expires_at
            && snapshot
                .room()
                .authorize_invite_link_permissions(record.created_by, record.permissions)
                .is_ok()
    }

    /// 操作者登录有效，看得见这个房间（受邀或已加入）。看不见的和没有这个房间一样回答。
    async fn visible_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> InviteLinkResult<PrivateRoomSnapshot> {
        if self.clock.now() >= actor.expires_at {
            return Err(InviteLinkFailure::new(
                operation,
                InviteLinkFailureKind::Forbidden,
            ));
        }
        let not_found = || InviteLinkFailure::new(operation, InviteLinkFailureKind::NotFound);
        let snapshot = self
            .rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(not_found)?;
        let visible = snapshot
            .room()
            .member(actor.principal_id)
            .is_some_and(|member| {
                matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
            });
        if visible {
            Ok(snapshot)
        } else {
            Err(not_found())
        }
    }

    /// 在 [`Self::visible_room`] 之上，操作者能管邀请链接。
    async fn managed_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> InviteLinkResult<PrivateRoomSnapshot> {
        let snapshot = self.visible_room(actor, catalog_id, operation).await?;
        snapshot
            .room()
            .authorize_invite_link(actor.principal_id)
            .map_err(|error| domain(operation, &error))?;
        Ok(snapshot)
    }
}

impl PrivateRoomInviteLinkUseCases for PrivateRoomInviteLinkService {
    fn create(&self, request: CreateInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>> {
        Box::pin(self.create_internal(request))
    }

    fn reveal(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>> {
        Box::pin(self.reveal_internal(request))
    }

    fn disable(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<()>> {
        Box::pin(self.disable_internal(request))
    }

    fn preview(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<InvitePreview>> {
        Box::pin(self.preview_internal(request))
    }

    fn redeem(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<RedeemedInvite>> {
        Box::pin(self.redeem_internal(request))
    }
}

fn sealed_plaintext(secret: &PrivateRoomInviteSecret) -> Vec<u8> {
    let mut plaintext = SEALED_PREFIX.to_vec();
    plaintext.extend_from_slice(secret.as_str().as_bytes());
    plaintext
}

fn invitee_membership(
    snapshot: &PrivateRoomSnapshot,
    principal_id: PrincipalId,
    operation: &'static str,
) -> InviteLinkResult<InviteeMembership> {
    match snapshot
        .room()
        .member(principal_id)
        .map(PrivateRoomMember::status)
    {
        Some(PrivateRoomMembershipStatus::Joined) => Ok(InviteeMembership::Joined),
        Some(PrivateRoomMembershipStatus::Invited) => Ok(InviteeMembership::Invited),
        Some(PrivateRoomMembershipStatus::Banned) => Err(InviteLinkFailure::new(
            operation,
            InviteLinkFailureKind::Banned,
        )),
        Some(PrivateRoomMembershipStatus::Declined | PrivateRoomMembershipStatus::Removed)
        | None => Ok(InviteeMembership::Absent),
    }
}

const fn domain(operation: &'static str, error: &DomainError) -> InviteLinkFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => InviteLinkFailureKind::Forbidden,
        DomainError::Validation { .. } => InviteLinkFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => InviteLinkFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => InviteLinkFailureKind::Internal,
    };
    InviteLinkFailure::new(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> InviteLinkFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => InviteLinkFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => InviteLinkFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => InviteLinkFailureKind::NotFound,
        RepositoryErrorKind::Conflict => InviteLinkFailureKind::Conflict,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            InviteLinkFailureKind::Internal
        }
    };
    InviteLinkFailure::new(operation, kind)
}
