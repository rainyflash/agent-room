//! 申请和审批。同一个人对同一间房同时只能有一个在等的申请，被拒的 24 小时内不能再申请；房主和
//! 管理员（有“管理”这一位的人）看得到、批准或拒绝。批准以后以房主的名义邀请，申请人的网页照接受
//! 邀请那一套自己进来。

use agent_room_domain::{
    ids::RoomCatalogId,
    private_rooms::{PrivateRoomCapability, PrivateRoomMember},
    room_join_requests::{
        JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS, JOIN_REQUEST_RETENTION_MILLIS, JoinRequestMessage,
        JoinRequestStatus,
    },
    room_listing::RoomJoinMode,
    time::UtcMillis,
};

use super::{
    AnswerJoinRequest, JoinPublicRoom, JoinRequestState, PublicJoinFailure, PublicJoinFailureKind,
    PublicJoinResult, PublicRoomJoinService, RequestToJoin, current_membership, ensure_not_banned,
    repository,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{
        JoinRequestCreateOutcome, JoinRequestRecord, JoinRequestSweep, PendingJoinRequest,
        PrivateRoomSnapshot,
    },
    room_people::shown_display_name,
};

impl PublicRoomJoinService {
    pub(super) async fn request_internal(
        &self,
        request: RequestToJoin,
    ) -> PublicJoinResult<JoinRequestState> {
        const OPERATION: &str = "private_room.join_request.create";
        self.ensure_session(&request.actor, OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let requester = request.actor.principal_id;
        if let Some(membership) = current_membership(&snapshot, requester) {
            return Ok(JoinRequestState::Member(membership));
        }
        let listing = self.listed(&snapshot, OPERATION).await?;
        ensure_not_banned(&snapshot, requester, OPERATION)?;
        match listing.settings.join_mode() {
            // 申请框开着的时候房主改成了直接进：不用等，直接进来。
            RoomJoinMode::Open => {
                let membership = self
                    .invite_as_owner(&snapshot, &listing, requester, OPERATION)
                    .await?;
                return Ok(JoinRequestState::Member(membership));
            }
            RoomJoinMode::InviteOnly => {
                return Err(PublicJoinFailure::new(
                    OPERATION,
                    PublicJoinFailureKind::InviteOnly,
                ));
            }
            RoomJoinMode::Request => {}
        }
        let message = JoinRequestMessage::parse(&request.message).map_err(|_| {
            PublicJoinFailure::new(OPERATION, PublicJoinFailureKind::InvalidRequest)
        })?;
        if let Some(pending) = self
            .requests
            .pending_join_request(request.catalog_id, requester)
            .await
            .map_err(|error| repository(OPERATION, &error))?
        {
            return Ok(JoinRequestState::Pending {
                requested_at: pending.requested_at,
            });
        }
        let now = self.clock.now();
        self.ensure_not_recently_rejected(&request, now, OPERATION)
            .await?;
        let record = JoinRequestRecord {
            catalog_id: request.catalog_id,
            requester,
            message,
            status: JoinRequestStatus::Pending,
            requested_at: now,
            decided_at: None,
            decided_by: None,
        };
        let requested_at = match self
            .requests
            .create_join_request(&record)
            .await
            .map_err(|error| repository(OPERATION, &error))?
        {
            JoinRequestCreateOutcome::Created => now,
            JoinRequestCreateOutcome::AlreadyPending(existing) => existing.requested_at,
        };
        Ok(JoinRequestState::Pending { requested_at })
    }

    async fn ensure_not_recently_rejected(
        &self,
        request: &RequestToJoin,
        now: UtcMillis,
        operation: &'static str,
    ) -> PublicJoinResult<()> {
        let Some(rejected_at) = self
            .requests
            .last_rejected_at(request.catalog_id, request.actor.principal_id)
            .await
            .map_err(|error| repository(operation, &error))?
        else {
            return Ok(());
        };
        let retry_at = UtcMillis::new(
            rejected_at
                .value()
                .saturating_add(JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS),
        )
        .map_err(|_| PublicJoinFailure::new(operation, PublicJoinFailureKind::Internal))?;
        if now < retry_at {
            Err(PublicJoinFailure::recently_rejected(operation, retry_at))
        } else {
            Ok(())
        }
    }

    pub(super) async fn withdraw_internal(&self, request: JoinPublicRoom) -> PublicJoinResult<()> {
        const OPERATION: &str = "private_room.join_request.withdraw";
        self.ensure_session(&request.actor, OPERATION)?;
        self.requests
            .decide_join_request(
                request.catalog_id,
                request.actor.principal_id,
                JoinRequestStatus::Withdrawn,
                None,
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    pub(super) async fn pending_requests_internal(
        &self,
        request: JoinPublicRoom,
    ) -> PublicJoinResult<Vec<PendingJoinRequest>> {
        const OPERATION: &str = "private_room.join_request.list";
        self.managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let pending = self
            .requests
            .pending_join_requests(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        // 和房间里的名字一样，昵称里像邮箱的只留 @ 前面那段。
        Ok(pending
            .into_iter()
            .map(|pending| PendingJoinRequest {
                requester_display_name: shown_display_name(&pending.requester_display_name),
                ..pending
            })
            .collect())
    }

    pub(super) async fn answer_internal(
        &self,
        request: AnswerJoinRequest,
    ) -> PublicJoinResult<JoinRequestStatus> {
        const OPERATION: &str = "private_room.join_request.answer";
        let snapshot = self
            .managed_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let not_found = || PublicJoinFailure::new(OPERATION, PublicJoinFailureKind::NotFound);
        self.requests
            .pending_join_request(request.catalog_id, request.requester)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(not_found)?;
        let status = if request.approve {
            self.admit_requester(&snapshot, &request, OPERATION).await?;
            JoinRequestStatus::Approved
        } else {
            JoinRequestStatus::Rejected
        };
        let decided = self
            .requests
            .decide_join_request(
                request.catalog_id,
                request.requester,
                status,
                Some(request.actor.principal_id),
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        if decided {
            Ok(status)
        } else {
            // 刚刚被别的管理员处理了，或者申请人自己撤回了。
            Err(PublicJoinFailure::new(
                OPERATION,
                PublicJoinFailureKind::Conflict,
            ))
        }
    }

    /// 批准：还不在房间里的以房主的名义邀请。被封禁的不能批准，要先解封。
    async fn admit_requester(
        &self,
        snapshot: &PrivateRoomSnapshot,
        request: &AnswerJoinRequest,
        operation: &'static str,
    ) -> PublicJoinResult<()> {
        if current_membership(snapshot, request.requester).is_some() {
            return Ok(());
        }
        ensure_not_banned(snapshot, request.requester, operation)?;
        // 改回私密时在等的申请一起作废，所以这里不公开了就是刚刚改的。
        let listing = self.listed(snapshot, operation).await.map_err(|failure| {
            if failure.kind() == PublicJoinFailureKind::NotFound {
                PublicJoinFailure::new(operation, PublicJoinFailureKind::Conflict)
            } else {
                failure
            }
        })?;
        self.invite_as_owner(snapshot, &listing, request.requester, operation)
            .await?;
        Ok(())
    }

    /// 登录有效、在房间里、有管理权限、房间还在使用中。不在房间里的和没有这个房间一样回答。
    async fn managed_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PublicJoinResult<PrivateRoomSnapshot> {
        self.ensure_session(actor, operation)?;
        let snapshot = self.load(catalog_id, operation).await?;
        let room = snapshot.room();
        if !room
            .member(actor.principal_id)
            .is_some_and(PrivateRoomMember::has_joined)
        {
            return Err(PublicJoinFailure::new(
                operation,
                PublicJoinFailureKind::NotFound,
            ));
        }
        if !room.allows(actor.principal_id, PrivateRoomCapability::Manage) {
            return Err(PublicJoinFailure::new(
                operation,
                PublicJoinFailureKind::Forbidden,
            ));
        }
        Ok(snapshot)
    }

    pub(super) async fn sweep_internal(&self) -> PublicJoinResult<JoinRequestSweep> {
        const OPERATION: &str = "private_room.join_request.sweep";
        let now = self.clock.now();
        let decided_before =
            UtcMillis::new(now.value().saturating_sub(JOIN_REQUEST_RETENTION_MILLIS))
                .map_err(|_| PublicJoinFailure::new(OPERATION, PublicJoinFailureKind::Internal))?;
        self.requests
            .sweep_join_requests(now, decided_before)
            .await
            .map_err(|error| repository(OPERATION, &error))
    }
}
