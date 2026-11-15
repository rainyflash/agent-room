//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 的“归档和恢复”，第 2e-2 步）。
//!
//! 整间房的归档只认房主、不能恢复（[`PrivateRoom::archive`]）。子房间不一样：一件事一个子房间，
//! 做完就收起来，收错了要拿得回来。谁能收、谁能拿回来在这里判断；房间自己的状态照旧只有“在用”和
//! “归档”两样，为什么归档的、谁归档的另记在父子关系上。

use crate::{
    error::{DomainError, DomainResult},
    ids::PrincipalId,
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus, PrivateRoomMember},
    sub_rooms::manages_sub_room,
};

/// 一间子房间为什么归档了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomArchiveReason {
    /// 管得了这一间的人收起来的。
    Manual,
    /// 30 天里没人说话，自动收起来的（2j）。没有谁决定过要关它，原来在里面的人谁都能恢复。
    Inactive,
    /// 跟着父房间归档了。父房间不能恢复，它也回不来。
    Parent,
}

impl SubRoomArchiveReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Inactive => "inactive",
            Self::Parent => "parent",
        }
    }
}

impl TryFrom<&str> for SubRoomArchiveReason {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "manual" => Ok(Self::Manual),
            "inactive" => Ok(Self::Inactive),
            "parent" => Ok(Self::Parent),
            _ => Err(DomainError::Validation {
                field: "sub_room_archive_reason",
                reason: "只能是 manual、inactive 或 parent",
            }),
        }
    }
}

/// 为什么归档、恢复不了这间子房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomArchiveRefusal {
    /// 管不了这一间（恢复自动归档的，原来在里面的人也行）。
    ManagersOnly,
    /// 父房间归档了：它下面的子房间收不了，也拿不回来。
    ParentArchived,
}

/// 能不能把这间子房间归档：这一间的子房间管理员、父房间的房主和管理员（[`manages_sub_room`]）。
///
/// # Errors
///
/// 归档不了时返回原因。
pub fn authorize_sub_room_archive(
    parent: &PrivateRoom,
    sub_room: &PrivateRoom,
    actor: PrincipalId,
) -> Result<(), SubRoomArchiveRefusal> {
    if !manages_sub_room(parent, sub_room, actor) {
        return Err(SubRoomArchiveRefusal::ManagersOnly);
    }
    if parent.status() != PrivateRoomLifecycleStatus::Active {
        return Err(SubRoomArchiveRefusal::ParentArchived);
    }
    Ok(())
}

/// 能不能把归档了的这间子房间恢复（`reason` 是它为什么归档的）：能归档的人都能恢复；30 天没人说话
/// 自动归档的，原来在里面、现在还在父房间里的人也能。跟着父房间归档的回不来。
///
/// # Errors
///
/// 恢复不了时返回原因。
pub fn authorize_sub_room_restore(
    parent: &PrivateRoom,
    sub_room: &PrivateRoom,
    actor: PrincipalId,
    reason: SubRoomArchiveReason,
) -> Result<(), SubRoomArchiveRefusal> {
    let was_inside = parent
        .member(actor)
        .is_some_and(PrivateRoomMember::has_joined)
        && sub_room
            .member(actor)
            .is_some_and(PrivateRoomMember::has_joined);
    let may = manages_sub_room(parent, sub_room, actor)
        || (reason == SubRoomArchiveReason::Inactive && was_inside);
    if !may {
        return Err(SubRoomArchiveRefusal::ManagersOnly);
    }
    if parent.status() != PrivateRoomLifecycleStatus::Active
        || reason == SubRoomArchiveReason::Parent
    {
        return Err(SubRoomArchiveRefusal::ParentArchived);
    }
    Ok(())
}

/// 归档了的这一间：只改房间的状态，成员、权限原样留着（恢复以后原来的人接着说）。谁能归档先用
/// [`authorize_sub_room_archive`] 看。已经归档的回 `None`。
///
/// # Errors
///
/// 版本用尽时返回错误。
pub fn archived_sub_room(sub_room: &PrivateRoom) -> DomainResult<Option<PrivateRoom>> {
    with_status(sub_room, PrivateRoomLifecycleStatus::Archived)
}

/// 恢复了的这一间。谁能恢复先用 [`authorize_sub_room_restore`] 看。没归档的回 `None`。
///
/// # Errors
///
/// 版本用尽时返回错误。
pub fn restored_sub_room(sub_room: &PrivateRoom) -> DomainResult<Option<PrivateRoom>> {
    with_status(sub_room, PrivateRoomLifecycleStatus::Active)
}

