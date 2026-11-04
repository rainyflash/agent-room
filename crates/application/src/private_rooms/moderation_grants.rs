//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员把房间交给
//! 自己的一个 Agent 管。给的时候要最近登录过（这是把权力交出去，和治理一样）；规则 1 到 2000 个字，
//! 能做的几样至少勾一样，默认 30 天、最长 30 天。同一个 Agent 在同一个房间再给一次就盖掉上一份。
//!
//! 看在用的授权要是房主或管理员；撤一份要是房主、管理员，或者给它的那个人（他后来不当管理员了也能撤）。
//! Agent 照授权动手是第 7a-2 步。

use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{AgentId, RoomCatalogId, RoomModerationGrantId},
    private_rooms::PrivateRoomMember,
    room_moderation_grants::{
        RoomModerationPower, RoomModerationPowers, RoomModerationRules,
        room_moderation_grant_lifetime,
    },
    time::DurationMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        ActiveRoomModerationGrant, Clock, PortFuture, PrivateRoomSnapshot, PrivateRoomStore,
        RoomModerationGrant, RoomModerationGrantStore,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationGrantFailureKind {
    /// 规则、勾的几样或者时长不对。
    InvalidRequest,
    /// 登录过期，没有最近登录过，或者不是房主、管理员。
    Forbidden,
    /// 给的不是他自己的、还在用的 Agent。
    NotYourAgent,
    /// 没有这个房间、他不在房间里，或者没有这份授权。
    NotFound,
    /// 房间已经归档。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationGrantFailure {
    operation: &'static str,
    kind: ModerationGrantFailureKind,
}

impl ModerationGrantFailure {
    pub const fn new(operation: &'static str, kind: ModerationGrantFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> ModerationGrantFailureKind {
        self.kind
    }
}

