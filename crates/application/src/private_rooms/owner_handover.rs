//! 房主删除了账户（`specs/redesign/room-roles.md` 的“房主不在了（R6）”）：删除任务在停用外部账户
//! 以后、匿名化本地记录（`finalize_local`）以前调这里。
//!
//! - 他名下在用的房间交给当管理员最久的那位：先在房间里写一条 `room.owner_handover.v1`，再存。
//!   没有管理员的不动，照旧由 `finalize_local` 归档。子房间的房主跟着父房间，存的那个事务里一起改。
//! - 他在别人房间里当管理员的那几间记成离开。普通成员、只能看的照旧由 `finalize_local` 的 SQL 改。
//!
//! 两样都记身份变动，动手的人为空。做几遍结果都一样：交出去了的、已经离开的，再列他的房间时就不在
//! 了；写了状态、没存成的，重来时挑中同一个人，写一样的内容聊天服务器直接回旧的事件。

use std::sync::Arc;

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    owner_handover::handover_candidates,
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus},
    room_role_changes::standing_changes,
};

use crate::ports::{
    Clock, MatrixRoomId, MatrixUserId, OwnerHandoverWrite, PortFuture,
    PrivateRoomListingSaveOutcome, PrivateRoomListingStore, PrivateRoomMatrixGateway,
    PrivateRoomOwnerHandoverGateway, PrivateRoomPrincipalDirectory, PrivateRoomSnapshot,
    PrivateRoomStore, RoomRoleChange, RoomRoleChangeLog,
};

/// 交房间没做完：删除任务记下这个原因，照原来的退避重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerHandoverFailure {
    code: &'static str,
}

impl OwnerHandoverFailure {
    /// 读、存房间或身份变动记录没成。
    pub const PERSISTENCE: Self = Self {
        code: "private_room.handover.persistence",
    };
    /// 写那条状态或者改发言级别没成。
    pub const MATRIX: Self = Self {
        code: "private_room.handover.matrix",
    };
    /// 读到以后房间被人改了，或者领域规则不让交。
    pub const CONFLICT: Self = Self {
        code: "private_room.handover.conflict",
    };

    /// 记进删除任务的失败原因。
    pub const fn code(self) -> &'static str {
        self.code
    }
}

/// 这一遍做了什么。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OwnerHandoverOutcome {
    /// 交出去的房间。
    pub handed_over: usize,
    /// 没有能接手的人、留给 `finalize_local` 归档的房间。
    pub without_successor: usize,
    /// 他当管理员、记成离开的房间。
    pub left_as_admin: usize,
}

/// 删除任务只拿这一样。
pub trait DeletedOwnerRooms: Send + Sync {
    /// 把 `principal_id` 名下的房间交出去，他当管理员的房间记成离开。`former_owner` 是他在 Matrix
    /// 上的 ID，写进交房间的那条状态。
    fn hand_over_rooms_of<'a>(
        &'a self,
        principal_id: PrincipalId,
        former_owner: &'a MatrixUserId,
    ) -> PortFuture<'a, Result<OwnerHandoverOutcome, OwnerHandoverFailure>>;
}

pub struct OwnerHandoverDependencies {
    pub store: Arc<dyn PrivateRoomStore>,
    pub listings: Arc<dyn PrivateRoomListingStore>,
    pub role_changes: Arc<dyn RoomRoleChangeLog>,
    pub principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    pub matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub announcements: Arc<dyn PrivateRoomOwnerHandoverGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct OwnerHandoverService {
    store: Arc<dyn PrivateRoomStore>,
    listings: Arc<dyn PrivateRoomListingStore>,
    role_changes: Arc<dyn RoomRoleChangeLog>,
    principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    matrix: Arc<dyn PrivateRoomMatrixGateway>,
    announcements: Arc<dyn PrivateRoomOwnerHandoverGateway>,
    clock: Arc<dyn Clock>,
}

impl OwnerHandoverService {
    pub fn new(dependencies: OwnerHandoverDependencies) -> Self {
        Self {
            store: dependencies.store,
            listings: dependencies.listings,
            role_changes: dependencies.role_changes,
            principals: dependencies.principals,
            matrix: dependencies.matrix,
            announcements: dependencies.announcements,
            clock: dependencies.clock,
        }
    }

    async fn hand_over_internal(
        &self,
        principal_id: PrincipalId,
        former_owner: &MatrixUserId,
    ) -> Result<OwnerHandoverOutcome, OwnerHandoverFailure> {
        // 列出来的都是他此刻受邀或已加入的房间，没有子房间。
        let rooms = self
            .store
            .list_for_principal(principal_id)
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?;
        let mut outcome = OwnerHandoverOutcome::default();
        for snapshot in rooms {
            let room = snapshot.room();
            if room.status() == PrivateRoomLifecycleStatus::Archived
                || snapshot.parent_catalog_id().is_some()
            {
                continue;
            }
            if room.owner_principal_id() == principal_id {
                if self.hand_over(&snapshot, former_owner).await? {
                    outcome.handed_over += 1;
                } else {
                    outcome.without_successor += 1;
                }
            } else if room
                .member(principal_id)
                .is_some_and(|member| member.has_joined() && member.permissions().manage())
            {
                self.leave_as_admin(room, principal_id).await?;
                outcome.left_as_admin += 1;
            }
        }
        Ok(outcome)
    }

