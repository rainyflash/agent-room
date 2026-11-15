//! 子房间第 2d-2 步（`specs/redesign/sub-rooms.md` 的“门”“进门”）：人申请进要批准的子房间、撤回，
//! 管得了这一间的人（这一间的管理员、父房间的房主和管理员）看在等的申请、批准或拒绝。
//!
//! 申请和公开房间的申请是同一张表、同一套规则：同一个人对同一间只有一个在等的，没人处理也不作废，
//! 被拒的 24 小时内不能再申请。不一样的是批准：公开房间以房主的名义邀请，子房间照它自己的规则直接
//! 把人记成已加入（和他自己点“加入”同一段，[`PrivateRoomService::admit_into_sub_room`]）。
//!
//! 在等的申请什么时候作废，由存储在改门、改父房间的事务里做（换完的门不收申请、他不在父房间里了），
//! 每小时的清理再兜一遍（子房间归档了的也在那里作废）。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomLifecycleStatus,
    room_join_requests::{
        JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS, JoinRequestMessage, JoinRequestStatus,
    },
    sub_rooms::{
        SubRoomEntryRefusal, authorize_sub_room_admission, authorize_sub_room_join,
        manages_sub_room,
    },
    time::UtcMillis,
};

use crate::{
    ports::{JoinRequestRecord, PendingJoinRequest, SubRoomJoinRequestOutcome},
    room_people::shown_display_name,
};

use super::{
    super::{
        AnswerSubRoomJoinRequest, PrivateRoomFailureKind, PrivateRoomFailureStage,
        PrivateRoomResult, RequestSubRoomJoin, SubRoomJoinState, SubRoomMembershipAction,
        SubRoomRefusal,
        failure::{domain, failure, repository},
    },
    PrivateRoomService, ensure_active_actor,
    sub_room_membership::{LoadedSubRoom, SubRoomEntrant, entry_refused},
    sub_rooms::{not_found, refused},
};

