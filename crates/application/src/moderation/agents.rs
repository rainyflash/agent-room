//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：本机 Agent 照主人给的管理授权
//! 自己动手。它给的是在消息里看到的发言人的聊天账号，这里认成一个人（维护者 2026-11-03 定：动别人的 Agent
//! 等于动它的主人），照房主和管理员自己动手的那一套落进台账，执行人记成给授权的人，另记是他的哪个 Agent、
//! 按哪份授权、它写的理由。动手不用最近登录，授权本身就是主人的同意；房主、管理员和给授权的人动不了。
//!
//! 这一步只有禁言（可以限时），踢出、封禁、解封要改成员记录，是第 7a-2c 步；网络 Agent 当对象也留到那一步。
//! 网络 Agent 没有主人，拿不到授权，所以只有本机 Agent 动手的入口。

use std::sync::Arc;

use agent_room_domain::{
    ids::{
        AgentId, AgentInstanceId, ModerationActionId, PrincipalId, RoomCatalogId,
        RoomModerationGrantId,
    },
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditOutcome, ModerationReason,
        ModerationTarget, ModerationTargetKind,
    },
    private_rooms::PrivateRoomMembershipStatus,
    room_moderation_grants::{
        AgentModerationNote, RoomModerationPower, RoomModerationPowers, RoomModerationRules,
        agent_mute_duration,
    },
    time::{DurationMillis, UtcMillis},
};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentLobbyAccessRepository, AgentModerationAttribution, AgentModerationRecord,
        AgentModerationStore, MatrixUserId, ModerationActionReservationOutcome, PortFuture,
        PrivateRoomStore, RoomModerationGrant,
    },
};

use super::{ModerationFailure, ModerationFailureKind, ModerationService};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentModerationFailureKind {
    /// 设备、连接和 Agent 对不上，或者登录过期了。
    Unauthorized,
    /// 它在这个房间里没有在用的授权，或者给授权的人已经不是房主、管理员了。
    NotGranted,
    /// 授权里没勾这一样。
    NotAllowed,
    /// 对象是房主、管理员、给授权的人，或者他们的 Agent。
    Protected,
    /// 认不出这个聊天账号是谁。
    TargetNotFound,
    /// 这一步还做不了：踢出、封禁、解封，或者对象是网络 Agent（第 7a-2c 步）。
    Unsupported,
    InvalidRequest,
    Forbidden,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentModerationFailure {
    operation: &'static str,
    kind: AgentModerationFailureKind,
}

impl AgentModerationFailure {
    pub const fn new(operation: &'static str, kind: AgentModerationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentModerationFailureKind {
        self.kind
    }
}

pub type AgentModerationResult<T> = Result<T, AgentModerationFailure>;

/// 设备签名的请求：哪个 Agent 的哪个连接，在哪个房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationScope {
    pub device: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub room_catalog_id: RoomCatalogId,
}