fn with_status(
    sub_room: &PrivateRoom,
    status: PrivateRoomLifecycleStatus,
) -> DomainResult<Option<PrivateRoom>> {
    let mut changed = sub_room.clone();
    Ok(changed.change_status(status)?.then_some(changed))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        ids::RoomCatalogId,
        private_rooms::{PrivateRoomCapability, PrivateRoomPermissions},
        sub_rooms::{new_sub_room, sub_room_permissions},
    };

    fn principal() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn catalog() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    /// 父房间：房主、管理员、两个能发言的成员。子房间是 `creator` 建的（这一间的管理员），`member`
    /// 进过这一间，`outsider` 没进过。
    struct Rooms {
        parent: PrivateRoom,
        sub_room: PrivateRoom,
        owner: PrincipalId,
        admin: PrincipalId,
        creator: PrincipalId,
        member: PrincipalId,
        outsider: PrincipalId,
    }

    fn rooms() -> Rooms {
        let owner = principal();
        let (admin, creator, member, outsider) =
            (principal(), principal(), principal(), principal());
        let mut parent = PrivateRoom::create(catalog(), owner);
        let speaker = PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
        ])
        .expect("权限有效");
        let admin_permissions = PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Manage,
        ])
        .expect("权限有效");
        for (who, granted) in [
            (admin, admin_permissions),
            (creator, speaker),
            (member, speaker),
            (outsider, speaker),
        ] {
            parent.invite(owner, who, granted).expect("房主能邀请");
            parent.accept_invitation(who).expect("受邀的人能接受");
        }
        let mut sub_room = new_sub_room(catalog(), &parent, creator).expect("成员能建");
        sub_room
            .admit(
                member,
                sub_room_permissions(speaker, false).expect("权限有效"),
            )
            .expect("成员能进");
        Rooms {
            parent,
            sub_room,
            owner,
            admin,
            creator,
            member,
            outsider,
        }
    }

    #[test]
    fn 这一间的管理员和父房间的房主_管理员能归档_别人不能() {
        let rooms = rooms();
        for who in [rooms.creator, rooms.owner, rooms.admin] {
            assert_eq!(
                authorize_sub_room_archive(&rooms.parent, &rooms.sub_room, who),
                Ok(())
            );
        }
        for who in [rooms.member, rooms.outsider, principal()] {
            assert_eq!(
                authorize_sub_room_archive(&rooms.parent, &rooms.sub_room, who),
                Err(SubRoomArchiveRefusal::ManagersOnly)
            );
        }
    }

    #[test]
    fn 归档只改状态_成员原样留着_恢复回到在用() {
        let rooms = rooms();
        let archived = archived_sub_room(&rooms.sub_room)
            .expect("版本够用")
            .expect("在用的能归档");
        assert_eq!(archived.status(), PrivateRoomLifecycleStatus::Archived);
        assert_eq!(
            archived.version(),
            rooms.sub_room.version().next().expect("版本够用")
        );
        assert!(
            archived.members().eq(rooms.sub_room.members()),
            "成员、权限都不动"
        );
        assert_eq!(archived_sub_room(&archived).expect("版本够用"), None);

        let restored = restored_sub_room(&archived)
            .expect("版本够用")
            .expect("归档的能恢复");
        assert_eq!(restored.status(), PrivateRoomLifecycleStatus::Active);
        assert!(restored.members().eq(rooms.sub_room.members()));
        assert_eq!(restored_sub_room(&restored).expect("版本够用"), None);
    }

    #[test]
    fn 手动归档的只有管得了的人能恢复_自动归档的原来在里面的人也能() {
        let rooms = rooms();
        let archived = archived_sub_room(&rooms.sub_room)
            .expect("版本够用")
            .expect("能归档");
        for who in [rooms.creator, rooms.owner, rooms.admin] {
            for reason in [SubRoomArchiveReason::Manual, SubRoomArchiveReason::Inactive] {
                assert_eq!(
                    authorize_sub_room_restore(&rooms.parent, &archived, who, reason),
                    Ok(())
                );
            }
        }
        assert_eq!(
            authorize_sub_room_restore(
                &rooms.parent,
                &archived,
                rooms.member,
                SubRoomArchiveReason::Manual
            ),
            Err(SubRoomArchiveRefusal::ManagersOnly)
        );
        assert_eq!(
            authorize_sub_room_restore(
                &rooms.parent,
                &archived,
                rooms.member,
                SubRoomArchiveReason::Inactive
            ),
            Ok(()),
            "原来在里面的人能恢复自动归档的"
        );
        for who in [rooms.outsider, principal()] {
            assert_eq!(
                authorize_sub_room_restore(
                    &rooms.parent,
                    &archived,
                    who,
                    SubRoomArchiveReason::Inactive
                ),
                Err(SubRoomArchiveRefusal::ManagersOnly),
                "没进过的不行"
            );
        }
    }

    #[test]
    fn 原来在里面_已经退出父房间的不能恢复() {
        let mut rooms = rooms();
        rooms.parent.leave(rooms.member).expect("成员能退出");
        let archived = archived_sub_room(&rooms.sub_room)
            .expect("版本够用")
            .expect("能归档");
        assert_eq!(
            authorize_sub_room_restore(
                &rooms.parent,
                &archived,
                rooms.member,
                SubRoomArchiveReason::Inactive
            ),
            Err(SubRoomArchiveRefusal::ManagersOnly)
        );
    }

    #[test]
    fn 父房间归档了_收不了也拿不回来_跟着父房间归档的也拿不回来() {
        let mut rooms = rooms();
        let archived = archived_sub_room(&rooms.sub_room)
            .expect("版本够用")
            .expect("能归档");
        assert_eq!(
            authorize_sub_room_restore(
                &rooms.parent,
                &archived,
                rooms.owner,
                SubRoomArchiveReason::Parent
            ),
            Err(SubRoomArchiveRefusal::ParentArchived)
        );
        rooms.parent.archive(rooms.owner).expect("房主能归档");
        assert_eq!(
            authorize_sub_room_archive(&rooms.parent, &rooms.sub_room, rooms.owner),
            Err(SubRoomArchiveRefusal::ParentArchived)
        );
        assert_eq!(
            authorize_sub_room_restore(
                &rooms.parent,
                &archived,
                rooms.owner,
                SubRoomArchiveReason::Manual
            ),
            Err(SubRoomArchiveRefusal::ParentArchived)
        );
    }

    #[test]
    fn 归档的原因认得三种() {
        for reason in [
            SubRoomArchiveReason::Manual,
            SubRoomArchiveReason::Inactive,
            SubRoomArchiveReason::Parent,
        ] {
            assert_eq!(SubRoomArchiveReason::try_from(reason.as_str()), Ok(reason));
        }
        assert!(SubRoomArchiveReason::try_from("deleted").is_err());
    }
}
