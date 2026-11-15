//! 子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，第 2e-1 步）：房间的房主和管理员建、
//! 改名、排先后、删。分类只管怎么摆，不碰聊天服务器，也不要最近登录过。
//!
//! 重名、上限、先后由存储在锁着父房间的事务里说了算；这里只管谁能做、名字合不合规矩。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    sub_room_categories::{
        arranges_sub_room_categories, sub_room_category_name_key, validate_sub_room_category_name,
    },
};

use crate::ports::{
    NewSubRoomCategory, SubRoomCategory, SubRoomCategoryName, SubRoomCategoryOutcome,
    SubRoomCategoryRemoval, SubRoomCategoryRevision,
};

use super::{
    super::{
        CreateSubRoomCategory, DeleteSubRoomCategory, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomResult, ReviseSubRoomCategory, SubRoomRefusal,
        failure::{domain, failure, repository},
    },
    PrivateRoomService, ensure_active_actor,
    sub_rooms::{ensure_not_sub_room, refused},
};

impl PrivateRoomService {
    pub(super) async fn create_sub_room_category_internal(
        &self,
        request: CreateSubRoomCategory,
    ) -> PrivateRoomResult<Vec<SubRoomCategory>> {
        const OPERATION: &str = "sub_room.category.create";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        self.ensure_arranges_categories(request.parent_catalog_id, actor, OPERATION)
            .await?;
        let name = category_name(&request.name, OPERATION)?;
        let outcome = self
            .sub_rooms
            .create_sub_room_category(&NewSubRoomCategory {
                id: request.category_id,
                parent_catalog_id: request.parent_catalog_id,
                name,
                created_by: actor,
                created_at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        saved(outcome, OPERATION)
    }

    pub(super) async fn revise_sub_room_category_internal(
        &self,
        request: ReviseSubRoomCategory,
    ) -> PrivateRoomResult<Vec<SubRoomCategory>> {
        const OPERATION: &str = "sub_room.category.revise";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        if request.name.is_none() && request.position.is_none() {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::InvalidRequest,
            ));
        }
        let actor = request.actor.principal_id;
        self.ensure_arranges_categories(request.parent_catalog_id, actor, OPERATION)
            .await?;
        let name = request
            .name
            .as_deref()
            .map(|name| category_name(name, OPERATION))
            .transpose()?;
        let outcome = self
            .sub_rooms
            .revise_sub_room_category(&SubRoomCategoryRevision {
                id: request.category_id,
                parent_catalog_id: request.parent_catalog_id,
                name,
                position: request.position,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        saved(outcome, OPERATION)
    }

    pub(super) async fn delete_sub_room_category_internal(
        &self,
        request: DeleteSubRoomCategory,
    ) -> PrivateRoomResult<Vec<SubRoomCategory>> {
        const OPERATION: &str = "sub_room.category.delete";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        self.ensure_arranges_categories(request.parent_catalog_id, actor, OPERATION)
            .await?;
        let outcome = self
            .sub_rooms
            .delete_sub_room_category(&SubRoomCategoryRemoval {
                id: request.category_id,
                parent_catalog_id: request.parent_catalog_id,
                removed_by: actor,
                at: now,
            })
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        saved(outcome, OPERATION)
    }

    /// 管分类的得是父房间里已加入的房主或管理员。不在房间里的人看到的和不存在的房间一样；地址
    /// 是一间子房间时请他去父房间做。
    async fn ensure_arranges_categories(
        &self,
        parent_catalog_id: RoomCatalogId,
        actor: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let parent = self
            .load_as_member(parent_catalog_id, actor, operation)
            .await?;
        ensure_not_sub_room(&parent, operation)?;
        if arranges_sub_room_categories(parent.room(), actor) {
            Ok(())
        } else {
            Err(refused(operation, SubRoomRefusal::CategoryAdminsOnly))
        }
    }
}

fn category_name(name: &str, operation: &'static str) -> PrivateRoomResult<SubRoomCategoryName> {
    let name = name.trim();
    validate_sub_room_category_name(name).map_err(|error| domain(operation, &error))?;
    Ok(SubRoomCategoryName {
        name: name.to_owned(),
        name_key: sub_room_category_name_key(name),
    })
}

fn saved(
    outcome: SubRoomCategoryOutcome,
    operation: &'static str,
) -> PrivateRoomResult<Vec<SubRoomCategory>> {
    let refusal = match outcome {
        SubRoomCategoryOutcome::Saved(categories) => return Ok(categories),
        SubRoomCategoryOutcome::NameTaken => SubRoomRefusal::CategoryNameTaken,
        SubRoomCategoryOutcome::LimitReached => SubRoomRefusal::CategoryLimitReached,
        SubRoomCategoryOutcome::Missing => SubRoomRefusal::CategoryMissing,
        // 同一个号建过别的分类，或者刚看过以后父房间归档了、不在了。
        SubRoomCategoryOutcome::IdTaken | SubRoomCategoryOutcome::ParentUnavailable => {
            return Err(conflict(operation));
        }
    };
    Err(PrivateRoomFailure::new(
        operation,
        PrivateRoomFailureStage::Persistence,
        PrivateRoomFailureKind::SubRoom(refusal),
    ))
}

const fn conflict(operation: &'static str) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Persistence,
        PrivateRoomFailureKind::Conflict,
    )
}
