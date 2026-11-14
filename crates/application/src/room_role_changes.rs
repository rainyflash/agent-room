//! 读一间房的身份变动记录（`specs/redesign/room-roles.md` 的“记录”）。
//!
//! 记录是保存房间时顺手写下的（`private_rooms::service` 的 `save_if_changed`），这里只管谁能读：
//! 房主和管理员。归档了的房间照样能读，那是事后查“谁动过谁”最用得上的时候。

use std::sync::Arc;

use agent_room_domain::ids::RoomCatalogId;

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{Clock, PortFuture, PrivateRoomStore, RoomRoleChangeLog, RoomRoleChangeRecord},
};

/// 一次给最近这么多条，新的在前。先不分页：改身份是很少做的事。
pub const MAX_ROLE_CHANGES: u16 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRoomRoleChanges {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomRoleChangeFailure {
    /// 登录过期。
    Forbidden,
    /// 在房间里，但不是房主或管理员。
    ManagersOnly,
    /// 没有这个房间，或者不是它已加入的成员。
    NotFound,
    DependencyUnavailable,
    Internal,
}

pub type RoomRoleChangeResult<T> = Result<T, RoomRoleChangeFailure>;

pub trait RoomRoleChangeUseCases: Send + Sync {
    fn recent(
        &self,
        request: ListRoomRoleChanges,
    ) -> PortFuture<'_, RoomRoleChangeResult<Vec<RoomRoleChangeRecord>>>;
}

pub struct RoomRoleChangeDependencies {
    pub log: Arc<dyn RoomRoleChangeLog>,
    pub private_rooms: Arc<dyn PrivateRoomStore>,
    pub clock: Arc<dyn Clock>,
}

pub struct RoomRoleChangeService {
    log: Arc<dyn RoomRoleChangeLog>,
    private_rooms: Arc<dyn PrivateRoomStore>,
    clock: Arc<dyn Clock>,
}

impl RoomRoleChangeService {
    pub fn new(dependencies: RoomRoleChangeDependencies) -> Self {
        Self {
            log: dependencies.log,
            private_rooms: dependencies.private_rooms,
            clock: dependencies.clock,
        }
    }

    async fn recent_internal(
        &self,
        request: ListRoomRoleChanges,
    ) -> RoomRoleChangeResult<Vec<RoomRoleChangeRecord>> {
        if request.actor.expires_at <= self.clock.now() {
            return Err(RoomRoleChangeFailure::Forbidden);
        }
        self.ensure_runs_room(&request).await?;
        self.log
            .recent_role_changes(request.catalog_id, MAX_ROLE_CHANGES)
            .await
            .map_err(|error| repository(&error))
    }

    /// 只给已加入、身上有“管理”的人看（房主身上五项都有）。不看房间归没归档：
    /// `PrivateRoom::allows` 在归档的房间里只认查看，这里要的是这个人的身份。不是成员的和没有这个
    /// 房间一样回答，不让人拿房间 ID 试探。
    async fn ensure_runs_room(&self, request: &ListRoomRoleChanges) -> RoomRoleChangeResult<()> {
        let snapshot = self
            .private_rooms
            .find_by_catalog(request.catalog_id)
            .await
            .map_err(|error| repository(&error))?
            .ok_or(RoomRoleChangeFailure::NotFound)?;
        let member = snapshot
            .room()
            .member(request.actor.principal_id)
            .filter(|member| member.has_joined())
            .ok_or(RoomRoleChangeFailure::NotFound)?;
        if member.permissions().manage() {
            Ok(())
        } else {
            Err(RoomRoleChangeFailure::ManagersOnly)
        }
    }
}

const fn repository(error: &RepositoryError) -> RoomRoleChangeFailure {
    match error.kind() {
        RepositoryErrorKind::Unavailable => RoomRoleChangeFailure::DependencyUnavailable,
        RepositoryErrorKind::NotFound => RoomRoleChangeFailure::NotFound,
        RepositoryErrorKind::Forbidden => RoomRoleChangeFailure::Forbidden,
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => RoomRoleChangeFailure::Internal,
    }
}

impl RoomRoleChangeUseCases for RoomRoleChangeService {
    fn recent(
        &self,
        request: ListRoomRoleChanges,
    ) -> PortFuture<'_, RoomRoleChangeResult<Vec<RoomRoleChangeRecord>>> {
        Box::pin(self.recent_internal(request))
    }
}