    /// 照先后挨个试：公开的房间轮不到名下已经有一间公开房间的人。返回 `false` 是一位都轮不上。
    async fn hand_over(
        &self,
        snapshot: &PrivateRoomSnapshot,
        former_owner: &MatrixUserId,
    ) -> Result<bool, OwnerHandoverFailure> {
        let catalog_id = snapshot.catalog().id();
        let tenure = self
            .role_changes
            .admin_tenure(catalog_id)
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?;
        let candidates = handover_candidates(snapshot.room(), &tenure);
        if candidates.is_empty() {
            return Ok(false);
        }
        let matrix_room =
            MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
                .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?;
        let counts_toward_quota = self.counts_toward_quota(catalog_id).await?;
        for successor in candidates {
            if counts_toward_quota && self.owns_other_listed_room(successor, catalog_id).await? {
                continue;
            }
            let Some(owner) = self
                .principals
                .matrix_user_id(successor)
                .await
                .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?
            else {
                continue;
            };
            let handover = OwnerHandoverWrite {
                owner,
                former_owner: former_owner.clone(),
            };
            if self
                .hand_over_to(snapshot, &matrix_room, successor, &handover)
                .await?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 写状态、补发言级别、存。返回 `false` 是存的时候才发现他刚公开了别的房间，什么都没存，发言级别
    /// 也收回了。
    async fn hand_over_to(
        &self,
        snapshot: &PrivateRoomSnapshot,
        matrix_room: &MatrixRoomId,
        successor: PrincipalId,
        handover: &OwnerHandoverWrite,
    ) -> Result<bool, OwnerHandoverFailure> {
        let before = snapshot.room();
        let mut room = before.clone();
        room.hand_over_from_deleted_owner(successor)
            .map_err(|_| OwnerHandoverFailure::CONFLICT)?;
        self.announcements
            .announce_owner_handover(matrix_room, handover)
            .await
            .map_err(|_| OwnerHandoverFailure::MATRIX)?;
        // “管理”不带着“发言”：接手的人原来不能说话的，房主什么都能做，先在聊天服务器上补上。
        let spoke = before
            .member(successor)
            .is_some_and(|member| member.permissions().speak());
        if !spoke {
            self.set_speaking(matrix_room, &handover.owner, true)
                .await?;
        }
        let saved = self
            .listings
            .save_transferred_room_recording(
                &room,
                before.version(),
                self.clock.now(),
                &unattended_changes(before, &room),
            )
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?;
        match saved {
            PrivateRoomListingSaveOutcome::Saved => Ok(true),
            PrivateRoomListingSaveOutcome::OwnerLimitReached => {
                if !spoke {
                    self.set_speaking(matrix_room, &handover.owner, false)
                        .await?;
                }
                Ok(false)
            }
            PrivateRoomListingSaveOutcome::ShareCodeTaken
            | PrivateRoomListingSaveOutcome::Stale => Err(OwnerHandoverFailure::CONFLICT),
        }
    }

    /// 这间房占不占接手的人的公开房间名额：公开着、没绑仓库的才占（`specs/redesign/project-rooms.md`）。
    async fn counts_toward_quota(
        &self,
        catalog_id: RoomCatalogId,
    ) -> Result<bool, OwnerHandoverFailure> {
        let listed = self
            .listings
            .listing(catalog_id)
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?
            .is_some_and(|listing| listing.settings.listed());
        if !listed {
            return Ok(false);
        }
        let bound = self
            .listings
            .repository_bound(catalog_id)
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?;
        Ok(!bound)
    }

    async fn owns_other_listed_room(
        &self,
        principal_id: PrincipalId,
        catalog_id: RoomCatalogId,
    ) -> Result<bool, OwnerHandoverFailure> {
        Ok(self
            .listings
            .unbound_listed_rooms_owned_by(principal_id)
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)?
            .iter()
            .any(|room| *room != catalog_id))
    }

    /// 他的聊天账户这时已经停用、离开了所有房间，聊天服务器上不用再动。
    async fn leave_as_admin(
        &self,
        before: &PrivateRoom,
        principal_id: PrincipalId,
    ) -> Result<(), OwnerHandoverFailure> {
        let mut room = before.clone();
        if !room
            .leave(principal_id)
            .map_err(|_| OwnerHandoverFailure::CONFLICT)?
        {
            return Ok(());
        }
        self.store
            .save_recording(
                &room,
                before.version(),
                self.clock.now(),
                &unattended_changes(before, &room),
            )
            .await
            .map_err(|_| OwnerHandoverFailure::PERSISTENCE)
    }

    async fn set_speaking(
        &self,
        room_id: &MatrixRoomId,
        user_id: &MatrixUserId,
        allowed: bool,
    ) -> Result<(), OwnerHandoverFailure> {
        self.matrix
            .set_speaking(room_id, user_id, allowed)
            .await
            .map_err(|_| OwnerHandoverFailure::MATRIX)
    }
}

impl DeletedOwnerRooms for OwnerHandoverService {
    fn hand_over_rooms_of<'a>(
        &'a self,
        principal_id: PrincipalId,
        former_owner: &'a MatrixUserId,
    ) -> PortFuture<'a, Result<OwnerHandoverOutcome, OwnerHandoverFailure>> {
        Box::pin(self.hand_over_internal(principal_id, former_owner))
    }
}

/// 不是谁动的手：身份变动记成动手的人为空。
fn unattended_changes(before: &PrivateRoom, after: &PrivateRoom) -> Vec<RoomRoleChange> {
    standing_changes(before, after)
        .into_iter()
        .map(|change| RoomRoleChange {
            actor: None,
            change,
        })
        .collect()
}
