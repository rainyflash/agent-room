//! 子房间（`specs/redesign/sub-rooms.md` 第 2a 步）：在一间房下面建子房间、读名单，和一间房里
//! 子房间的三项设置（`specs/redesign/room-roles.md` R4）。人加入、退出、改名和换门（2b、2d）在
//! 旁边的 `sub_room_membership`。
//!
//! 子房间自己是一间私人房间，建 Matrix 房间、存成员都走私人房间那一套；多出来的是它挂在谁下面、
//! 谁建的、谁在管。口令、邀请链接、公开、往外请人这些只有父房间有，各自的用例读到房间以后先看
//! [`ensure_not_sub_room`]。

use std::collections::BTreeSet;

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{
        PrivateRoom, PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus,
    },
    room_join_requests::JoinRequestStatus,
    room_listing::RoomJoinMode,
    rooms::{
        RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus, RoomCatalogVisibility,
    },
    sub_room_archive::{RestoreStanding, SubRoomArchiveReason, restore_allowed},
    sub_rooms::{
        SubRoomCreationRefusal, SubRoomDoor, SubRoomPolicy, authorize_sub_room_creation,
        new_sub_room, runs_parent_room, sub_room_name_key, validate_sub_room_name,
    },
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryErrorKind,
    ports::{
        MatrixCreateRoom, MatrixRoomId, MatrixRoomPowerProfile, MatrixRoomPreset,
        MatrixRoomVisibility, MatrixUserId, NewSubRoom, PendingJoinRequest, PortFuture,
        PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment, PrivateRoomSnapshot,
        SubRoomCategory, SubRoomCreationBlock, SubRoomCreationCheck, SubRoomEntry,
        SubRoomParentFacts, SubRoomPolicyRecord, SubRoomPrivacy,
    },
};

use super::{
    super::{
        AnswerSubRoomJoinRequest, ArchiveSubRoom, ChangeSubRoomManager, ChangeSubRoomPolicy,
        CreateSubRoom, CreateSubRoomCategory, DeleteSubRoomCategory, GovernSubRoomMember,
        InspectSubRoomPolicy, ListSubRooms, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomResult, RequestSubRoomJoin, RestoreSubRoom,
        ReviseSubRoom, ReviseSubRoomCategory, SubRoomJoinState, SubRoomList,
        SubRoomMembershipAction, SubRoomPolicyView, SubRoomRefusal, SubRoomView,
        failure::{domain, failure, matrix, repository},
        sub_room_privacy::{SubRoomPrivacyFailure, encrypt_sub_rooms, mark_open_sub_rooms_listed},
    },
    PrivateRoomService, active_room_actor_matrix_user, ensure_active_actor, private_room_alias,
};