impl PrivateRoomService {
    pub(super) async fn request_sub_room_join_internal(
        &self,
        request: RequestSubRoomJoin,
    ) -> PrivateRoomResult<SubRoomJoinState> {
        const OPERATION: &str = "sub_room.join_request.create";
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
        match authorize_sub_room_join(parent.room(), sub_room.room(), actor, link.access) {
            // 不用等：这一间没有门（申请框开着的时候门拆了也走到这里）、他是父房间的房主或管理员、
            // 他本来就在里面。和他自己点“加入”一样。
            Ok(_) => {
                self.join_sub_room_internal(SubRoomMembershipAction {
                    actor: request.actor,
                    parent_catalog_id: request.parent_catalog_id,
                    catalog_id: request.catalog_id,
                })
                .await?;
                return Ok(SubRoomJoinState::Joined);
            }
            Err(SubRoomEntryRefusal::ApprovalRequired) => {}
            Err(refusal) => return Err(entry_refused(OPERATION, refusal)),
        }
        let message = JoinRequestMessage::parse(&request.message)
            .map_err(|error| domain(OPERATION, &error))?;
        if let Some(pending) = self
            .join_requests
            .pending_join_request(request.catalog_id, actor)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
        {
            return Ok(SubRoomJoinState::Pending {
                requested_at: pending.requested_at,
            });
        }
        self.ensure_not_recently_rejected(request.catalog_id, actor, now, OPERATION)
            .await?;
        let record = JoinRequestRecord {
            catalog_id: request.catalog_id,
            requester: actor,
            message,
            status: JoinRequestStatus::Pending,
            requested_at: now,
            decided_at: None,
            decided_by: None,
        };
        match self
            .sub_rooms
            .request_sub_room_join(request.parent_catalog_id, &record)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
        {
            SubRoomJoinRequestOutcome::Created => {
                Ok(SubRoomJoinState::Pending { requested_at: now })
            }
            SubRoomJoinRequestOutcome::AlreadyPending(requested_at) => {
                Ok(SubRoomJoinState::Pending { requested_at })
            }
            // 读到门以后、存之前门换了（拆了、改成只能请进来、藏起来了）或者归档了：请他看过新的再说。
            SubRoomJoinRequestOutcome::Closed => Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::Conflict,
            )),
            // 读到他在父房间里以后、存之前他被移出去了：和不在父房间里的人一样回。
            SubRoomJoinRequestOutcome::NotInParent => Err(not_found(OPERATION)),
        }
    }

    /// 被拒还不到 24 小时的不能再申请，和公开房间一样。
    async fn ensure_not_recently_rejected(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
        now: UtcMillis,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let Some(rejected_at) = self
            .join_requests
            .last_rejected_at(catalog_id, requester)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?
        else {
            return Ok(());
        };
        let retry_at = UtcMillis::new(
            rejected_at
                .value()
                .saturating_add(JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS),
        )
        .map_err(|error| domain(operation, &error))?;
        if now < retry_at {
            Err(refused(operation, SubRoomRefusal::RecentlyRejected).retrying_at(retry_at))
        } else {
            Ok(())
        }
    }

    /// 撤回只看这是不是他自己的申请、这间子房间是不是挂在地址里的房间下面，不看他现在找不找得到它：
    /// 申请以后这一间藏起来了的，他也撤得回来。
    pub(super) async fn withdraw_sub_room_join_request_internal(
        &self,
        request: SubRoomMembershipAction,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "sub_room.join_request.withdraw";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let link = self
            .sub_rooms
            .sub_room_link(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
            .filter(|link| link.parent_catalog_id == request.parent_catalog_id)
            .ok_or_else(|| not_found(OPERATION))?;
        self.join_requests
            .decide_join_request(
                link.catalog_id,
                request.actor.principal_id,
                JoinRequestStatus::Withdrawn,
                None,
                now,
            )
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(())
    }

    pub(super) async fn sub_room_join_requests_internal(
        &self,
        request: SubRoomMembershipAction,
    ) -> PrivateRoomResult<Vec<PendingJoinRequest>> {
        const OPERATION: &str = "sub_room.join_request.list";
        let actor = request.actor.principal_id;
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
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
        if !manages_sub_room(parent.room(), sub_room.room(), actor) {
            return Err(refused(OPERATION, SubRoomRefusal::ManagersOnly));
        }
        let pending = self
            .join_requests
            .pending_join_requests(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(shown_requests(pending))
    }

    /// 批准：照子房间的规则把他记成已加入（不看门，门就是让管这一间的人开的），在 Matrix 上邀请
    /// 他，再把申请记成批准。和公开房间一样先进人、后记申请：反过来的话进人没成，申请却已经不在等了，
    /// 再点一次也找不到它。
    pub(super) async fn answer_sub_room_join_request_internal(
        &self,
        request: AnswerSubRoomJoinRequest,
    ) -> PrivateRoomResult<JoinRequestStatus> {
        const OPERATION: &str = "sub_room.join_request.answer";
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
        if !manages_sub_room(parent.room(), sub_room.room(), actor) {
            return Err(refused(OPERATION, SubRoomRefusal::ManagersOnly));
        }
        let active = PrivateRoomLifecycleStatus::Active;
        if parent.room().status() != active || sub_room.room().status() != active {
            return Err(refused(OPERATION, SubRoomRefusal::Archived));
        }
        self.join_requests
            .pending_join_request(request.catalog_id, request.requester)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
            .ok_or_else(|| not_found(OPERATION))?;
        let status = if request.approve {
            // 被封禁着的要先解封（说的是“他”）；不在父房间里了的和没有这个申请一样回（他离开父房间时
            // 申请就作废了）。
            let permissions =
                authorize_sub_room_admission(parent.room(), sub_room.room(), request.requester)
                    .map_err(|refusal| match refusal {
                        SubRoomEntryRefusal::Banned => {
                            refused(OPERATION, SubRoomRefusal::TargetBanned)
                        }
                        other => entry_refused(OPERATION, other),
                    })?;
            let matrix_user = self
                .principal_matrix_user(request.requester, OPERATION)
                .await?;
            self.admit_into_sub_room(
                request.parent_catalog_id,
                sub_room,
                &SubRoomEntrant {
                    principal: request.requester,
                    matrix_user: &matrix_user,
                    permissions,
                },
                now,
                OPERATION,
            )
            .await?;
            JoinRequestStatus::Approved
        } else {
            JoinRequestStatus::Rejected
        };
        let decided = self
            .join_requests
            .decide_join_request(
                request.catalog_id,
                request.requester,
                status,
                Some(actor),
                now,
            )
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        if decided {
            Ok(status)
        } else {
            // 刚刚被别的管理员处理了，或者他自己撤回了。
            Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::Conflict,
            ))
        }
    }
}

/// 给管这一间的人（和管它的网络 Agent）看的在等的申请：和房间里的名字一样，昵称里像邮箱的只留 @
/// 前面那段。
pub(super) fn shown_requests(pending: Vec<PendingJoinRequest>) -> Vec<PendingJoinRequest> {
    pending
        .into_iter()
        .map(|pending| PendingJoinRequest {
            requester_display_name: shown_display_name(&pending.requester_display_name),
            ..pending
        })
        .collect()
}
