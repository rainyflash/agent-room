//! 子房间第 2b 步（`specs/redesign/sub-rooms.md`）：人自己进一间子房间、从里面退出，和改它的名字、
//! 用途；第 2d-1 步：装门、拆门、换一种门，和“不在名单里显示”。申请和批准（2d-2）在旁边的
//! `sub_room_requests`，把人记进来的那一段两边共用（[`PrivateRoomService::admit_into_sub_room`]）。
//!
//! 加入不走“受邀再接受”两步：加入是他自己点的，控制面直接把他记成已加入，在 Matrix 上邀请他、
//! 设好发言级别，网页接着照“服务器记着已加入、Matrix 上是受邀”那一套替他加入。退出也只要这一个
//! 请求：控制面替他在 Matrix 上离开，不用网页先退。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomPermissions},
    room_listing::RoomJoinMode,
    sub_rooms::{
        SubRoomDoor, SubRoomEntryRefusal, authorize_sub_room_join, manages_sub_room,
        runs_parent_room, sub_room_name_key, validate_sub_room_name,
    },
    time::UtcMillis,
};

use crate::ports::{
    MatrixRoomId, MatrixUserId, PrivateRoomSnapshot, SubRoomAdmission, SubRoomDoorChange,
    SubRoomLink, SubRoomRevision, SubRoomRevisionBlock,
};

use super::{
    super::{
        PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage, PrivateRoomResult,
        ReviseSubRoom, SubRoomMembershipAction, SubRoomRefusal,
        failure::{domain, failure, matrix, repository},
    },
    MemberAgentChange, PrivateRoomService, active_actor_matrix_user, ensure_active_actor,
    matrix_room_id, replace_room,
    role_changes::role_changes,
    sub_rooms::{has_joined, not_found, refused},
};

/// 一间子房间，连同它所在的房间和父子关系。
pub(super) struct LoadedSubRoom {
    pub(super) parent: PrivateRoomSnapshot,
    pub(super) sub_room: PrivateRoomSnapshot,
    pub(super) link: SubRoomLink,
}

/// 要记进子房间的人：他是谁、聊天服务器上是谁、在这一间里该有什么权限。
pub(super) struct SubRoomEntrant<'a> {
    pub(super) principal: PrincipalId,
    pub(super) matrix_user: &'a MatrixUserId,
    pub(super) permissions: PrivateRoomPermissions,
}

impl PrivateRoomService {
    pub(super) async fn join_sub_room_internal(
        &self,
        request: SubRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "sub_room.join";
        let now = self.clock.now();
        let actor_matrix = active_actor_matrix_user(&request.actor, now, OPERATION)?;
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
        let permissions =
            authorize_sub_room_join(parent.room(), sub_room.room(), actor, link.access)
                .map_err(|refusal| entry_refused(OPERATION, refusal))?;
        self.admit_into_sub_room(
            request.parent_catalog_id,
            sub_room,
            &SubRoomEntrant {
                principal: actor,
                matrix_user: &actor_matrix,
                permissions,
            },
            now,
            OPERATION,
        )
        .await
    }

    /// 把一个人记进子房间，在 Matrix 上邀请他。他自己点“加入”和管得了这一间的人批准他的申请都走
    /// 这里；进不进得来（看不看门）由调的人先判断好。
    ///
    /// 发言级别先设好（他还没被邀请，设了也用不上），再记成员，最后才邀请：没记成的不会留下一张没人
    /// 认的邀请。他在父房间或者这一间里被禁言着就不给发言级别，不然退出再进来就把禁言冲掉了；记好
    /// 以后再看一眼禁言。读到他在父房间里以后、存之前他被移出去的，什么都不写，回“找不到”。
    pub(super) async fn admit_into_sub_room(
        &self,
        parent_catalog_id: RoomCatalogId,
        sub_room: PrivateRoomSnapshot,
        entrant: &SubRoomEntrant<'_>,
        now: UtcMillis,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let catalog_id = sub_room.catalog().id();
        let principal = entrant.principal;
        let mut room = sub_room.room().clone();
        let changed = room
            .admit(principal, entrant.permissions)
            .map_err(|error| domain(operation, &error))?;
        let matrix_room = matrix_room_id(&sub_room, operation)?;
        if changed {
            let speaks = entrant.permissions.speak()
                && !self.muted_in(catalog_id, principal, operation).await?;
            self.set_speaking(&matrix_room, entrant.matrix_user, speaks, operation)
                .await?;
            let standing = role_changes(sub_room.room(), &room, principal);
            let admitted = self
                .sub_rooms
                .admit_to_sub_room(&SubRoomAdmission {
                    room: &room,
                    expected_version: sub_room.room().version(),
                    parent_catalog_id,
                    principal,
                    at: now,
                    role_changes: &standing,
                })
                .await
                .map_err(|error| {
                    repository(operation, PrivateRoomFailureStage::Persistence, &error)
                })?;
            if !admitted {
                // 读到他在父房间里以后、存之前他被移出去了：和不在父房间里的人一样回。
                return Err(not_found(operation));
            }
            // 记成成员以前正在落的禁言看不到他在这一间里，落不到这儿：记好以后再看一眼，压着的收回。
            if speaks && self.muted_in(catalog_id, principal, operation).await? {
                self.set_speaking(&matrix_room, entrant.matrix_user, false, operation)
                    .await?;
            }
        }
        // 已经在里面的再来一次也补一遍邀请（可以重复发）：上一次邀请没发成的，或者他自己在 Matrix 上
        // 拒了邀请的，这样才回得来。发言级别不重设：他要是被禁言着，再来一次不该把它冲掉。
        self.matrix
            .invite(&matrix_room, entrant.matrix_user)
            .await
            .map_err(|error| matrix(operation, error))?;
        replace_room(sub_room, room, operation)
    }

