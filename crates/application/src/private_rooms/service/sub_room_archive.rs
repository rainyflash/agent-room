//! 子房间第 2e-2 步（`specs/redesign/sub-rooms.md` 的“归档和恢复”）：把一间子房间收起来、拿回来。
//!
//! 库里那一半（房间的状态，谁、什么时候、为什么归档的，在等的申请作废）由存储在一个事务里做完；聊天
//! 服务器上锁住、解开排进子房间的任务表（这间子房间自己的那一件），这个请求结束前先做一遍。没做成不算
//! 请求失败：事还排着，后台接着做（`sub_room_sync`）。
//!
//! 网络 Agent 用 `agent_room_close_room` 归档它管的子房间（2h-3a，[`AgentSubRoomArchival`]）：它自己
//! 当房主的房间下面每一间照房主归档；凭口令、敲门进了别人的房间、在下面建的，照“它建的”认它
//! （[`authorize_creator_agent_sub_room_archive`]）。恢复只给人做。
//!
//! 30 天没人说话的由每小时那一轮收起来（2j-2，[`InactiveSubRoomArchival`]）：没有动手的人，原因记
//! `inactive`，原来在里面的人谁都能恢复。安静不安静由那一轮问过聊天服务器再定，这里只管存。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    sub_room_archive::{
        SubRoomArchiveReason, SubRoomArchiveRefusal, archived_sub_room,
        authorize_creator_agent_sub_room_archive, authorize_sub_room_archive,
        authorize_sub_room_restore, restored_sub_room,
    },
    time::UtcMillis,
};

use crate::ports::{
    PortFuture, PrivateRoomSnapshot, SubRoomArchival, SubRoomArchiveOutcome, SubRoomRestoration,
};

use super::{
    super::{
        ArchiveSubRoom, PrivateRoomActor, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomResult, RestoreSubRoom, SubRoomRefusal,
        failure::{domain, failure, repository},
    },
    PrivateRoomService, ensure_active_actor, ensure_active_room_actor, replace_room,
    sub_room_membership::LoadedSubRoom,
    sub_rooms::{has_joined, not_found, refused},
};

/// 网络 Agent 要归档的一间（2h-3a）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveAgentSubRoom {
    /// 只认网络 Agent：本机 Agent 建的子房间由主人在网页上归档。
    pub actor: PrivateRoomActor,
    pub agent_id: AgentId,
    pub catalog_id: RoomCatalogId,
}

/// 网络 Agent 归档子房间这一样。网络 Agent 的用例只拿这个。
pub trait AgentSubRoomArchival: Send + Sync {
    /// 归档好的（或者本来就归档了的）子房间；这间房不是子房间时回 `None`，照整间房关。
    fn archive_agent_sub_room(
        &self,
        request: ArchiveAgentSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>>;
}

/// 自动归档那一轮用的。
pub trait InactiveSubRoomArchival: Send + Sync {
    /// 把这间子房间记成没人说话、自动归档。归档了的回 `true`；已经归档了、不再挂在这间房下面、父房间
    /// 不在用了的什么都不做，回 `false`。读到以后又被存过的报错，下一轮再来。
    fn archive_inactive_sub_room(
        &self,
        parent_catalog_id: RoomCatalogId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, PrivateRoomResult<bool>>;
}

impl InactiveSubRoomArchival for PrivateRoomService {
    fn archive_inactive_sub_room(
        &self,
        parent_catalog_id: RoomCatalogId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, PrivateRoomResult<bool>> {
        Box::pin(self.archive_inactive_sub_room_internal(parent_catalog_id, catalog_id))
    }
}

impl AgentSubRoomArchival for PrivateRoomService {
    fn archive_agent_sub_room(
        &self,
        request: ArchiveAgentSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(self.archive_agent_sub_room_internal(request))
    }
}

impl PrivateRoomService {
    pub(super) async fn archive_sub_room_internal(
        &self,
        request: ArchiveSubRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "sub_room.archive";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let LoadedSubRoom {
            parent, sub_room, ..
        } = self
            .load_sub_room(
                request.parent_catalog_id,
                request.catalog_id,
                actor,
                OPERATION,
            )
            .await?;
        authorize_sub_room_archive(parent.room(), sub_room.room(), actor)
            .map_err(|refusal| archive_refused(OPERATION, refusal))?;
        self.archive_loaded(&parent, sub_room, actor, now, OPERATION)
            .await
    }

    /// 谁能归档已经看过了：存成归档、锁住聊天服务器上的房间。已经归档了的原样回：上一次聊天服务器上
    /// 没锁成的还排着，后台接着做。
    async fn archive_loaded(
        &self,
        parent: &PrivateRoomSnapshot,
        sub_room: PrivateRoomSnapshot,
        by: PrincipalId,
        now: UtcMillis,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let Some(archived) =
            archived_sub_room(sub_room.room()).map_err(|error| domain(operation, &error))?
        else {
            return Ok(sub_room);
        };
        let outcome = self
            .sub_rooms
            .archive_sub_room(&SubRoomArchival {
                room: &archived,
                expected_version: sub_room.room().version(),
                parent_catalog_id: parent.catalog().id(),
                by: Some(by),
                reason: SubRoomArchiveReason::Manual,
                at: now,
            })
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?;
        stored(outcome, operation)?;
        self.sync_sub_rooms_under(parent).await;
        replace_room(sub_room, archived, operation)
    }

