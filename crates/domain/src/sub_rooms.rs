//! 子房间（`specs/redesign/sub-rooms.md`）：挂在一间私人房间下面的房间，只有一层。
//!
//! 子房间自己照旧是一间私人房间（同一个 [`PrivateRoom`] 聚合），这里只放它多出来的规则：谁能建、
//! 建好时里面有谁、一个人在子房间里的权限怎么从父房间里的算出来、名字怎么比重名。

use crate::{
    DomainError, DomainResult,
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomMember,
        PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    version::AggregateVersion,
};

/// 一间房里没归档的子房间最多这么多。防失控用的，不是为了管人。
pub const MAXIMUM_OPEN_SUB_ROOMS: u16 = 100;
/// 同一个人或 Agent 一小时最多建这么多。
pub const MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR: u16 = 10;
/// 安静了这么多天的子房间自动归档（开着的时候）。
pub const SUB_ROOM_AUTO_ARCHIVE_DAYS: u8 = 30;

/// 谁能在这间房下面建子房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomCreators {
    /// 已加入、能发言的人都能建（默认）。
    Members,
    /// 只有房主和管理员。
    Admins,
}

impl SubRoomCreators {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Members => "members",
            Self::Admins => "admins",
        }
    }
}

impl TryFrom<&str> for SubRoomCreators {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "members" => Ok(Self::Members),
            "admins" => Ok(Self::Admins),
            _ => Err(DomainError::Validation {
                field: "sub_room_creators",
                reason: "只能是 members 或 admins",
            }),
        }
    }
}

/// 一间房里子房间的三项设置（`specs/redesign/room-roles.md`），只有房主改得动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomPolicy {
    pub creators: SubRoomCreators,
    /// 房间里的 Agent 能不能建子房间。
    pub agents_may_create: bool,
    /// 安静的子房间要不要自动归档。
    pub auto_archive: bool,
}

impl SubRoomPolicy {
    /// 没定过的房间：成员都能建、Agent 能建、自动归档开着。
    pub const DEFAULT: Self = Self {
        creators: SubRoomCreators::Members,
        agents_may_create: true,
        auto_archive: true,
    };
}

/// 为什么建不了子房间。分开说，回答里要写清原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomCreationRefusal {
    /// 父房间归档了。
    ParentArchived,
    /// 不在父房间里，或者在里面但不能发言。
    NotASpeakingMember,
    /// 房主定了只有管理员能建。
    AdminsOnly,
    /// 房主关掉了“Agent 能建”。
    AgentsMayNotCreate,
}

/// `creator` 能不能在 `parent` 下面建子房间。`by_agent` 是他的 Agent 替他建。
///
/// # Errors
///
/// 建不了时返回原因。
pub fn authorize_sub_room_creation(
    parent: &PrivateRoom,
    creator: PrincipalId,
    policy: SubRoomPolicy,
    by_agent: bool,
) -> Result<(), SubRoomCreationRefusal> {
    if parent.status() != PrivateRoomLifecycleStatus::Active {
        return Err(SubRoomCreationRefusal::ParentArchived);
    }
    let member = parent
        .member(creator)
        .filter(|member| member.has_joined() && member.permissions().speak())
        .ok_or(SubRoomCreationRefusal::NotASpeakingMember)?;
    let runs_parent = parent.owner_principal_id() == creator || member.permissions().manage();
    if policy.creators == SubRoomCreators::Admins && !runs_parent {
        return Err(SubRoomCreationRefusal::AdminsOnly);
    }
    if by_agent && !policy.agents_may_create {
        return Err(SubRoomCreationRefusal::AgentsMayNotCreate);
    }
    Ok(())
}

/// 一个人在子房间里的权限：查看、发言、自动发言照他在父房间里的；“管理”看他管不管这一间
/// （这一间的子房间管理员，或者父房间的管理员）；“邀请”一律没有，子房间不能往外请人。
///
/// # Errors
///
/// 他在父房间里连查看都没有时返回错误：这样的人不该在子房间里。
pub fn sub_room_permissions(
    in_parent: PrivateRoomPermissions,
    manages: bool,
) -> DomainResult<PrivateRoomPermissions> {
    if !in_parent.view() {
        return Err(DomainError::Validation {
            field: "sub_room_member",
            reason: "不在父房间里的人不能在子房间里",
        });
    }
    let carried = [
        (true, PrivateRoomCapability::View),
        (in_parent.speak(), PrivateRoomCapability::Speak),
        (in_parent.automate(), PrivateRoomCapability::Automate),
        (manages, PrivateRoomCapability::Manage),
    ];
    PrivateRoomPermissions::from_capabilities(
        carried
            .into_iter()
            .filter_map(|(held, capability)| held.then_some(capability)),
    )
}

