//! 分享页放实况的开关（`specs/redesign/project-rooms.md`“分享页放实况”，3P-6a）：
//! `GET`、`PUT /private-rooms/{c}/live-view`。
//!
//! 房间里已加入的人都看得到开没开、此刻放没放、差哪几样（房间页顶栏那一行小字、设置里的开关都按它）；
//! 只有房主和管理员能开关。这几样都满足才放，有一样不满足就不放：
//!
//! 1. 是主房间（子房间一概不放）；
//! 2. 房间在使用中（没关）；
//! 3. 公开着；
//! 4. 没被平台冻结、从目录里拿掉；
//! 5. 不加密：目录上的标记是 false，打开的时候再问一遍聊天服务器房间状态里有没有 `m.room.encryption`；
//! 6. 加入方式是直接进。
//!
//! 打开时记下这一刻（`enabled_at`），分享页只放这一刻以后说的话。改成要申请或只能邀请、改回私密、关掉
//! 房间时，库里在同一个事务里删掉这一行（`postgres-adapter` 的 `rooms/live_views.rs`），再开就是新的一刻。
//! 平台冻结、拿掉时不删，只是不放。

use std::sync::Arc;

use agent_room_domain::{
    ids::RoomCatalogId,
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomMember},
    room_listing::RoomJoinMode,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, EnableRoomLiveViewOutcome, MatrixFailure, MatrixFailureKind, MatrixRoomId,
        PortFuture, PrivateRoomSnapshot, PrivateRoomStore, RoomEncryptionState, RoomLiveViewFacts,
        RoomLiveViewRecord, RoomLiveViewStore,
    },
};

/// 放实况要满足的一样，按检查的先后排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LiveViewCondition {
    /// 这是一间子房间：子房间一概不放。
    SubRoom,
    /// 房间关了（归档）。
    Closed,
    /// 房间不公开。
    NotListed,
    /// 平台冻结了，或者从目录里拿掉了。
    PlatformHold,
    /// 房间加密。
    Encrypted,
    /// 加入方式不是直接进：要申请、只能邀请的房间不让外面的人绕过门看里面的话。
    JoinMode,
}

impl LiveViewCondition {
    /// 回答和错误详情里的写法。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SubRoom => "sub_room",
            Self::Closed => "closed",
            Self::NotListed => "not_listed",
            Self::PlatformHold => "platform_hold",
            Self::Encrypted => "encrypted",
            Self::JoinMode => "join_mode",
        }
    }
}

/// 一间房此刻的实况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveViewState {
    /// 开着：谁、什么时候开的。
    pub enabled: Option<RoomLiveViewRecord>,
    /// 此刻不满足的几样，按 [`LiveViewCondition`] 的先后。空的才放得了。
    pub unmet: Vec<LiveViewCondition>,
    /// 问的人能不能开关（房主和管理员）。
    pub can_change: bool,
}