pub type ModerationGrantResult<T> = Result<T, ModerationGrantFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRoomModeration {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub grant_id: RoomModerationGrantId,
    pub agent_id: AgentId,
    /// 原样的规则，去掉首尾空白以后存。
    pub rules: String,
    pub powers: Vec<RoomModerationPower>,
    /// 不填是 30 天。
    pub lifetime: Option<DurationMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRoomModerationGrants {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeRoomModerationGrant {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub grant_id: RoomModerationGrantId,
}

pub trait RoomModerationGrantUseCases: Send + Sync {
    fn grant(
        &self,
        request: GrantRoomModeration,
    ) -> PortFuture<'_, ModerationGrantResult<ActiveRoomModerationGrant>>;

    fn list(
        &self,
        request: ListRoomModerationGrants,
    ) -> PortFuture<'_, ModerationGrantResult<Vec<ActiveRoomModerationGrant>>>;

    /// 撤掉一份。已经撤了的也算撤了。
    fn revoke(
        &self,
        request: RevokeRoomModerationGrant,
    ) -> PortFuture<'_, ModerationGrantResult<()>>;
}

pub struct RoomModerationGrantDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub grants: Arc<dyn RoomModerationGrantStore>,
    pub clock: Arc<dyn Clock>,
}

pub struct RoomModerationGrantService {
    rooms: Arc<dyn PrivateRoomStore>,
    grants: Arc<dyn RoomModerationGrantStore>,
    clock: Arc<dyn Clock>,
}

impl RoomModerationGrantService {
    pub fn new(dependencies: RoomModerationGrantDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            grants: dependencies.grants,
            clock: dependencies.clock,
        }
    }

    async fn grant_internal(
        &self,
        request: GrantRoomModeration,
    ) -> ModerationGrantResult<ActiveRoomModerationGrant> {
        const OPERATION: &str = "private_room.moderation_grant.create";
        let now = self.clock.now();
        if now >= request.actor.expires_at || !request.actor.recently_authenticated {
            return Err(failure(OPERATION, ModerationGrantFailureKind::Forbidden));
        }
        let actor = request.actor.principal_id;
        let snapshot = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        snapshot
            .room()
            .authorize_moderation_grants(actor)
            .map_err(|error| domain(OPERATION, &error))?;
        let invalid = |_| failure(OPERATION, ModerationGrantFailureKind::InvalidRequest);
        let rules = RoomModerationRules::parse(&request.rules).map_err(invalid)?;
        let powers = RoomModerationPowers::new(request.powers).map_err(invalid)?;
        let lifetime = room_moderation_grant_lifetime(request.lifetime).map_err(invalid)?;
        let expires_at = now.checked_add(lifetime).map_err(invalid)?;
        let agent_display_name = self
            .grants
            .owned_agent_name(actor, request.agent_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or(failure(OPERATION, ModerationGrantFailureKind::NotYourAgent))?;
        let grant = RoomModerationGrant {
            id: request.grant_id,
            catalog_id: request.catalog_id,
            agent_id: request.agent_id,
            granted_by: actor,
            rules,
            powers,
            created_at: now,
            expires_at,
            revoked_at: None,
        };
        self.grants
            .replace_room_moderation_grant(&grant)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(ActiveRoomModerationGrant {
            grant,
            agent_display_name,
        })
    }

    async fn list_internal(
        &self,
        request: ListRoomModerationGrants,
    ) -> ModerationGrantResult<Vec<ActiveRoomModerationGrant>> {
        const OPERATION: &str = "private_room.moderation_grant.list";
        let now = self.clock.now();
        if now >= request.actor.expires_at {
            return Err(failure(OPERATION, ModerationGrantFailureKind::Forbidden));
        }
        let snapshot = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        snapshot
            .room()
            .authorize_moderation_grants(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.grants
            .active_room_moderation_grants(request.catalog_id, now)
            .await
            .map_err(|error| repository(OPERATION, &error))
    }

    async fn revoke_internal(
        &self,
        request: RevokeRoomModerationGrant,
    ) -> ModerationGrantResult<()> {
        const OPERATION: &str = "private_room.moderation_grant.revoke";
        let now = self.clock.now();
        if now >= request.actor.expires_at {
            return Err(failure(OPERATION, ModerationGrantFailureKind::Forbidden));
        }
        let actor = request.actor.principal_id;
        let grant = self
            .grants
            .room_moderation_grant(request.grant_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .filter(|grant| grant.catalog_id == request.catalog_id)
            .ok_or(failure(OPERATION, ModerationGrantFailureKind::NotFound))?;
        // 给它的人自己总能撤；别人要是这个房间此刻的房主或管理员。
        if grant.granted_by != actor {
            let snapshot = self
                .joined_room(&request.actor, request.catalog_id, OPERATION)
                .await?;
            snapshot
                .room()
                .authorize_moderation_grants(actor)
                .map_err(|error| domain(OPERATION, &error))?;
        }
        if grant.revoked_at.is_some() {
            return Ok(());
        }
        self.grants
            .revoke_room_moderation_grant(grant.id, actor, now)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    /// 他已经加入的这个房间；没有这个房间、他不在里面都当作找不到。
    async fn joined_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> ModerationGrantResult<PrivateRoomSnapshot> {
        self.rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .filter(|snapshot| {
                snapshot
                    .room()
                    .member(actor.principal_id)
                    .is_some_and(PrivateRoomMember::has_joined)
            })
            .ok_or(failure(operation, ModerationGrantFailureKind::NotFound))
    }
}

impl RoomModerationGrantUseCases for RoomModerationGrantService {
    fn grant(
        &self,
        request: GrantRoomModeration,
    ) -> PortFuture<'_, ModerationGrantResult<ActiveRoomModerationGrant>> {
        Box::pin(self.grant_internal(request))
    }

    fn list(
        &self,
        request: ListRoomModerationGrants,
    ) -> PortFuture<'_, ModerationGrantResult<Vec<ActiveRoomModerationGrant>>> {
        Box::pin(self.list_internal(request))
    }

    fn revoke(
        &self,
        request: RevokeRoomModerationGrant,
    ) -> PortFuture<'_, ModerationGrantResult<()>> {
        Box::pin(self.revoke_internal(request))
    }
}

const fn failure(
    operation: &'static str,
    kind: ModerationGrantFailureKind,
) -> ModerationGrantFailure {
    ModerationGrantFailure::new(operation, kind)
}

const fn domain(operation: &'static str, error: &DomainError) -> ModerationGrantFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => ModerationGrantFailureKind::Forbidden,
        DomainError::Validation { .. } => ModerationGrantFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => ModerationGrantFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => ModerationGrantFailureKind::Internal,
    };
    failure(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> ModerationGrantFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => ModerationGrantFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => ModerationGrantFailureKind::NotFound,
        RepositoryErrorKind::Conflict => ModerationGrantFailureKind::Conflict,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => ModerationGrantFailureKind::Internal,
    };
    failure(operation, kind)
}