pub trait SubRoomUseCases: Send + Sync {
    /// 在一间房下面建子房间。建好以后建的人已经在里面，是这一间的管理员。
    fn create_sub_room(
        &self,
        request: CreateSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 一间房下面看的人看得到的子房间。父房间里已加入的人才能读。
    fn list_sub_rooms(
        &self,
        request: ListSubRooms,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomList>>;

    /// 谁能建子房间、Agent 能不能建、自动归档开没开。父房间里已加入的人都能读。
    fn sub_room_policy(
        &self,
        request: InspectSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>>;

    /// 改这三项，只有房主能改。
    fn change_sub_room_policy(
        &self,
        request: ChangeSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>>;

    /// 人自己进一间子房间。没有门的，父房间里已加入的人点一下就进；有门的只有父房间的房主和管理员
    /// 能这样进。已经在里面的再来一次不算错。
    fn join_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 人从一间子房间退出，他的 Agent 跟着出去。本来就不在里面的不算错。
    fn leave_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 改子房间的名字、用途、门和“不在名单里显示”。管得了这一间的人能改。
    fn revise_sub_room(
        &self,
        request: ReviseSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 把一间子房间归档（2e-2）：只能看、不能说，名字和名额让出来，成员、门、分类、管理员原样
    /// 留着。管得了这一间的人能做。已经归档的原样回。
    fn archive_sub_room(
        &self,
        request: ArchiveSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 把归档了的一间子房间恢复（2e-2）：原来的人接着说，被禁言着的照旧说不了。能归档的人能恢复，
    /// 自动归档的原来在里面的人也能；跟着父房间归档的回不来。没归档的原样回。
    fn restore_sub_room(
        &self,
        request: RestoreSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 申请进一间要批准的子房间，可以留一句话。不用批准的（没有门了、父房间的房主和管理员、本来
    /// 就在里面的）直接进；已经有一个在等的就原样回那一个，留言不改。
    fn request_sub_room_join(
        &self,
        request: RequestSubRoomJoin,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomJoinState>>;

    /// 撤回自己在等的申请，没有也算成功。
    fn withdraw_sub_room_join_request(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 管得了这一间的人看在等的申请，先来的在前。
    fn sub_room_join_requests(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PendingJoinRequest>>>;

    /// 管得了这一间的人批准或拒绝一个申请。批准的直接记成已加入。
    fn answer_sub_room_join_request(
        &self,
        request: AnswerSubRoomJoinRequest,
    ) -> PortFuture<'_, PrivateRoomResult<JoinRequestStatus>>;

    /// 管得了这一间的人把父房间里的人直接请进来，哪种门都行。已经在里面的只补一遍邀请。
    fn admit_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 管得了这一间的人把一个人移出这一间。不在里面的也算成功。
    fn remove_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 在这一间里封禁一个人：之后他申请不了、也请不进来，先解封。
    fn ban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 解封：回到“已移出”。没封着的也算成功。
    fn unban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 设、撤一位子房间管理员。已经是那样的也算成功。
    fn change_sub_room_manager(
        &self,
        request: ChangeSubRoomManager,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;

    /// 建一个子房间的分类，排在最后（2e-1）。房间的房主和管理员能建。回这间房此刻所有的分类。
    fn create_sub_room_category(
        &self,
        request: CreateSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>>;

    /// 给分类改名、挪位置。
    fn revise_sub_room_category(
        &self,
        request: ReviseSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>>;

    /// 删一个分类，里面的子房间回到“未分类”。已经删掉的也算成功。
    fn delete_sub_room_category(
        &self,
        request: DeleteSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>>;
}

impl SubRoomUseCases for PrivateRoomService {
    fn create_sub_room(
        &self,
        request: CreateSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.create_sub_room_internal(request))
    }

    fn list_sub_rooms(
        &self,
        request: ListSubRooms,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomList>> {
        Box::pin(self.list_sub_rooms_internal(request))
    }

    fn sub_room_policy(
        &self,
        request: InspectSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>> {
        Box::pin(self.sub_room_policy_internal(request))
    }

    fn change_sub_room_policy(
        &self,
        request: ChangeSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>> {
        Box::pin(self.change_sub_room_policy_internal(request))
    }

    fn join_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.join_sub_room_internal(request))
    }

    fn leave_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.leave_sub_room_internal(request))
    }

    fn revise_sub_room(
        &self,
        request: ReviseSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.revise_sub_room_internal(request))
    }

    fn archive_sub_room(
        &self,
        request: ArchiveSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.archive_sub_room_internal(request))
    }

    fn restore_sub_room(
        &self,
        request: RestoreSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.restore_sub_room_internal(request))
    }

    fn request_sub_room_join(
        &self,
        request: RequestSubRoomJoin,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomJoinState>> {
        Box::pin(self.request_sub_room_join_internal(request))
    }

    fn withdraw_sub_room_join_request(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.withdraw_sub_room_join_request_internal(request))
    }

    fn sub_room_join_requests(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PendingJoinRequest>>> {
        Box::pin(self.sub_room_join_requests_internal(request))
    }

    fn answer_sub_room_join_request(
        &self,
        request: AnswerSubRoomJoinRequest,
    ) -> PortFuture<'_, PrivateRoomResult<JoinRequestStatus>> {
        Box::pin(self.answer_sub_room_join_request_internal(request))
    }

    fn admit_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.admit_sub_room_member_internal(request))
    }

    fn remove_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.remove_sub_room_member_internal(request))
    }

    fn ban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.ban_sub_room_member_internal(request))
    }

    fn unban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.unban_sub_room_member_internal(request))
    }

    fn change_sub_room_manager(
        &self,
        request: ChangeSubRoomManager,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.change_sub_room_manager_internal(request))
    }

