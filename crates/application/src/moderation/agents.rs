//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：本机 Agent 照主人给的管理授权
//! 自己动手。它给的是在消息里看到的发言人的聊天账号，这里认成一个人（维护者 2026-11-03 定：动别人的 Agent
//! 等于动它的主人），照房主和管理员自己动手的那一套落进台账，执行人记成给授权的人，另记是他的哪个 Agent、
//! 按哪份授权、它写的理由。动手不用最近登录，授权本身就是主人的同意；房主、管理员和给授权的人动不了。
//!
//! 禁言可以限时；踢出、封禁照成员一节那一套落（[`crate::ports::ModerationMemberGateway`]）；解封就是撤回
//! Agent 做的那条封禁，人封的 Agent 解不了。网络 Agent 当对象留到第 7a-2c-3 步。网络 Agent 没有主人，拿不到
//! 授权，所以只有本机 Agent 动手的入口。

mod unban;

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
        AgentModerationStore, MatrixUserId, ModerationActionReservationOutcome,
        ModerationMemberRefusal, ModerationRoomContext, PortFuture, PrivateRoomStore,
        RoomModerationGrant,
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
    /// 踢出时他已经不在房间里。
    NotInRoom,
    /// 解封时他没被封着。
    NotBanned,
    /// 解封时封着他的是人，不是 Agent。
    BannedByPerson,
    /// 这一步还做不了：对象是网络 Agent（第 7a-2c-3 步）。
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
    /// 禁言、踢出、封禁要给；解封不用。
    pub reason: Option<ModerationReason>,
    pub note: AgentModerationNote,
    /// 限时禁言多久；不给就一直禁着，直到有人撤回。别的不收时长。
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

    /// 照授权动手。同一个动作 ID 再来一次交回已经记下的那条。解封交回被撤回的那条封禁。
    fn moderate(
        &self,
        request: AgentModerate,
    ) -> PortFuture<'_, AgentModerationResult<ModerationAction>>;

    /// 房主和管理员看台账时，哪些是 Agent 做的、哪些是 Agent 撤回的。
    fn list(
        &self,
        request: ListAgentModeration,
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationLedger>>;
}

/// 台账里 Agent 经手的：它做的动作，和它撤回的动作（解封）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationLedger {
    pub actions: Vec<AgentModerationRecord>,
    pub reversals: Vec<AgentModerationRecord>,
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
        let (kind, duration) = match request.power {
            RoomModerationPower::Mute => (
                Some(ModerationActionKind::Mute),
                agent_mute_duration(request.duration)
                    .map_err(|_| failure(OPERATION, AgentModerationFailureKind::InvalidRequest))?,
            ),
            _ if request.duration.is_some() => {
                return Err(failure(
                    OPERATION,
                    AgentModerationFailureKind::InvalidRequest,
                ));
            }
            RoomModerationPower::Kick => (Some(ModerationActionKind::Kick), None),
            RoomModerationPower::Ban => (Some(ModerationActionKind::Ban), None),
            RoomModerationPower::Unban => (None, None),
        };
        let subject = self.subject(&request.target, OPERATION).await?;
        self.ensure_unprotected(&grant, device_principal, subject, OPERATION)
            .await?;
        let target = ModerationTarget::new(ModerationTargetKind::Principal, subject.to_string())
            .map_err(|_| failure(OPERATION, AgentModerationFailureKind::Internal))?;
        let Some(kind) = kind else {
            return self.unban(&request, &grant, &target, OPERATION).await;
        };
        self.act(&request, &grant, kind, target, duration, OPERATION)
            .await
    }

    /// 落一条禁言、踢出或封禁：照房主和管理员自己动手那一套，执行人是给授权的人，预留时同一个事务记下是
    /// 这个 Agent。
    async fn act(
        &self,
        request: &AgentModerate,
        grant: &RoomModerationGrant,
        kind: ModerationActionKind,
        target: ModerationTarget,
        duration: Option<DurationMillis>,
        operation: &'static str,
    ) -> AgentModerationResult<ModerationAction> {
        let moderation = &self.moderation;
        let now = moderation.clock.now();
        let reason = request
            .reason
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::InvalidRequest))?;
        let context = self
            .granter_context(grant, kind, &target, operation)
            .await?;
        let rooms = moderation
            .effect_rooms(&context, kind, &target, operation)
            .await
            .map_err(ledger)?;
        self.ensure_member_action(&context, grant, kind, &target, operation)
            .await?;
        let expires_at = duration
            .map(|duration| now.checked_add(duration))
            .transpose()
            .map_err(|_| failure(operation, AgentModerationFailureKind::InvalidRequest))?;
        let action = ModerationAction::reserve(
            request.action_id,
            None,
            grant.granted_by,
            grant.catalog_id,
            kind,
            target,
            reason,
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

    /// 给授权的人此刻还得管得了这个房间：撤了他的管理，他的 Agent 也跟着管不了。
    async fn granter_context(
        &self,
        grant: &RoomModerationGrant,
        kind: ModerationActionKind,
        target: &ModerationTarget,
        operation: &'static str,
    ) -> AgentModerationResult<ModerationRoomContext> {
        let context = self
            .moderation
            .authority
            .inspect_room(grant.granted_by, grant.catalog_id, target)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::NotFound))?;
        if !context.role.allows(kind) {
            return Err(failure(operation, AgentModerationFailureKind::NotGranted));
        }
        Ok(context)
    }

    /// 踢出、封禁照成员一节那一套落之前先看做不做得了：踢出时他得还在房间里。
    async fn ensure_member_action(
        &self,
        context: &ModerationRoomContext,
        grant: &RoomModerationGrant,
        kind: ModerationActionKind,
        target: &ModerationTarget,
        operation: &'static str,
    ) -> AgentModerationResult<()> {
        let refusal = self
            .moderation
            .member_refusal(context, grant.catalog_id, kind, target, operation)
            .await
            .map_err(ledger)?;
        match refusal {
            None => Ok(()),
            Some(ModerationMemberRefusal::Owner) => {
                Err(failure(operation, AgentModerationFailureKind::Protected))
            }
            Some(ModerationMemberRefusal::NotInRoom) => {
                Err(failure(operation, AgentModerationFailureKind::NotInRoom))
            }
            Some(ModerationMemberRefusal::RoomUnavailable) => {
                Err(failure(operation, AgentModerationFailureKind::NotFound))
            }
        }
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
    ) -> AgentModerationResult<AgentModerationLedger> {
        const OPERATION: &str = "agent_moderation.list";
        if request.actor.expires_at <= self.moderation.clock.now() {
            return Err(failure(OPERATION, AgentModerationFailureKind::Forbidden));
        }
        self.moderation
            .require_room_moderator(&request.actor, request.room_catalog_id, OPERATION)
            .await
            .map_err(ledger)?;
        let actions = self
            .store
            .agent_actions(request.room_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        let reversals = self
            .store
            .agent_reversals(request.room_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(AgentModerationLedger { actions, reversals })
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
    ) -> PortFuture<'_, AgentModerationResult<AgentModerationLedger>> {
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