    async fn archive_agent_sub_room_internal(
        &self,
        request: ArchiveAgentSubRoom,
    ) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
        const OPERATION: &str = "sub_room.archive_by_agent";
        let now = self.clock.now();
        ensure_active_room_actor(&request.actor, now, OPERATION)?;
        if !matches!(request.actor, PrivateRoomActor::NetworkAgent(_)) {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ));
        }
        let viewer = request.actor.principal_id();
        let sub_room = self
            .find(request.catalog_id, OPERATION)
            .await?
            .ok_or_else(|| not_found(OPERATION))?;
        let Some(parent_id) = sub_room.parent_catalog_id() else {
            return Ok(None);
        };
        let parent = self.load(parent_id, OPERATION).await?;
        let link = self
            .sub_rooms
            .sub_room_link(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
            .ok_or_else(|| not_found(OPERATION))?;
        let authorized = if has_joined(parent.room(), viewer) {
            // 它自己的房间：合成主体是房主，照房主归档。
            authorize_sub_room_archive(parent.room(), sub_room.room(), viewer)
        } else {
            let inside = self
                .agent_inside(request.catalog_id, request.agent_id, OPERATION)
                .await?;
            let created = link.created_by_agent == Some(request.agent_id);
            // 不在父房间里的、藏起来又不是它建的也不在里面的，和没有这间房一样。
            if !self
                .agent_inside(parent_id, request.agent_id, OPERATION)
                .await?
                || (link.hidden && !created && !inside)
            {
                return Err(not_found(OPERATION));
            }
            authorize_creator_agent_sub_room_archive(
                parent.room(),
                link.created_by_agent,
                request.agent_id,
                inside,
            )
        };
        match authorized {
            Ok(()) => {}
            // 父房间关了：这一间跟着归档（后台在做），当它关上了。
            Err(SubRoomArchiveRefusal::ParentArchived) => return Ok(Some(sub_room)),
            Err(refusal) => return Err(archive_refused(OPERATION, refusal)),
        }
        self.archive_loaded(&parent, sub_room, viewer, now, OPERATION)
            .await
            .map(Some)
    }

    async fn archive_inactive_sub_room_internal(
        &self,
        parent_catalog_id: RoomCatalogId,
        catalog_id: RoomCatalogId,
    ) -> PrivateRoomResult<bool> {
        const OPERATION: &str = "sub_room.auto_archive";
        let now = self.clock.now();
        let Some(parent) = self.find(parent_catalog_id, OPERATION).await? else {
            return Ok(false);
        };
        let Some(sub_room) = self
            .find(catalog_id, OPERATION)
            .await?
            .filter(|sub_room| sub_room.parent_catalog_id() == Some(parent_catalog_id))
        else {
            return Ok(false);
        };
        let Some(archived) =
            archived_sub_room(sub_room.room()).map_err(|error| domain(OPERATION, &error))?
        else {
            return Ok(false);
        };
        let outcome = self
            .sub_rooms
            .archive_sub_room(&SubRoomArchival {
                room: &archived,
                expected_version: sub_room.room().version(),
                parent_catalog_id,
                by: None,
                reason: SubRoomArchiveReason::Inactive,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        if outcome != SubRoomArchiveOutcome::Done {
            return Ok(false);
        }
        // 聊天服务器上锁住它：没做成的还排着，后台接着做。
        self.sync_sub_rooms_under(&parent).await;
        Ok(true)
    }

    pub(super) async fn restore_sub_room_internal(
        &self,
        request: RestoreSubRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "sub_room.restore";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let LoadedSubRoom {
            parent,
            sub_room,
            link,
        } = self
            .load_sub_room(
                request.parent_catalog_id,
                request.catalog_id,
                actor,
                OPERATION,
            )
            .await?;
        // 没归档的，和归档了却没记原因的，都只认管得了这一间的人。
        let reason = link
            .archived
            .map_or(SubRoomArchiveReason::Manual, |record| record.reason);
        authorize_sub_room_restore(parent.room(), sub_room.room(), actor, reason)
            .map_err(|refusal| archive_refused(OPERATION, refusal))?;
        // 没归档：上一次聊天服务器上没解开的还排着，后台接着做。
        let Some(restored) =
            restored_sub_room(sub_room.room()).map_err(|error| domain(OPERATION, &error))?
        else {
            return Ok(sub_room);
        };
        let outcome = self
            .sub_rooms
            .restore_sub_room(&SubRoomRestoration {
                room: &restored,
                expected_version: sub_room.room().version(),
                parent_catalog_id: request.parent_catalog_id,
                by: actor,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        stored(outcome, OPERATION)?;
        self.sync_sub_rooms_under(&parent).await;
        replace_room(sub_room, restored, OPERATION)
    }
}

const fn archive_refused(
    operation: &'static str,
    refusal: SubRoomArchiveRefusal,
) -> PrivateRoomFailure {
    match refusal {
        SubRoomArchiveRefusal::ManagersOnly => refused(operation, SubRoomRefusal::ManagersOnly),
        SubRoomArchiveRefusal::ParentArchived => refused(operation, SubRoomRefusal::Archived),
    }
}

/// 存的时候没写成的原因。
const fn stored(outcome: SubRoomArchiveOutcome, operation: &'static str) -> PrivateRoomResult<()> {
    match outcome {
        SubRoomArchiveOutcome::Done => Ok(()),
        // 刚看过以后父房间归档了、这一间不在了，或者别人刚归档、刚恢复过：请他看过新的再来。
        SubRoomArchiveOutcome::Unavailable => Err(failure(
            operation,
            PrivateRoomFailureStage::Persistence,
            PrivateRoomFailureKind::Conflict,
        )),
        // 恢复时撞了名字：先给同名的那一间改个名。
        SubRoomArchiveOutcome::NameTaken => Err(refused(operation, SubRoomRefusal::NameTaken)),
        SubRoomArchiveOutcome::LimitReached => {
            Err(refused(operation, SubRoomRefusal::LimitReached))
        }
    }
}