/// 新建的子房间：房主是父房间的房主，建的人已经在里面、是这一间的管理员。
///
/// 建的人就是父房间的房主时，里面只有他一个。
///
/// # Errors
///
/// 建的人不在父房间里时返回错误。
pub fn new_sub_room(
    catalog_id: RoomCatalogId,
    parent: &PrivateRoom,
    creator: PrincipalId,
) -> DomainResult<PrivateRoom> {
    let owner = parent.owner_principal_id();
    let mut members = vec![PrivateRoomMember::restore(
        owner,
        PrivateRoomMembershipStatus::Joined,
        PrivateRoomPermissions::ALL,
    )?];
    if creator != owner {
        let in_parent = parent
            .member(creator)
            .filter(|member| member.has_joined())
            .ok_or(DomainError::Forbidden {
                action: "在这间房下面建子房间",
            })?;
        members.push(PrivateRoomMember::restore(
            creator,
            PrivateRoomMembershipStatus::Joined,
            sub_room_permissions(in_parent.permissions(), true)?,
        )?);
    }
    PrivateRoom::restore(
        catalog_id,
        owner,
        PrivateRoomLifecycleStatus::Active,
        members,
        AggregateVersion::INITIAL,
        None,
    )
}

/// 比重名用的键：去掉两头的空白、转成小写。同一间房里没归档的子房间不重名，不分大小写。
#[must_use]
pub fn sub_room_name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 子房间的名字：Agent 按 `父房间名/子房间名` 找它，所以名字里不能有 `/`，两头也不带空白。
/// 长度和控制字符照房间名的规则，由房间目录查。
///
/// # Errors
///
/// 名字里有 `/`、两头带空白或者是空的时返回错误。
pub fn validate_sub_room_name(name: &str) -> DomainResult<()> {
    if name.is_empty() || name != name.trim() {
        return Err(DomainError::Validation {
            field: "sub_room_name",
            reason: "名字不能是空的，两头不带空白",
        });
    }
    if name.contains('/') {
        return Err(DomainError::Validation {
            field: "sub_room_name",
            reason: "名字里不能有 /",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn principal() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn catalog() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn permissions(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).expect("权限有效")
    }

    const SPEAKER: [PrivateRoomCapability; 3] = [
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Automate,
    ];

    /// 父房间：房主，一个能发言的成员，一个管理员，一个只能看的，一个还没接受邀请的。
    struct Parent {
        room: PrivateRoom,
        owner: PrincipalId,
        member: PrincipalId,
        admin: PrincipalId,
        viewer: PrincipalId,
        invited: PrincipalId,
    }

    fn parent() -> Parent {
        let owner = principal();
        let (member, admin, viewer, invited) = (principal(), principal(), principal(), principal());
        let mut room = PrivateRoom::create(catalog(), owner);
        let admin_permissions = permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ]);
        for (who, granted) in [
            (member, permissions(&SPEAKER)),
            (admin, admin_permissions),
            (viewer, permissions(&[PrivateRoomCapability::View])),
            (invited, permissions(&SPEAKER)),
        ] {
            room.invite(owner, who, granted).expect("房主能邀请");
            if who != invited {
                room.accept_invitation(who).expect("受邀的人能接受");
            }
        }
        Parent {
            room,
            owner,
            member,
            admin,
            viewer,
            invited,
        }
    }

    #[test]
    fn 默认已加入能发言的人都能建_只能看的和没进来的不能() {
        let parent = parent();
        let policy = SubRoomPolicy::DEFAULT;
        for who in [parent.owner, parent.member, parent.admin] {
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, policy, false),
                Ok(())
            );
            // 他们的 Agent 也能建。
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, policy, true),
                Ok(())
            );
        }
        for who in [parent.viewer, parent.invited, principal()] {
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, policy, false),
                Err(SubRoomCreationRefusal::NotASpeakingMember)
            );
        }
    }

    #[test]
    fn 房主收紧以后成员建不了_关掉_agent_能建以后谁的_agent_都建不了() {
        let parent = parent();
        let admins_only = SubRoomPolicy {
            creators: SubRoomCreators::Admins,
            ..SubRoomPolicy::DEFAULT
        };
        assert_eq!(
            authorize_sub_room_creation(&parent.room, parent.member, admins_only, false),
            Err(SubRoomCreationRefusal::AdminsOnly)
        );
        for who in [parent.owner, parent.admin] {
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, admins_only, false),
                Ok(())
            );
        }

        let no_agents = SubRoomPolicy {
            agents_may_create: false,
            ..SubRoomPolicy::DEFAULT
        };
        for who in [parent.owner, parent.admin, parent.member] {
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, no_agents, true),
                Err(SubRoomCreationRefusal::AgentsMayNotCreate)
            );
            // 人自己照样能建。
            assert_eq!(
                authorize_sub_room_creation(&parent.room, who, no_agents, false),
                Ok(())
            );
        }
    }

    #[test]
    fn 归档了的房间下面建不了() {
        let mut parent = parent();
        parent.room.archive(parent.owner).expect("房主能归档");
        assert_eq!(
            authorize_sub_room_creation(&parent.room, parent.owner, SubRoomPolicy::DEFAULT, false),
            Err(SubRoomCreationRefusal::ParentArchived)
        );
    }

    #[test]
    fn 建好的子房间房主是父房间的房主_建的人在里面管这一间_没有邀请() {
        let parent = parent();
        let id = catalog();
        let sub_room = new_sub_room(id, &parent.room, parent.member).expect("成员能建");

        assert_eq!(sub_room.catalog_id(), id);
        assert_eq!(sub_room.owner_principal_id(), parent.owner);
        assert_eq!(sub_room.status(), PrivateRoomLifecycleStatus::Active);
        assert_eq!(sub_room.version(), AggregateVersion::INITIAL);
        assert_eq!(sub_room.created_by_agent(), None);
        assert_eq!(sub_room.members().len(), 2);
        let creator = sub_room.member(parent.member).expect("建的人在里面");
        assert!(creator.has_joined());
        assert_eq!(
            creator.permissions(),
            permissions(&[
                PrivateRoomCapability::View,
                PrivateRoomCapability::Speak,
                PrivateRoomCapability::Automate,
                PrivateRoomCapability::Manage,
            ])
        );
        let owner = sub_room.member(parent.owner).expect("房主在里面");
        assert!(owner.has_joined());
        assert_eq!(owner.permissions(), PrivateRoomPermissions::ALL);

        // 房主自己建的，里面只有他一个。
        let own = new_sub_room(catalog(), &parent.room, parent.owner).expect("房主能建");
        assert_eq!(own.members().len(), 1);
        // 不在父房间里的人建不出来。
        assert!(new_sub_room(catalog(), &parent.room, principal()).is_err());
        assert!(new_sub_room(catalog(), &parent.room, parent.invited).is_err());
    }

    #[test]
    fn 子房间里的权限照父房间里的算_管理单看管不管这一间_邀请不带进来() {
        let admin_in_parent = permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ]);
        assert_eq!(
            sub_room_permissions(admin_in_parent, false).expect("有效"),
            permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Speak])
        );
        assert_eq!(
            sub_room_permissions(permissions(&[PrivateRoomCapability::View]), true).expect("有效"),
            permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Manage])
        );
        assert_eq!(
            sub_room_permissions(permissions(&SPEAKER), false).expect("有效"),
            permissions(&SPEAKER)
        );
        assert!(sub_room_permissions(PrivateRoomPermissions::NONE, true).is_err());
    }

    #[test]
    fn 名字不分大小写比重名_不能带斜杠和两头的空白() {
        assert_eq!(sub_room_name_key("  Release Plan "), "release plan");
        assert_eq!(sub_room_name_key("发版"), "发版");
        assert_eq!(sub_room_name_key("ÄB"), sub_room_name_key("äb"));
        assert!(validate_sub_room_name("release-plan").is_ok());
        assert!(validate_sub_room_name("发版 的事").is_ok());
        for bad in ["", " ", "a/b", " lead", "trail ", "/"] {
            assert!(validate_sub_room_name(bad).is_err(), "{bad:?}");
        }
    }
}
