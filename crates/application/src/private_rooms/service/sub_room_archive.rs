//! 子房间第 2e-2 步（`specs/redesign/sub-rooms.md` 的“归档和恢复”）：把一间子房间收起来、拿回来。
//!
//! 库里那一半（房间的状态，谁、什么时候、为什么归档的，在等的申请作废）由存储在一个事务里做完；聊天
//! 服务器上锁住、解开排进子房间的任务表（这间子房间自己的那一件），这个请求结束前先做一遍。没做成不算
//! 请求失败：事还排着，后台接着做（`sub_room_sync`）。

use agent_room_domain::sub_room_archive::{
    SubRoomArchiveReason, SubRoomArchiveRefusal, archived_sub_room, authorize_sub_room_archive,
    authorize_sub_room_restore, restored_sub_room,
};

use crate::ports::{
    PrivateRoomSnapshot, SubRoomArchival, SubRoomArchiveOutcome, SubRoomRestoration,
};

use super::{
    super::{
        ArchiveSubRoom, PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage,
        PrivateRoomResult, RestoreSubRoom, SubRoomRefusal,
        failure::{domain, failure, repository},
    },
    PrivateRoomService, ensure_active_actor, replace_room,
    sub_room_membership::LoadedSubRoom,
    sub_rooms::refused,
};

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
        // 已经归档了：上一次聊天服务器上没锁成的还排着，后台接着做。
        let Some(archived) =
            archived_sub_room(sub_room.room()).map_err(|error| domain(OPERATION, &error))?
        else {
            return Ok(sub_room);
        };
        let outcome = self
            .sub_rooms
            .archive_sub_room(&SubRoomArchival {
                room: &archived,
                expected_version: sub_room.room().version(),
                parent_catalog_id: request.parent_catalog_id,
                by: Some(actor),
                reason: SubRoomArchiveReason::Manual,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        stored(outcome, OPERATION)?;
        self.sync_sub_rooms_under(&parent).await;
        replace_room(sub_room, archived, OPERATION)
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