    pub(super) async fn leave_sub_room_internal(
        &self,
        request: SubRoomMembershipAction,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "sub_room.leave";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let actor = request.actor.principal_id;
        let LoadedSubRoom { sub_room, .. } = self
            .load_sub_room(
                request.parent_catalog_id,
                request.catalog_id,
                actor,
                OPERATION,
            )
            .await?;
        if sub_room.room().owner_principal_id() == actor {
            return Err(refused(OPERATION, SubRoomRefusal::OwnerStays));
        }
        let inside = sub_room
            .room()
            .member(actor)
            .is_some_and(|member| member.status().carries_permissions());
        if !inside {
            // 本来就不在里面：没什么可退的，同一个请求重发也走到这里。
            return Ok(());
        }
        if sub_room.room().status() != PrivateRoomLifecycleStatus::Active {
            return Err(refused(OPERATION, SubRoomRefusal::Archived));
        }
        let mut room = sub_room.room().clone();
        let changed = room
            .leave(actor)
            .map_err(|error| domain(OPERATION, &error))?;
        // 控制面替他在 Matrix 上离开：他可能从没点开过这一间，在 Matrix 上还只是受邀。可以重复做。
        let matrix_room = matrix_room_id(&sub_room, OPERATION)?;
        self.matrix
            .kick(&matrix_room, &actor_matrix)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.govern_member_agents(&matrix_room, actor, MemberAgentChange::Evict, OPERATION)
            .await?;
        self.save_if_changed(sub_room.room(), &room, actor, changed, OPERATION)
            .await
    }