/// Agent 读到的授权：规则、能做哪几样、到什么时候。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationView {
    pub grant_id: RoomModerationGrantId,
    pub rules: RoomModerationRules,
    pub powers: RoomModerationPowers,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerate {
    pub scope: AgentModerationScope,
    pub action_id: ModerationActionId,
    pub power: RoomModerationPower,
    /// 它在消息里看到的发言人的聊天账号。
    pub target: MatrixUserId,
    pub reason: ModerationReason,
    pub note: AgentModerationNote,
    /// 限时禁言多久；不给就一直禁着，直到有人撤回。
    pub duration: Option<DurationMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAgentModeration {
    pub actor: AuthenticatedPrincipal,
    pub room_catalog_id: RoomCatalogId,
}

pub trait AgentModerationUseCases: Send + Sync {
    /// 这个 Agent 在这个房间里的授权。
    fn rules(
        &self,
        scope: AgentModerationScope,
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationView>>;

    /// 照授权动手。同一个动作 ID 再来一次交回已经记下的那条。
    fn moderate(
        &self,
        request: AgentModerate,
    ) -> PortFuture<'_, AgentModerationResult<ModerationAction>>;

    /// 房主和管理员看台账时，哪些是 Agent 做的。
    fn list(
        &self,
        request: ListAgentModeration,
    ) -> PortFuture<'_, AgentModerationResult<Vec<AgentModerationRecord>>>;
}

pub struct AgentModerationDependencies {
    pub moderation: Arc<ModerationService>,
    pub store: Arc<dyn AgentModerationStore>,
    pub access: Arc<dyn AgentLobbyAccessRepository>,
    pub rooms: Arc<dyn PrivateRoomStore>,
}

pub struct AgentModerationService {
    moderation: Arc<ModerationService>,
    store: Arc<dyn AgentModerationStore>,
    access: Arc<dyn AgentLobbyAccessRepository>,
    rooms: Arc<dyn PrivateRoomStore>,
}

impl AgentModerationService {
    pub fn new(dependencies: AgentModerationDependencies) -> Self {
        Self {
            moderation: dependencies.moderation,
            store: dependencies.store,
            access: dependencies.access,
            rooms: dependencies.rooms,
        }
    }

    async fn rules_internal(
        &self,
        scope: AgentModerationScope,
    ) -> AgentModerationResult<AgentModerationView> {
        const OPERATION: &str = "agent_moderation.rules";
        let (grant, _) = self.granted(&scope, OPERATION).await?;
        Ok(AgentModerationView {
            grant_id: grant.id,
            rules: grant.rules,
            powers: grant.powers,
            expires_at: grant.expires_at,
        })
    }

    async fn moderate_internal(
        &self,
        request: AgentModerate,
    ) -> AgentModerationResult<ModerationAction> {
        const OPERATION: &str = "agent_moderation.moderate";
        let (grant, device_principal) = self.granted(&request.scope, OPERATION).await?;
        if !grant.powers.allows(request.power) {
            return Err(failure(OPERATION, AgentModerationFailureKind::NotAllowed));
        }
        if request.power != RoomModerationPower::Mute {
            return Err(failure(OPERATION, AgentModerationFailureKind::Unsupported));
        }
        let duration = agent_mute_duration(request.duration)
            .map_err(|_| failure(OPERATION, AgentModerationFailureKind::InvalidRequest))?;
        let subject = self.subject(&request.target, OPERATION).await?;
        self.ensure_unprotected(&grant, device_principal, subject, OPERATION)
            .await?;
        let target = ModerationTarget::new(ModerationTargetKind::Principal, subject.to_string())
            .map_err(|_| failure(OPERATION, AgentModerationFailureKind::Internal))?;
        self.mute(&request, &grant, target, duration, OPERATION)
            .await
    }

    /// 落一条禁言：照房主和管理员自己动手那一套，执行人是给授权的人，预留时同一个事务记下是这个 Agent。
    async fn mute(
        &self,
        request: &AgentModerate,
        grant: &RoomModerationGrant,
        target: ModerationTarget,
        duration: Option<DurationMillis>,
        operation: &'static str,
    ) -> AgentModerationResult<ModerationAction> {
        let moderation = &self.moderation;
        let now = moderation.clock.now();
        // 给授权的人此刻还得是房主或管理员：撤了他的管理，他的 Agent 也跟着管不了。
        let context = moderation
            .authority
            .inspect_room(grant.granted_by, grant.catalog_id, &target)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::NotFound))?;
        if !context.role.can_moderate_room() {
            return Err(failure(operation, AgentModerationFailureKind::NotGranted));
        }
        let rooms = moderation
            .effect_rooms(&context, ModerationActionKind::Mute, &target, operation)
            .await
            .map_err(ledger)?;
        let expires_at = duration
            .map(|duration| now.checked_add(duration))
            .transpose()
            .map_err(|_| failure(operation, AgentModerationFailureKind::InvalidRequest))?;
        let action = ModerationAction::reserve(
            request.action_id,
            None,
            grant.granted_by,
            grant.catalog_id,
            ModerationActionKind::Mute,
            target,
            request.reason,
            now,
            expires_at,
        )
        .map_err(|_| failure(operation, AgentModerationFailureKind::InvalidRequest))?;
        let correlation_id = moderation.identifiers.moderation_audit_event_id();
        let requested = moderation
            .action_audit(
                &action,
                "moderation.action.requested",
                ModerationAuditOutcome::Allowed,
                correlation_id,
                now,
                operation,
            )
            .map_err(ledger)?;
        let attribution = AgentModerationAttribution {
            action_id: action.id(),
            agent_id: grant.agent_id,
            grant_id: grant.id,
            note: request.note.clone(),
        };
        let action = match self
            .store
            .reserve_agent_action(&action, &requested, &attribution)
            .await
            .map_err(|error| repository(operation, &error))?
        {
            ModerationActionReservationOutcome::Reserved(action) => action,
            ModerationActionReservationOutcome::Existing(existing) => return Ok(existing),
        };
        let applied = moderation.apply_effects(&action, &context, &rooms).await;
        moderation
            .finish_applied(action, applied, correlation_id, operation)
            .await
            .map_err(ledger)
    }

    /// 设备签名的请求对得上这个 Agent 的这个连接，它在这个房间里有在用的授权。交回授权和连接代表的账号。
    async fn granted(
        &self,
        scope: &AgentModerationScope,
        operation: &'static str,
    ) -> AgentModerationResult<(RoomModerationGrant, PrincipalId)> {
        let now = self.moderation.clock.now();
        if scope.device.access_token_expires_at <= now {
            return Err(failure(operation, AgentModerationFailureKind::Unauthorized));
        }
        let access = self
            .access
            .find_lobby_access(scope.agent_instance_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::Unauthorized))?;
        if !access.active
            || access.agent_id != scope.agent_id
            || access.device_id != scope.device.device_id
        {
            return Err(failure(operation, AgentModerationFailureKind::Unauthorized));
        }
        let grant = self
            .store
            .active_agent_grant(scope.room_catalog_id, scope.agent_id, now)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::NotGranted))?;
        Ok((grant, access.principal_id))
    }

    /// 认人：人的账号就是他，Agent 认成它的主人。网络 Agent 的合成主体在房间里没有账号，这一步先不动它。
    async fn subject(
        &self,
        target: &MatrixUserId,
        operation: &'static str,
    ) -> AgentModerationResult<PrincipalId> {
        let subject = self
            .store
            .moderation_subject(target)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::TargetNotFound))?;
        if subject.network_agent {
            return Err(failure(operation, AgentModerationFailureKind::Unsupported));
        }
        Ok(subject.principal_id)
    }

    /// 房主、管理员（受邀和已加入的都算）、给授权的人和这个连接代表的人动不了，连同他们的 Agent：Agent
    /// 已经认成了它的主人。
    async fn ensure_unprotected(
        &self,
        grant: &RoomModerationGrant,
        device_principal: PrincipalId,
        subject: PrincipalId,
        operation: &'static str,
    ) -> AgentModerationResult<()> {
        let snapshot = self
            .rooms
            .find_by_catalog(grant.catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::NotFound))?;
        let room = snapshot.room();
        let manages = room.member(subject).is_some_and(|member| {
            member.permissions().manage()
                && matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
        });
        if subject == room.owner_principal_id()
            || subject == grant.granted_by
            || subject == device_principal
            || manages
        {
            return Err(failure(operation, AgentModerationFailureKind::Protected));
        }
        Ok(())
    }

    async fn list_internal(
        &self,
        request: ListAgentModeration,
    ) -> AgentModerationResult<Vec<AgentModerationRecord>> {
        const OPERATION: &str = "agent_moderation.list";
        if request.actor.expires_at <= self.moderation.clock.now() {
            return Err(failure(OPERATION, AgentModerationFailureKind::Forbidden));
        }
        self.moderation
            .require_room_moderator(&request.actor, request.room_catalog_id, OPERATION)
            .await
            .map_err(ledger)?;
        self.store
            .agent_actions(request.room_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))
    }
}