    fn create_sub_room_category(
        &self,
        request: CreateSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>> {
        Box::pin(self.create_sub_room_category_internal(request))
    }

    fn revise_sub_room_category(
        &self,
        request: ReviseSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>> {
        Box::pin(self.revise_sub_room_category_internal(request))
    }

    fn delete_sub_room_category(
        &self,
        request: DeleteSubRoomCategory,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<SubRoomCategory>>> {
        Box::pin(self.delete_sub_room_category_internal(request))
    }
}

/// 建子房间时在 Matrix 上要做的：建房的请求，和每个人能不能发言。
struct PreparedSubRoom {
    matrix: PrivateMatrixRoomCreation,
    speaking_assignments: Vec<PrivateMatrixSpeakingAssignment>,
    /// 拿到了发言级别的人。存好以后再看一眼他们是不是被禁言着。
    speakers: Vec<(PrincipalId, MatrixUserId)>,
}

impl PrivateRoomService {
    async fn create_sub_room_internal(
        &self,
        request: CreateSubRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "sub_room.create";
        let now = self.clock.now();
        let creator_matrix_user = active_room_actor_matrix_user(&request.actor, now, OPERATION)?;
        self.ensure_sub_room_creator(&request, OPERATION).await?;
        validate_sub_room_name(&request.name).map_err(|error| domain(OPERATION, &error))?;
        let door = SubRoomDoor::new(request.access, request.hidden)
            .map_err(|error| domain(OPERATION, &error))?;
        let creator = request.actor.principal_id();
        let parent = self
            .load_as_member(request.parent_catalog_id, creator, OPERATION)
            .await?;
        if parent.parent_catalog_id().is_some() {
            return Err(refused(OPERATION, SubRoomRefusal::Nested));
        }
        if let Some(existing) = self.find(request.catalog_id, OPERATION).await? {
            let existing = self
                .ensure_same_sub_room(&request, existing, OPERATION)
                .await?;
            // 上一次存好以后没跟上父房间的，这次补上。
            self.follow_parent_privacy(&request, now, OPERATION).await?;
            return Ok(existing);
        }
        let facts = self
            .authorized_parent_facts(&request, &parent, OPERATION)
            .await?;
        // 有门的一律加密；没有门的跟父房间。
        let encrypted = facts.end_to_end_encrypted || door.has_door();

        // 目录的规则和重名、上限先查：查不过的别等 Matrix 房间建好了才发现。
        let catalog = sub_room_catalog(&request, &parent, OPERATION)?;
        let check = SubRoomCreationCheck {
            catalog_id: request.catalog_id,
            parent_catalog_id: request.parent_catalog_id,
            name_key: sub_room_name_key(&request.name),
            category_id: request.category_id,
            created_by: creator,
            created_by_agent: request.created_by_agent,
            at: now,
        };
        self.ensure_sub_room_creatable(&check, OPERATION).await?;

        let room = new_sub_room(request.catalog_id, parent.room(), creator)
            .map_err(|error| domain(OPERATION, &error))?;
        let prepared = self
            .prepare_sub_room(
                &request,
                &parent,
                &room,
                creator_matrix_user,
                encrypted,
                OPERATION,
            )
            .await?;
        let matrix_room_id = self
            .matrix_provisioner
            .create(&prepared.matrix)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.matrix
            .set_speaking_batch(&matrix_room_id, &prepared.speaking_assignments)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        let snapshot = self
            .build_snapshot(
                request.catalog_id,
                catalog,
                room,
                &matrix_room_id,
                OPERATION,
            )?
            .under(request.parent_catalog_id);
        let sub_room = new_sub_room_record(&request, &parent, snapshot, door, encrypted, now);
        let stored = self.store_sub_room(&request, sub_room, OPERATION).await?;
        self.hold_fresh_mutes(&request, &matrix_room_id, &prepared.speakers, OPERATION)
            .await?;
        self.follow_parent_privacy(&request, now, OPERATION).await?;
        Ok(stored)
    }

    /// 建的当口父房间可能刚改回私密、刚公开（`sub_room_privacy`）：存好以后再读一遍父房间，照它补上
    /// 加密、写下这一刻。改回私密那边先改父房间的标记、再读子房间；公开那边存好设置以后再读一遍
    /// 子房间。所以两边总有一边看得到对方。
    async fn follow_parent_privacy(
        &self,
        request: &CreateSubRoom,
        at: UtcMillis,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let persistence =
            |error| repository(operation, PrivateRoomFailureStage::Persistence, &error);
        let parent = request.parent_catalog_id;
        let this_one: Vec<SubRoomPrivacy> = self
            .listings
            .sub_room_privacy(parent)
            .await
            .map_err(persistence)?
            .into_iter()
            .filter(|sub_room| sub_room.catalog_id == request.catalog_id)
            .collect();
        if this_one.is_empty() {
            return Ok(());
        }
        let encrypted = self
            .sub_rooms
            .sub_room_parent_facts(parent)
            .await
            .map_err(persistence)?
            .is_some_and(|facts| facts.end_to_end_encrypted);
        if encrypted {
            encrypt_sub_rooms(
                self.listings.as_ref(),
                self.listing_matrix.as_ref(),
                &this_one,
            )
            .await
            .map_err(|failure| privacy_failure(operation, failure))?;
        }
        let listed = self
            .listings
            .listing(parent)
            .await
            .map_err(persistence)?
            .is_some_and(|listing| listing.settings.listed());
        if listed {
            mark_open_sub_rooms_listed(
                self.listings.as_ref(),
                self.listing_matrix.as_ref(),
                &this_one,
                at,
            )
            .await
            .map_err(|failure| privacy_failure(operation, failure))?;
        }
        Ok(())
    }

    /// 建的当口落下来的禁言看不到这间还没存的子房间：存好以后再看一眼，被禁言着的把发言级别收回。
    async fn hold_fresh_mutes(
        &self,
        request: &CreateSubRoom,
        matrix_room_id: &MatrixRoomId,
        speakers: &[(PrincipalId, MatrixUserId)],
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        for (principal, user) in speakers {
            if self
                .muted_in(request.parent_catalog_id, *principal, operation)
                .await?
            {
                self.matrix
                    .set_speaking(matrix_room_id, user, false)
                    .await
                    .map_err(|error| matrix(operation, error))?;
            }
        }
        Ok(())
    }

    /// 父房间加不加密、三项设置，顺带照设置看建的人此刻能不能建。
    async fn authorized_parent_facts(
        &self,
        request: &CreateSubRoom,
        parent: &PrivateRoomSnapshot,
        operation: &'static str,
    ) -> PrivateRoomResult<SubRoomParentFacts> {
        let facts = self
            .sub_rooms
            .sub_room_parent_facts(request.parent_catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?
            .ok_or_else(|| not_found(operation))?;
        authorize_sub_room_creation(
            parent.room(),
            request.actor.principal_id(),
            facts.policy,
            request.created_by_agent.is_some(),
        )
        .map_err(|refusal| creation_refused(operation, refusal))?;
        Ok(facts)
    }

    /// 重名、到上限、建得太快的，在建 Matrix 房间之前就说。
    async fn ensure_sub_room_creatable(
        &self,
        check: &SubRoomCreationCheck,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let blocked = self
            .sub_rooms
            .sub_room_creation_block(check)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?;
        match blocked {
            Some(block) => Err(creation_blocked(operation, block)),
            None => Ok(()),
        }
    }

    /// 把建好的子房间存下来。存的时候撞上已经有的房间号，多半是同一个请求的重试抢先存好了。
    async fn store_sub_room(
        &self,
        request: &CreateSubRoom,
        sub_room: NewSubRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        match self.sub_rooms.create_sub_room(&sub_room).await {
            Ok(None) => Ok(sub_room.snapshot),
            Ok(Some(block)) => Err(creation_blocked(operation, block)),
            Err(error) if error.kind() == RepositoryErrorKind::Conflict => {
                let existing = self
                    .find(request.catalog_id, operation)
                    .await?
                    .ok_or_else(|| conflict(operation))?;
                self.ensure_same_sub_room(request, existing, operation)
                    .await
            }
            Err(error) => Err(repository(
                operation,
                PrivateRoomFailureStage::Persistence,
                &error,
            )),
        }
    }

    /// 人建不带建的 Agent；Agent 替主人建必须带，且是操作者名下还在用的 Agent。
    async fn ensure_sub_room_creator(
        &self,
        request: &CreateSubRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let agent_id = match (request.actor.is_agent(), request.created_by_agent) {
            (false, None) => return Ok(()),
            (true, Some(agent_id)) => agent_id,
            _ => {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::InvalidRequest,
                )
                .rejecting("created_by_agent"));
            }
        };
        let owned = self
            .agents
            .owns_active_agent(request.actor.principal_id(), agent_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?;
        if owned {
            Ok(())
        } else {
            Err(failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ))
        }
    }

    /// 读一间房，看的人得是里面已加入的成员；不是的话这间房对他来说不存在。
    pub(super) async fn load_as_member(
        &self,
        catalog_id: RoomCatalogId,
        viewer: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let snapshot = self.load(catalog_id, operation).await?;
        if has_joined(snapshot.room(), viewer) {
            Ok(snapshot)
        } else {
            Err(not_found(operation))
        }
    }

    /// 同一个房间号已经建过：得是同一间房下面、同一个人（同一个 Agent）建的，名字、用途和门都
    /// 一样，不然是另一个请求撞了号。
    async fn ensure_same_sub_room(
        &self,
        request: &CreateSubRoom,
        existing: PrivateRoomSnapshot,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let link = self
            .sub_rooms
            .sub_room_link(request.catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?;
        let same = existing.parent_catalog_id() == Some(request.parent_catalog_id)
            && existing.catalog().name() == request.name
            && existing.catalog().description() == request.description
            && link.is_some_and(|link| {
                link.created_by == request.actor.principal_id()
                    && link.created_by_agent == request.created_by_agent
                    && link.access == request.access
                    && link.hidden == request.hidden
            });
        if same {
            Ok(existing)
        } else {
            Err(conflict(operation))
        }
    }

    /// Matrix 上的子房间：邀请父房间的房主和建的人，保留期跟父房间；加不加密由调用方说（有门的
    /// 一律加密，没有门的跟父房间）。
    async fn prepare_sub_room(
        &self,
        request: &CreateSubRoom,
        parent: &PrivateRoomSnapshot,
        room: &PrivateRoom,
        creator_matrix_user: Option<MatrixUserId>,
        end_to_end_encrypted: bool,
        operation: &'static str,
    ) -> PrivateRoomResult<PreparedSubRoom> {
        let creator = request.actor.principal_id();
        let owner = room.owner_principal_id();
        let mut invites = Vec::new();
        let mut speaking_assignments = Vec::new();
        let mut speakers = Vec::new();
        let mut seen = BTreeSet::new();
        for member in room.members() {
            let principal = member.principal_id();
            let user = if principal == creator {
                creator_matrix_user.clone()
            } else {
                self.owner_matrix_user(owner, operation).await?
            };
            // 网络 Agent 的合成主体没有自己的 Matrix 账号：不邀请它，由它的 Agent 代表进房间。
            let Some(user) = user else {
                continue;
            };
            if !seen.insert(user.as_str().to_owned()) {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Directory,
                    PrivateRoomFailureKind::Conflict,
                ));
            }
            // 在父房间里被禁言着的人，新建的子房间里也不给发言级别：不然建一间就绕过去了。
            let speaks = member.permissions().speak()
                && !self
                    .muted_in(request.parent_catalog_id, principal, operation)
                    .await?;
            if speaks {
                speakers.push((principal, user.clone()));
            }
            speaking_assignments.push(PrivateMatrixSpeakingAssignment::new(user.clone(), speaks));
            invites.push(user);
        }
        for reader in &self.trusted_matrix_readers {
            if seen.insert(reader.as_str().to_owned()) {
                speaking_assignments
                    .push(PrivateMatrixSpeakingAssignment::new(reader.clone(), false));
                invites.push(reader.clone());
            }
        }

        let alias = private_room_alias(request.catalog_id, !end_to_end_encrypted, operation)?;
        let mut matrix_request = MatrixCreateRoom::new(
            Some(request.name.clone()),
            (!request.description.is_empty()).then(|| request.description.clone()),
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::PrivateChat,
            false,
            invites,
        )
        .map_err(|error| domain(operation, &error))?
        .with_alias_localpart(alias.clone())
        .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
        if end_to_end_encrypted {
            matrix_request = matrix_request.with_end_to_end_encryption();
        }
        if let Some(retention_days) = parent.catalog().retention_days() {
            matrix_request = matrix_request
                .with_retention_days(retention_days)
                .map_err(|error| domain(operation, &error))?;
        }
        let matrix = if end_to_end_encrypted {
            PrivateMatrixRoomCreation::new(matrix_request, alias)
        } else {
            PrivateMatrixRoomCreation::new_public(matrix_request, alias)
        }
        .map_err(|error| domain(operation, &error))?;
        Ok(PreparedSubRoom {
            matrix,
            speaking_assignments,
            speakers,
        })
    }

    /// 父房间房主的 Matrix 账号；房主是网络 Agent 的合成主体时没有。
    async fn owner_matrix_user(
        &self,
        owner: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<Option<MatrixUserId>> {
        let agents =
            self.agents.principal_agents(owner).await.map_err(|error| {
                repository(operation, PrivateRoomFailureStage::Directory, &error)
            })?;
        if agents.network {
            return Ok(None);
        }
        self.principal_matrix_user(owner, operation).await.map(Some)
    }

    async fn list_sub_rooms_internal(
        &self,
        request: ListSubRooms,
    ) -> PrivateRoomResult<SubRoomList> {
        const OPERATION: &str = "sub_room.list";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let viewer = request.actor.principal_id;
        let asked = self.load(request.catalog_id, OPERATION).await?;
        let parent = match asked.parent_catalog_id() {
            Some(parent) => self.load(parent, OPERATION).await?,
            None => asked,
        };
        if !has_joined(parent.room(), viewer) {
            return Err(not_found(OPERATION));
        }
        let parent_catalog_id = parent.catalog().id();
        let runs_parent = runs_parent_room(parent.room(), viewer);
        let entries = self
            .sub_rooms
            .list_sub_rooms(parent_catalog_id, viewer)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        // 拿一间看不到的子房间的房间号来问，回的和不存在的房间一样。
        if request.catalog_id != parent_catalog_id
            && !entries
                .iter()
                .any(|entry| entry.catalog_id == request.catalog_id && visible(entry, runs_parent))
        {
            return Err(not_found(OPERATION));
        }
        let facts = self
            .sub_rooms
            .sub_room_parent_facts(parent_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?
            .ok_or_else(|| not_found(OPERATION))?;
        let can_create =
            authorize_sub_room_creation(parent.room(), viewer, facts.policy, false).is_ok();
        let categories = self
            .sub_rooms
            .sub_room_categories(parent_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        let parent_active = parent.room().status() == PrivateRoomLifecycleStatus::Active;
        let mut sub_rooms: Vec<SubRoomView> = entries
            .into_iter()
            // 归档的和没归档的各是一份：归档的让出了名字和名额，在“已归档”里。归档以前看得到的人，
            // 归档以后也看得到这一行。
            .filter(|entry| entry.archived == request.archived && visible(entry, runs_parent))
            .map(|entry| {
                // 和 `manages_sub_room` 一样：父房间的房主和管理员，加上在里面的这一间的管理员。
                let manages = runs_parent || (inside(&entry) && entry.managers.contains(&viewer));
                SubRoomView {
                    // 有门、看的人又不在里面的，里面有几个人、多久以前有人说话不给。
                    inside_visible: entry.access == RoomJoinMode::Open || inside(&entry),
                    manages,
                    restorable: entry.archived && restorable(&entry, manages, parent_active),
                    entry,
                }
            })
            .collect();
        if request.archived {
            // 最近归档的在前：多半是刚收错了要拿回来的。
            sub_rooms.sort_by_key(|view| {
                std::cmp::Reverse(view.entry.archive_record.map(|record| record.at))
            });
        }
        Ok(SubRoomList {
            parent_catalog_id,
            parent_name: parent.catalog().name().to_owned(),
            can_create,
            categories,
            sub_rooms,
        })
    }

    async fn sub_room_policy_internal(
        &self,
        request: InspectSubRoomPolicy,
    ) -> PrivateRoomResult<SubRoomPolicyView> {
        const OPERATION: &str = "sub_room.policy";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let viewer = request.actor.principal_id;
        let parent = self
            .load_as_member(request.parent_catalog_id, viewer, OPERATION)
            .await?;
        ensure_not_sub_room(&parent, OPERATION)?;
        let policy = self
            .stored_sub_room_policy(request.parent_catalog_id, OPERATION)
            .await?
            .map_or(SubRoomPolicy::DEFAULT, |record| record.policy);
        Ok(SubRoomPolicyView {
            policy,
            can_change: parent.room().owner_principal_id() == viewer,
        })
    }

    async fn change_sub_room_policy_internal(
        &self,
        request: ChangeSubRoomPolicy,
    ) -> PrivateRoomResult<SubRoomPolicyView> {
        const OPERATION: &str = "sub_room.policy.change";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let parent = self
            .load_as_member(request.parent_catalog_id, actor, OPERATION)
            .await?;
        ensure_not_sub_room(&parent, OPERATION)?;
        if parent.room().owner_principal_id() != actor {
            return Err(refused(OPERATION, SubRoomRefusal::PolicyOwnerOnly));
        }
        let view = SubRoomPolicyView {
            policy: request.policy,
            can_change: true,
        };
        let stored = self
            .stored_sub_room_policy(request.parent_catalog_id, OPERATION)
            .await?;
        // 没定过的房间用的就是默认值；和现在一样就不写。
        if stored.map_or(SubRoomPolicy::DEFAULT, |record| record.policy) == request.policy {
            return Ok(view);
        }
        let expected = stored.map(|record| record.version);
        let record = SubRoomPolicyRecord {
            catalog_id: request.parent_catalog_id,
            policy: request.policy,
            updated_by: actor,
            updated_at: now,
            version: expected
                .unwrap_or(AggregateVersion::INITIAL)
                .next()
                .map_err(|error| domain(OPERATION, &error))?,
        };
        let saved = self
            .sub_rooms
            .save_sub_room_policy(&record, expected)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        if saved {
            Ok(view)
        } else {
            // 另一个请求刚改过：请他看过新的再改。
            Err(conflict(OPERATION))
        }
    }

    async fn stored_sub_room_policy(
        &self,
        parent: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<Option<SubRoomPolicyRecord>> {
        self.sub_rooms
            .sub_room_policy(parent)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))
    }
}

/// 这件事只有父房间有：读到的是一间子房间时拒绝。
pub(super) fn ensure_not_sub_room(
    snapshot: &PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    if snapshot.parent_catalog_id().is_some() {
        Err(refused(operation, SubRoomRefusal::ParentOnly))
    } else {
        Ok(())
    }
}

/// 同上，但只对这间子房间里的人说“去父房间做”。不在里面的人照旧走后面的检查，拿到的回答和别的
/// 进不去的房间一样：从回答里看不出这个房间号是不是一间子房间。
pub(super) fn ensure_parent_room(
    snapshot: &PrivateRoomSnapshot,
    actor: PrincipalId,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    if has_joined(snapshot.room(), actor) {
        ensure_not_sub_room(snapshot, operation)
    } else {
        Ok(())
    }
}

/// 要存的这一间：挂在谁下面、谁建的、谁在管、什么门。
fn new_sub_room_record(
    request: &CreateSubRoom,
    parent: &PrivateRoomSnapshot,
    snapshot: PrivateRoomSnapshot,
    door: SubRoomDoor,
    end_to_end_encrypted: bool,
    created_at: UtcMillis,
) -> NewSubRoom {
    let creator = request.actor.principal_id();
    NewSubRoom {
        snapshot,
        parent_catalog_id: request.parent_catalog_id,
        parent_version: parent.room().version(),
        name_key: sub_room_name_key(&request.name),
        access: door.access(),
        hidden: door.hidden(),
        category_id: request.category_id,
        created_by: creator,
        created_by_agent: request.created_by_agent,
        // 房主本来就管每一间，不用再记成这一间的管理员。
        managers: if creator == parent.room().owner_principal_id() {
            Vec::new()
        } else {
            vec![creator]
        },
        end_to_end_encrypted,
        created_at,
    }
}

/// 子房间的目录条目：房主是父房间的房主，保留期跟父房间，一律不进目录。
fn sub_room_catalog(
    request: &CreateSubRoom,
    parent: &PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<RoomCatalog> {
    RoomCatalog::new(
        request.catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: request.name.clone(),
            description: request.description.clone(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(parent.room().owner_principal_id()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: parent.catalog().retention_days(),
            status: RoomCatalogStatus::Active,
        },
    )
    .map_err(|error| domain(operation, &error))
}

pub(super) fn has_joined(room: &PrivateRoom, principal: PrincipalId) -> bool {
    room.member(principal)
        .is_some_and(PrivateRoomMember::has_joined)
}

fn inside(entry: &SubRoomEntry) -> bool {
    entry.viewer_status == Some(PrivateRoomMembershipStatus::Joined)
}

/// 勾了“不在名单里显示”的，只有里面的人、父房间的房主和管理员看得到。
fn visible(entry: &SubRoomEntry, runs_parent: bool) -> bool {
    !entry.hidden || runs_parent || inside(entry)
}

/// 归档了的这一间看的人能不能恢复：和恢复本身同一套规矩（`authorize_sub_room_restore`）。看的人
/// 已经在父房间里（读得到名单就是），所以“原来在里面”只看这一间。没记原因的当是人收起来的。
fn restorable(entry: &SubRoomEntry, manages: bool, parent_active: bool) -> bool {
    let reason = entry
        .archive_record
        .map_or(SubRoomArchiveReason::Manual, |record| record.reason);
    restore_allowed(
        RestoreStanding {
            manages,
            was_inside: inside(entry),
            parent_active,
        },
        reason,
    )
    .is_ok()
}

pub(super) const fn refused(
    operation: &'static str,
    refusal: SubRoomRefusal,
) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Validation,
        PrivateRoomFailureKind::SubRoom(refusal),
    )
}

const fn creation_refused(
    operation: &'static str,
    refusal: SubRoomCreationRefusal,
) -> PrivateRoomFailure {
    let kind = match refusal {
        SubRoomCreationRefusal::ParentArchived => PrivateRoomFailureKind::Conflict,
        SubRoomCreationRefusal::NotASpeakingMember => PrivateRoomFailureKind::Forbidden,
        SubRoomCreationRefusal::AdminsOnly => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AdminsOnly)
        }
        SubRoomCreationRefusal::AgentsMayNotCreate => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AgentsMayNotCreate)
        }
    };
    failure(operation, PrivateRoomFailureStage::Validation, kind)
}