    pub(super) async fn revise_sub_room_internal(
        &self,
        request: ReviseSubRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "sub_room.revise";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let nothing_asked = request.name.is_none()
            && request.description.is_none()
            && request.access.is_none()
            && request.hidden.is_none();
        if nothing_asked {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::InvalidRequest,
            ));
        }
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
        if !manages_sub_room(parent.room(), sub_room.room(), actor) {
            return Err(refused(OPERATION, SubRoomRefusal::ManagersOnly));
        }
        let active = PrivateRoomLifecycleStatus::Active;
        if parent.room().status() != active || sub_room.room().status() != active {
            return Err(refused(OPERATION, SubRoomRefusal::Archived));
        }
        let name = request
            .name
            .unwrap_or_else(|| sub_room.catalog().name().to_owned());
        validate_sub_room_name(&name).map_err(|error| domain(OPERATION, &error))?;
        let description = request.description.map_or_else(
            || sub_room.catalog().description().to_owned(),
            |description| description.trim().to_owned(),
        );
        let revised = sub_room
            .renaming(name)
            .and_then(|renamed| renamed.describing(description))
            .map_err(|error| domain(OPERATION, &error))?;
        let matrix_room = matrix_room_id(&revised, OPERATION)?;
        let door = if request.access.is_some() || request.hidden.is_some() {
            let from = SubRoomDoor::new(link.access, link.hidden)
                .map_err(|error| domain(OPERATION, &error))?;
            let to = from
                .changed(request.access, request.hidden)
                .map_err(|error| domain(OPERATION, &error))?;
            self.prepare_door(&link, to, &matrix_room, now, OPERATION)
                .await?;
            Some(SubRoomDoorChange { from, to })
        } else {
            None
        };
        // 目录先改：名字重不重由它在锁着父房间的事务里说了算。
        let blocked = self
            .sub_rooms
            .revise_sub_room(&SubRoomRevision {
                catalog_id: request.catalog_id,
                parent_catalog_id: request.parent_catalog_id,
                name: revised.catalog().name().to_owned(),
                name_key: sub_room_name_key(revised.catalog().name()),
                description: revised.catalog().description().to_owned(),
                door,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        match blocked {
            Some(SubRoomRevisionBlock::NameTaken) => {
                return Err(refused(OPERATION, SubRoomRefusal::NameTaken));
            }
            // 刚看过以后它归档了。
            Some(SubRoomRevisionBlock::Unavailable) => {
                return Err(refused(OPERATION, SubRoomRefusal::Archived));
            }
            // 另一个请求刚换过门：请他看过新的再换。
            Some(SubRoomRevisionBlock::DoorChanged) => {
                return Err(failure(
                    OPERATION,
                    PrivateRoomFailureStage::Persistence,
                    PrivateRoomFailureKind::Conflict,
                ));
            }
            None => {}
        }
        // Matrix 上的房间名和话题跟着写。和现在的一样也写一遍（可以重复写）：上一次目录改好了、
        // 这一步没成的，再点一次保存就补上。
        self.matrix
            .set_name(&matrix_room, revised.catalog().name())
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.topics
            .set_topic(&matrix_room, revised.catalog().description())
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        Ok(revised)
    }

    /// 换门之前在聊天服务器上要先做的两样，都在存门之前：没做成就不换。两样重做都不出错；做了、
    /// 门却没存上（撞了名字、别人刚换过门）也不要紧，留下的样子都是说得通的。
    ///
    /// - 装门（现在不加密、换完有门）：先开加密，再把目录上的标记改过来。有门的一律加密，开了关
    ///   不掉。门还没存就先标成加密不要紧：“没有门、加密着”本来就是拆过门的子房间的样子。
    /// - 第一次拆门：写下这一刻（`room.listing.v1`），这以后才进来的人不补发以前的房间钥匙。写在
    ///   存门之前，门一开就进来的人一定排在这一刻后面。拆过的不再写：往后挪的话，第一次拆门以后
    ///   自己进来的人会变成“拆门以前就在”，拿得到后来又装上门那一段的钥匙。
    async fn prepare_door(
        &self,
        link: &SubRoomLink,
        door: SubRoomDoor,
        matrix_room: &MatrixRoomId,
        now: UtcMillis,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        if door.has_door() && !link.end_to_end_encrypted {
            self.listing_matrix
                .enable_encryption(matrix_room)
                .await
                .map_err(|error| matrix(operation, error))?;
            self.listings
                .mark_room_encrypted(link.catalog_id)
                .await
                .map_err(|error| {
                    repository(operation, PrivateRoomFailureStage::Persistence, &error)
                })?;
        }
        let had_door = link.access != RoomJoinMode::Open;
        if had_door && !door.has_door() && link.door_opened_at.is_none() {
            self.listing_matrix
                .mark_listed(matrix_room, now)
                .await
                .map_err(|error| matrix(operation, error))?;
        }
        Ok(())
    }

    /// 读一间子房间和它所在的房间，照看的人找得到的来：他得是父房间里已加入的人，地址里的父房间得
    /// 对得上；勾了“不在名单里显示”的，只有里面的人、父房间的房主和管理员找得到。找不到的回答都
    /// 一样，从回答里看不出这间子房间在不在。
    pub(super) async fn load_sub_room(
        &self,
        parent_catalog_id: RoomCatalogId,
        catalog_id: RoomCatalogId,
        viewer: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<LoadedSubRoom> {
        let parent = self
            .load_as_member(parent_catalog_id, viewer, operation)
            .await?;
        let sub_room = self
            .find(catalog_id, operation)
            .await?
            .filter(|sub_room| sub_room.parent_catalog_id() == Some(parent_catalog_id))
            .ok_or_else(|| not_found(operation))?;
        let link = self
            .sub_rooms
            .sub_room_link(catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?
            .ok_or_else(|| not_found(operation))?;
        let reachable = !link.hidden
            || runs_parent_room(parent.room(), viewer)
            || has_joined(sub_room.room(), viewer);
        if reachable {
            Ok(LoadedSubRoom {
                parent,
                sub_room,
                link,
            })
        } else {
            Err(not_found(operation))
        }
    }
}

pub(super) const fn entry_refused(
    operation: &'static str,
    refusal: SubRoomEntryRefusal,
) -> PrivateRoomFailure {
    match refusal {
        // 读房间时已经看过他在父房间里：走到这里是两次读之间变了，和找不到一样回。
        SubRoomEntryRefusal::NotInParent => not_found(operation),
        SubRoomEntryRefusal::Archived => refused(operation, SubRoomRefusal::Archived),
        SubRoomEntryRefusal::Banned => refused(operation, SubRoomRefusal::Banned),
        SubRoomEntryRefusal::ApprovalRequired => {
            refused(operation, SubRoomRefusal::ApprovalRequired)
        }
        SubRoomEntryRefusal::InviteOnly => refused(operation, SubRoomRefusal::InviteOnly),
    }
}