impl AgentModerationUseCases for AgentModerationService {
    fn rules(
        &self,
        scope: AgentModerationScope,
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationView>> {
        Box::pin(self.rules_internal(scope))
    }

    fn moderate(
        &self,
        request: AgentModerate,
    ) -> PortFuture<'_, AgentModerationResult<ModerationAction>> {
        Box::pin(self.moderate_internal(request))
    }

    fn list(
        &self,
        request: ListAgentModeration,
    ) -> PortFuture<'_, AgentModerationResult<Vec<AgentModerationRecord>>> {
        Box::pin(self.list_internal(request))
    }
}

const fn failure(
    operation: &'static str,
    kind: AgentModerationFailureKind,
) -> AgentModerationFailure {
    AgentModerationFailure::new(operation, kind)
}

/// 台账那边的失败照种类交回。
const fn ledger(failure: ModerationFailure) -> AgentModerationFailure {
    AgentModerationFailure::new(
        failure.operation(),
        match failure.kind() {
            ModerationFailureKind::InvalidRequest => AgentModerationFailureKind::InvalidRequest,
            ModerationFailureKind::Forbidden => AgentModerationFailureKind::Forbidden,
            ModerationFailureKind::NotFound => AgentModerationFailureKind::NotFound,
            ModerationFailureKind::Conflict | ModerationFailureKind::RateLimited => {
                AgentModerationFailureKind::Conflict
            }
            ModerationFailureKind::DependencyUnavailable => {
                AgentModerationFailureKind::DependencyUnavailable
            }
            ModerationFailureKind::Internal => AgentModerationFailureKind::Internal,
        },
    )
}

fn repository(operation: &'static str, error: &RepositoryError) -> AgentModerationFailure {
    failure(
        operation,
        match error.kind() {
            RepositoryErrorKind::Conflict => AgentModerationFailureKind::Conflict,
            RepositoryErrorKind::Forbidden => AgentModerationFailureKind::Forbidden,
            RepositoryErrorKind::NotFound => AgentModerationFailureKind::NotFound,
            RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
                AgentModerationFailureKind::Internal
            }
            RepositoryErrorKind::Unavailable => AgentModerationFailureKind::DependencyUnavailable,
        },
    )
}
