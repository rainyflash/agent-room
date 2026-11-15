//! 子房间第 2d-4 步（`specs/redesign/room-roles.md` 的 R5）：设、撤子房间管理员。
//!
//! 子房间管理员只管那一间：在那一间里身上有“管理”，管理员名单（`private_room_sub_room_manager`）上
//! 有他。两样要一起改：父房间里一变，子房间照父房间重算权限时，“管理”看的是名单
//! （`follow_parent_room`）。管得了这一间的人都能设；撤别人要父房间的房主或管理员，子房间管理员只能
//! 撤自己。把权力交出去、收回来要最近登录过（和设、撤管理员一样），自己不当了不用。

use agent_room_domain::{
    private_rooms::PrivateRoomMember,
    sub_rooms::{SubRoomManagerRefusal, authorize_sub_room_manager_change, sub_room_permissions},
};

use crate::ports::SubRoomManagerChange;

use super::{
    super::{
        ChangeSubRoomManager, PrivateRoomFailure, PrivateRoomFailureStage, PrivateRoomResult,
        SubRoomRefusal,
        failure::{domain, repository},
    },
    PrivateRoomService, ensure_active_actor, ensure_recent_actor,
    role_changes::role_changes,
    sub_room_membership::LoadedSubRoom,
    sub_rooms::refused,
};

impl PrivateRoomService {
    pub(super) async fn change_sub_room_manager_internal(
        &self,
        request: ChangeSubRoomManager,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "sub_room.manager.change";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let target = request.target;
        let LoadedSubRoom {
            parent, sub_room, ..
        } = self
            .load_managed_sub_room(
                request.parent_catalog_id,
                request.catalog_id,
                actor,
                OPERATION,
            )
            .await?;
        authorize_sub_room_manager_change(
            parent.room(),
            sub_room.room(),
            actor,
            target,
            request.grant,
        )
        .map_err(|refusal| manager_refused(OPERATION, refusal))?;
        let member = sub_room
            .room()
            .member(target)
            .ok_or_else(|| refused(OPERATION, SubRoomRefusal::TargetNotInside))?;
        if member.permissions().manage() == request.grant {
            // 已经是那样了：同一个请求重发也走到这里。
            return Ok(());
        }
        // 自己不当了是往下降，不用重新登录；别的都是把权力交出去、收回来。
        let stepping_down = actor == target && !request.grant;
        if !stepping_down {
            ensure_recent_actor(&request.actor, OPERATION)?;
        }
        let in_parent = parent
            .room()
            .member(target)
            .map(PrivateRoomMember::permissions)
            .ok_or_else(|| refused(OPERATION, SubRoomRefusal::NotInParent))?;
        let permissions = sub_room_permissions(in_parent, request.grant)
            .map_err(|error| domain(OPERATION, &error))?;
        // 以这一间的房主的名义改：谁设得了、撤得了上面已经照子房间的规则判断过了。
        let owner = sub_room.room().owner_principal_id();
        let mut room = sub_room.room().clone();
        room.update_permissions(owner, target, permissions)
            .map_err(|error| domain(OPERATION, &error))?;
        let standing = role_changes(sub_room.room(), &room, actor);
        let saved = self
            .sub_rooms
            .change_sub_room_manager(&SubRoomManagerChange {
                room: &room,
                expected_version: sub_room.room().version(),
                parent_catalog_id: request.parent_catalog_id,
                principal: target,
                grant: request.grant,
                changed_by: actor,
                at: now,
                role_changes: &standing,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        if saved {
            Ok(())
        } else {
            // 读到他在父房间里以后、存之前他被移出去了。
            Err(refused(OPERATION, SubRoomRefusal::NotInParent))
        }
    }
}

fn manager_refused(operation: &'static str, refusal: SubRoomManagerRefusal) -> PrivateRoomFailure {
    match refusal {
        SubRoomManagerRefusal::ManagersOnly => refused(operation, SubRoomRefusal::ManagersOnly),
        SubRoomManagerRefusal::NotInside => refused(operation, SubRoomRefusal::TargetNotInside),
        SubRoomManagerRefusal::ParentRuns => refused(operation, SubRoomRefusal::ManagedByParent),
        SubRoomManagerRefusal::PeerManager => refused(operation, SubRoomRefusal::PeerManager),
    }
}