impl LiveViewState {
    /// 分享页此刻放着这间房的实况。
    pub fn live(&self) -> bool {
        self.enabled.is_some() && self.unmet.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveViewFailureKind {
    /// 登录过期，或者不是房主、管理员。
    Forbidden,
    /// 没有这个房间，或者他不在房间里。
    NotFound,
    /// 要开的时候不满足这一样（几样都不满足时是排在最前的那一样）。
    NotEligible(LiveViewCondition),
    /// 开的那一下设置刚被别人改过：再看一眼再开。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveViewFailure {
    operation: &'static str,
    kind: LiveViewFailureKind,
}

impl LiveViewFailure {
    pub const fn new(operation: &'static str, kind: LiveViewFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> LiveViewFailureKind {
        self.kind
    }
}

pub type LiveViewResult<T> = Result<T, LiveViewFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectLiveView {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateLiveView {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub enabled: bool,
}

pub trait RoomLiveViewUseCases: Send + Sync {
    fn inspect(&self, request: InspectLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>>;

    /// 开或者关，回改好以后的样子。已经开着的再开不换那一刻，关着的再关什么都不做。
    fn update(&self, request: UpdateLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>>;
}

pub struct RoomLiveViewDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub live_views: Arc<dyn RoomLiveViewStore>,
    /// 打开时问聊天服务器加没加密。
    pub encryption: Arc<dyn RoomEncryptionState>,
    pub clock: Arc<dyn Clock>,
}

pub struct RoomLiveViewService {
    rooms: Arc<dyn PrivateRoomStore>,
    live_views: Arc<dyn RoomLiveViewStore>,
    encryption: Arc<dyn RoomEncryptionState>,
    clock: Arc<dyn Clock>,
}

impl RoomLiveViewService {
    pub fn new(dependencies: RoomLiveViewDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            live_views: dependencies.live_views,
            encryption: dependencies.encryption,
            clock: dependencies.clock,
        }
    }

    async fn inspect_internal(&self, request: InspectLiveView) -> LiveViewResult<LiveViewState> {
        const OPERATION: &str = "room_live.inspect";
        let snapshot = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let can_change = snapshot
            .room()
            .authorize_live_view_change(request.actor.principal_id)
            .is_ok();
        self.state(&snapshot, can_change, OPERATION).await
    }

    async fn update_internal(&self, request: UpdateLiveView) -> LiveViewResult<LiveViewState> {
        const OPERATION: &str = "room_live.update";
        let snapshot = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        snapshot
            .room()
            .authorize_live_view_change(request.actor.principal_id)
            .map_err(|_| LiveViewFailure::new(OPERATION, LiveViewFailureKind::Forbidden))?;
        if !request.enabled {
            self.live_views
                .disable_live_view(request.catalog_id)
                .await
                .map_err(|error| repository(OPERATION, &error))?;
            return self.state(&snapshot, true, OPERATION).await;
        }
        let facts = self.facts(request.catalog_id, OPERATION).await?;
        let unmet = unmet_conditions(&snapshot, &facts);
        if let Some(first) = unmet.first() {
            return Err(LiveViewFailure::new(
                OPERATION,
                LiveViewFailureKind::NotEligible(*first),
            ));
        }
        if let Some(enabled) = facts.live_view {
            return Ok(LiveViewState {
                enabled: Some(enabled),
                unmet,
                can_change: true,
            });
        }
        self.ensure_not_encrypted(&snapshot, OPERATION).await?;
        let outcome = self
            .live_views
            .enable_live_view(
                request.catalog_id,
                request.actor.principal_id,
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        match outcome {
            EnableRoomLiveViewOutcome::Enabled(record)
            | EnableRoomLiveViewOutcome::AlreadyEnabled(record) => Ok(LiveViewState {
                enabled: Some(record),
                unmet: Vec::new(),
                can_change: true,
            }),
            EnableRoomLiveViewOutcome::NotEligible => {
                // 刚被改了设置、刚归档：再读一遍，照实说是哪一样。
                let facts = self.facts(request.catalog_id, OPERATION).await?;
                let kind = unmet_conditions(&snapshot, &facts)
                    .first()
                    .map_or(LiveViewFailureKind::Conflict, |condition| {
                        LiveViewFailureKind::NotEligible(*condition)
                    });
                Err(LiveViewFailure::new(OPERATION, kind))
            }
        }
    }

    /// 打开之前问一遍聊天服务器：目录上记着不加密，房间状态里却有 `m.room.encryption`（改回私密做到
    /// 一半）也不放。
    async fn ensure_not_encrypted(
        &self,
        snapshot: &PrivateRoomSnapshot,
        operation: &'static str,
    ) -> LiveViewResult<()> {
        let room = MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
            .map_err(|_| LiveViewFailure::new(operation, LiveViewFailureKind::Internal))?;
        let encrypted = self
            .encryption
            .room_encrypted(&room)
            .await
            .map_err(|error| matrix(operation, error))?;
        if encrypted {
            return Err(LiveViewFailure::new(
                operation,
                LiveViewFailureKind::NotEligible(LiveViewCondition::Encrypted),
            ));
        }
        Ok(())
    }

    async fn state(
        &self,
        snapshot: &PrivateRoomSnapshot,
        can_change: bool,
        operation: &'static str,
    ) -> LiveViewResult<LiveViewState> {
        let facts = self.facts(snapshot.room().catalog_id(), operation).await?;
        Ok(LiveViewState {
            enabled: facts.live_view,
            unmet: unmet_conditions(snapshot, &facts),
            can_change,
        })
    }

    async fn facts(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> LiveViewResult<RoomLiveViewFacts> {
        self.live_views
            .live_view_facts(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or(LiveViewFailure::new(
                operation,
                LiveViewFailureKind::NotFound,
            ))
    }

    /// 登录有效、房间在、操作者已加入。看不到的和没有这个房间一样回答。
    async fn joined_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> LiveViewResult<PrivateRoomSnapshot> {
        if self.clock.now() >= actor.expires_at {
            return Err(LiveViewFailure::new(
                operation,
                LiveViewFailureKind::Forbidden,
            ));
        }
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
            .ok_or(LiveViewFailure::new(
                operation,
                LiveViewFailureKind::NotFound,
            ))
    }
}

impl RoomLiveViewUseCases for RoomLiveViewService {
    fn inspect(&self, request: InspectLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>> {
        Box::pin(self.inspect_internal(request))
    }

    fn update(&self, request: UpdateLiveView) -> PortFuture<'_, LiveViewResult<LiveViewState>> {
        Box::pin(self.update_internal(request))
    }
}

/// 此刻不满足的几样。从没公开过的房间没有加入方式，只说不公开。
fn unmet_conditions(
    snapshot: &PrivateRoomSnapshot,
    facts: &RoomLiveViewFacts,
) -> Vec<LiveViewCondition> {
    let mut unmet = Vec::new();
    if snapshot.parent_catalog_id().is_some() {
        unmet.push(LiveViewCondition::SubRoom);
    }
    if snapshot.room().status() != PrivateRoomLifecycleStatus::Active {
        unmet.push(LiveViewCondition::Closed);
    }
    if !facts.listing.is_some_and(|(listed, _)| listed) {
        unmet.push(LiveViewCondition::NotListed);
    }
    if facts.platform_hold {
        unmet.push(LiveViewCondition::PlatformHold);
    }
    if facts.end_to_end_encrypted {
        unmet.push(LiveViewCondition::Encrypted);
    }
    if facts
        .listing
        .is_some_and(|(_, join_mode)| join_mode != RoomJoinMode::Open)
    {
        unmet.push(LiveViewCondition::JoinMode);
    }
    unmet
}

const fn repository(operation: &'static str, error: &RepositoryError) -> LiveViewFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => LiveViewFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => LiveViewFailureKind::NotFound,
        RepositoryErrorKind::Conflict => LiveViewFailureKind::Conflict,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => LiveViewFailureKind::Internal,
    };
    LiveViewFailure::new(operation, kind)
}

/// 聊天服务器暂时问不到的，过一会儿再点；别的是配置或数据不对。
const fn matrix(operation: &'static str, error: MatrixFailure) -> LiveViewFailure {
    let kind = match error.kind() {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => LiveViewFailureKind::DependencyUnavailable,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => LiveViewFailureKind::Internal,
    };
    LiveViewFailure::new(operation, kind)
}