const fn creation_blocked(
    operation: &'static str,
    block: SubRoomCreationBlock,
) -> PrivateRoomFailure {
    let kind = match block {
        SubRoomCreationBlock::NameTaken => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::NameTaken)
        }
        SubRoomCreationBlock::RoomLimitReached => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::LimitReached)
        }
        SubRoomCreationBlock::CreatorLimitReached => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::RateLimited)
        }
        // 刚检查过以后父房间归档了，或者不在了。
        SubRoomCreationBlock::ParentUnavailable => PrivateRoomFailureKind::Conflict,
        SubRoomCreationBlock::CategoryMissing => {
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::CategoryMissing)
        }
    };
    failure(operation, PrivateRoomFailureStage::Persistence, kind)
}

pub(super) const fn not_found(operation: &'static str) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Persistence,
        PrivateRoomFailureKind::NotFound,
    )
}

const fn conflict(operation: &'static str) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Persistence,
        PrivateRoomFailureKind::Conflict,
    )
}

fn privacy_failure(operation: &'static str, failure: SubRoomPrivacyFailure) -> PrivateRoomFailure {
    match failure {
        SubRoomPrivacyFailure::Repository(error) => {
            repository(operation, PrivateRoomFailureStage::Persistence, &error)
        }
        SubRoomPrivacyFailure::Matrix(error) => matrix(operation, error),
    }
}
