//! 子房间（`specs/redesign/sub-rooms.md`）：挂在一间私人房间下面的房间，只有一层。
//!
//! 子房间自己照旧是一间私人房间（同一个 [`PrivateRoom`] 聚合），这里只放它多出来的规则：谁能建、
//! 建好时里面有谁、一个人在子房间里的权限怎么从父房间里的算出来、父房间变了它怎么跟着变、名字怎么
//! 比重名。

use std::collections::BTreeSet;

use crate::{
    DomainError, DomainResult,
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomMember,
        PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    room_listing::RoomJoinMode,
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

/// 子房间的门（界面上叫“谁能进”）和“不在名单里显示”。没有门就是 [`RoomJoinMode::Open`]：父房间里
/// 的人点一下就进。门只对父房间里面的人开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomDoor {
    access: RoomJoinMode,
    hidden: bool,
}

impl SubRoomDoor {
    /// 新建的子房间默认没有门。
    pub const NONE: Self = Self {
        access: RoomJoinMode::Open,
        hidden: false,
    };

    /// # Errors
    ///
    /// 没有门的子房间勾了“不在名单里显示”时返回错误：谁都进得来的房间藏不住。
    pub const fn new(access: RoomJoinMode, hidden: bool) -> DomainResult<Self> {
        if hidden && matches!(access, RoomJoinMode::Open) {
            return Err(DomainError::Validation {
                field: "sub_room_hidden",
                reason: "没有门的子房间不能不在名单里显示",
            });
        }
        Ok(Self { access, hidden })
    }

    #[must_use]
    pub const fn access(self) -> RoomJoinMode {
        self.access
    }

    #[must_use]
    pub const fn hidden(self) -> bool {
        self.hidden
    }

    /// 有门的子房间一律加密。
    #[must_use]
    pub const fn has_door(self) -> bool {
        !matches!(self.access, RoomJoinMode::Open)
    }

    /// 换门、勾或者去掉“不在名单里显示”。没带的那一项不动；拆门的时候没提隐藏的，隐藏跟着去掉。
    ///
    /// # Errors
    ///
    /// 换完是“没有门、又不在名单里显示”时返回错误。
    pub const fn changed(
        self,
        access: Option<RoomJoinMode>,
        hidden: Option<bool>,
    ) -> DomainResult<Self> {
        let access = match access {
            Some(access) => access,
            None => self.access,
        };
        let hidden = match hidden {
            Some(hidden) => hidden,
            None => self.hidden && !matches!(access, RoomJoinMode::Open),
        };
        Self::new(access, hidden)
    }
}

/// 为什么进不了这间子房间。分开说，回答里要写清原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomEntryRefusal {
    /// 不是父房间里已加入的人。子房间的门只对房间里面的人开关。
    NotInParent,
    /// 子房间或者它的父房间归档了。
    Archived,
    /// 在这间子房间里被封禁着。
    Banned,
    /// 这间要批准：得先申请。
    ApprovalRequired,
    /// 这间只能请进来。
    InviteOnly,
}

/// 父房间的房主和管理员：每一间子房间他们都看得到、进得去、管得了。
#[must_use]
pub fn runs_parent_room(parent: &PrivateRoom, principal: PrincipalId) -> bool {
    parent.owner_principal_id() == principal
        || parent
            .member(principal)
            .is_some_and(|member| member.has_joined() && member.permissions().manage())
}

/// 一个人管不管得了这间子房间：父房间的房主和管理员都管得了，进没进来都一样；别的人要在这一间
/// 里、身上有“管理”（这一间的子房间管理员）。
#[must_use]
pub fn manages_sub_room(
    parent: &PrivateRoom,
    sub_room: &PrivateRoom,
    principal: PrincipalId,
) -> bool {
    let in_parent = parent
        .member(principal)
        .is_some_and(PrivateRoomMember::has_joined);
    in_parent
        && (runs_parent_room(parent, principal)
            || sub_room
                .member(principal)
                .is_some_and(|member| member.has_joined() && member.permissions().manage()))
}

/// 一个人自己点“加入”进一间子房间。他得是父房间里已加入的人、没在这一间里被封禁；有门的只有父房间
/// 的房主和管理员不用批准。过了就回他在这一间里该有的权限（父房间的管理员进来就是这一间的管理员）。
/// 已经在里面的不再看门，回的是他现在的权限。
///
/// # Errors
///
/// 进不了时返回原因。
pub fn authorize_sub_room_join(
    parent: &PrivateRoom,
    sub_room: &PrivateRoom,
    principal: PrincipalId,
    access: RoomJoinMode,
) -> Result<PrivateRoomPermissions, SubRoomEntryRefusal> {
    let in_parent = parent
        .member(principal)
        .filter(|member| member.has_joined())
        .ok_or(SubRoomEntryRefusal::NotInParent)?;
    let active = PrivateRoomLifecycleStatus::Active;
    if parent.status() != active || sub_room.status() != active {
        return Err(SubRoomEntryRefusal::Archived);
    }
    let banned = sub_room
        .member(principal)
        .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned);
    if banned {
        return Err(SubRoomEntryRefusal::Banned);
    }
    if let Some(inside) = sub_room
        .member(principal)
        .filter(|member| member.has_joined())
    {
        return Ok(inside.permissions());
    }
    let runs_parent = runs_parent_room(parent, principal);
    if !runs_parent {
        match access {
            RoomJoinMode::Open => {}
            RoomJoinMode::Request => return Err(SubRoomEntryRefusal::ApprovalRequired),
            RoomJoinMode::InviteOnly => return Err(SubRoomEntryRefusal::InviteOnly),
        }
    }
    sub_room_permissions(in_parent.permissions(), runs_parent)
        .map_err(|_| SubRoomEntryRefusal::NotInParent)
}

/// 照父房间现在的样子重算一遍子房间（`specs/redesign/sub-rooms.md` 的“对账”）。做几遍结果都一样：
///
/// - 房主换成父房间的房主：他在每一间子房间里，权限全有；
/// - 在父房间里已经不是已加入的人，从子房间移出；
/// - 还在的人，查看、发言、自动发言照他在父房间里的，“管理”给父房间的管理员和这一间的子房间管理员
///   （`managers`），“邀请”一律没有；
/// - 父房间归档了，子房间跟着归档。父房间没归档时，子房间归没归档是它自己的事。
///
/// 已经移出、封禁、拒绝的人原样留着。没有要改的回 `None`；有就回改好的子房间，版本加一。
///
/// # Errors
///
/// 版本用尽时返回错误。
pub fn follow_parent_room(
    parent: &PrivateRoom,
    sub_room: &PrivateRoom,
    managers: &BTreeSet<PrincipalId>,
) -> DomainResult<Option<PrivateRoom>> {
    let owner = parent.owner_principal_id();
    let owner_member = || {
        PrivateRoomMember::restore(
            owner,
            PrivateRoomMembershipStatus::Joined,
            PrivateRoomPermissions::ALL,
        )
    };
    let mut members = Vec::with_capacity(sub_room.members().len() + 1);
    for member in sub_room.members() {
        let principal = member.principal_id();
        let followed = if principal == owner {
            owner_member()?
        } else if !member.status().carries_permissions() {
            member.clone()
        } else if let Some(in_parent) = parent
            .member(principal)
            .filter(|in_parent| in_parent.has_joined())
        {
            let runs_it = in_parent.permissions().manage() || managers.contains(&principal);
            PrivateRoomMember::restore(
                principal,
                member.status(),
                sub_room_permissions(in_parent.permissions(), runs_it)?,
            )?
        } else {
            PrivateRoomMember::restore(
                principal,
                PrivateRoomMembershipStatus::Removed,
                PrivateRoomPermissions::NONE,
            )?
        };
        members.push(followed);
    }
    if sub_room.member(owner).is_none() {
        members.push(owner_member()?);
    }
    let status = match parent.status() {
        PrivateRoomLifecycleStatus::Archived => PrivateRoomLifecycleStatus::Archived,
        PrivateRoomLifecycleStatus::Active => sub_room.status(),
    };
    let unchanged = owner == sub_room.owner_principal_id()
        && status == sub_room.status()
        && members
            .iter()
            .all(|member| sub_room.member(member.principal_id()) == Some(member));
    if unchanged {
        return Ok(None);
    }
    PrivateRoom::restore(
        sub_room.catalog_id(),
        owner,
        status,
        members,
        sub_room.version().next()?,
        sub_room.created_by_agent(),
    )
    .map(Some)
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
    fn 没有门的父房间里的人都能进_权限照父房间里的_管理员进来就管这一间() {
        let parent = parent();
        let sub_room = new_sub_room(catalog(), &parent.room, parent.owner).expect("房主能建");
        let join = |who| authorize_sub_room_join(&parent.room, &sub_room, who, RoomJoinMode::Open);

        assert_eq!(join(parent.member), Ok(permissions(&SPEAKER)));
        assert_eq!(
            join(parent.viewer),
            Ok(permissions(&[PrivateRoomCapability::View]))
        );
        // 父房间的管理员：发言照父房间里的，邀请不带进来，管理给上。
        assert_eq!(
            join(parent.admin),
            Ok(permissions(&[
                PrivateRoomCapability::View,
                PrivateRoomCapability::Speak,
                PrivateRoomCapability::Manage,
            ]))
        );
        // 还没接受父房间邀请的、根本不在父房间里的，进不来。
        assert_eq!(join(parent.invited), Err(SubRoomEntryRefusal::NotInParent));
        assert_eq!(join(principal()), Err(SubRoomEntryRefusal::NotInParent));
    }

    #[test]
    fn 有门的只有房主和管理员不用批准_封禁着的和归档了的谁都进不来() {
        let parent = parent();
        let mut sub_room = new_sub_room(catalog(), &parent.room, parent.owner).expect("房主能建");
        for (access, refusal) in [
            (RoomJoinMode::Request, SubRoomEntryRefusal::ApprovalRequired),
            (RoomJoinMode::InviteOnly, SubRoomEntryRefusal::InviteOnly),
        ] {
            let join = |who| authorize_sub_room_join(&parent.room, &sub_room, who, access);
            assert_eq!(join(parent.member), Err(refusal));
            assert!(join(parent.admin).is_ok(), "父房间的管理员不看门");
            assert!(join(parent.owner).is_ok(), "房主不看门");
            assert_eq!(join(principal()), Err(SubRoomEntryRefusal::NotInParent));
        }

        // 已经在里面的再点一次不看门，权限还是他现在的。
        sub_room
            .admit(parent.viewer, permissions(&SPEAKER))
            .expect("先进来");
        assert_eq!(
            authorize_sub_room_join(
                &parent.room,
                &sub_room,
                parent.viewer,
                RoomJoinMode::InviteOnly
            ),
            Ok(permissions(&SPEAKER))
        );

        // 在这一间里被封禁着：没有门也进不来，父房间的管理员也一样。
        for who in [parent.member, parent.admin] {
            sub_room.admit(who, permissions(&SPEAKER)).expect("先进来");
            sub_room.ban_member(parent.owner, who).expect("房主能封禁");
            assert_eq!(
                authorize_sub_room_join(&parent.room, &sub_room, who, RoomJoinMode::Open),
                Err(SubRoomEntryRefusal::Banned)
            );
        }

        // 子房间归档了，或者父房间归档了。
        let mut archived = sub_room.clone();
        archived.archive(parent.owner).expect("房主能归档");
        assert_eq!(
            authorize_sub_room_join(&parent.room, &archived, parent.viewer, RoomJoinMode::Open),
            Err(SubRoomEntryRefusal::Archived)
        );
        let mut closed = parent.room.clone();
        closed.archive(parent.owner).expect("房主能归档");
        assert_eq!(
            authorize_sub_room_join(&closed, &sub_room, parent.viewer, RoomJoinMode::Open),
            Err(SubRoomEntryRefusal::Archived)
        );
    }

    #[test]
    fn 门_没有门的不能藏_拆门的时候隐藏跟着去掉_没带的那一项不动() {
        assert_eq!(SubRoomDoor::NONE.access(), RoomJoinMode::Open);
        assert!(!SubRoomDoor::NONE.has_door());
        assert!(SubRoomDoor::new(RoomJoinMode::Open, true).is_err());
        let hidden = SubRoomDoor::new(RoomJoinMode::Request, true).expect("有门的能藏");
        assert!(hidden.has_door() && hidden.hidden());
        assert!(
            SubRoomDoor::new(RoomJoinMode::InviteOnly, false)
                .expect("只能请进来")
                .has_door()
        );

        // 没带的不动。
        assert_eq!(hidden.changed(None, None), Ok(hidden));
        let invite_only = hidden
            .changed(Some(RoomJoinMode::InviteOnly), None)
            .expect("换一种门");
        assert_eq!(invite_only.access(), RoomJoinMode::InviteOnly);
        assert!(invite_only.hidden(), "换一种门不动隐藏");
        let shown = hidden.changed(None, Some(false)).expect("去掉隐藏");
        assert_eq!(shown.access(), RoomJoinMode::Request);
        assert!(!shown.hidden());

        // 拆门：没提隐藏的跟着去掉；明说还要藏的拒绝。
        assert_eq!(
            hidden.changed(Some(RoomJoinMode::Open), None),
            Ok(SubRoomDoor::NONE)
        );
        assert!(
            hidden
                .changed(Some(RoomJoinMode::Open), Some(true))
                .is_err()
        );
        // 没有门的单勾隐藏也拒绝；装门的同时勾可以。
        assert!(SubRoomDoor::NONE.changed(None, Some(true)).is_err());
        assert_eq!(
            SubRoomDoor::NONE.changed(Some(RoomJoinMode::Request), Some(true)),
            Ok(hidden)
        );
    }

    #[test]
    fn 管这一间的是父房间的房主和管理员_加上这一间里有管理的人() {
        let parent = parent();
        let sub_room = new_sub_room(catalog(), &parent.room, parent.member).expect("成员能建");

        assert!(runs_parent_room(&parent.room, parent.owner));
        assert!(runs_parent_room(&parent.room, parent.admin));
        assert!(!runs_parent_room(&parent.room, parent.member));
        assert!(!runs_parent_room(&parent.room, principal()));

        // 建的人管这一间；父房间的房主和管理员进没进来都管得了；别的人管不了。
        assert!(manages_sub_room(&parent.room, &sub_room, parent.member));
        assert!(manages_sub_room(&parent.room, &sub_room, parent.owner));
        assert!(manages_sub_room(&parent.room, &sub_room, parent.admin));
        assert!(!manages_sub_room(&parent.room, &sub_room, parent.viewer));
        assert!(!manages_sub_room(&parent.room, &sub_room, principal()));

        // 建的人被移出父房间以后（子房间这边还没对账），也管不了了。
        let mut without = parent.room.clone();
        without
            .remove_member(parent.owner, parent.member)
            .expect("房主能移出");
        assert!(!manages_sub_room(&without, &sub_room, parent.member));
    }

    /// 父房间的成员、管理员、只能看的人都进了的一间子房间，成员建的（他是这一间的管理员）。
    fn crowded(parent: &Parent) -> PrivateRoom {
        let mut sub_room = new_sub_room(catalog(), &parent.room, parent.member).expect("成员能建");
        for who in [parent.admin, parent.viewer] {
            let granted = authorize_sub_room_join(&parent.room, &sub_room, who, RoomJoinMode::Open)
                .expect("没有门，进得来");
            sub_room.admit(who, granted).expect("记得进去");
        }
        sub_room
    }

    fn standing(room: &PrivateRoom, who: PrincipalId) -> (PrivateRoomMembershipStatus, u8) {
        let member = room.member(who).expect("名单里有他");
        (member.status(), member.permissions().bits())
    }

    #[test]
    fn 父房间没变_子房间不用改_做几遍都一样() {
        let parent = parent();
        let sub_room = crowded(&parent);
        let managers = BTreeSet::from([parent.member]);

        assert_eq!(
            follow_parent_room(&parent.room, &sub_room, &managers),
            Ok(None)
        );

        // 改过一遍以后再做一遍，没有要改的。
        let mut changed = parent.room.clone();
        changed
            .remove_member(parent.owner, parent.viewer)
            .expect("房主能移出");
        let followed = follow_parent_room(&changed, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(
            followed.version(),
            sub_room.version().next().expect("还有版本")
        );
        assert_eq!(follow_parent_room(&changed, &followed, &managers), Ok(None));
    }

    #[test]
    fn 被移出父房间的人从子房间出去_退出的被封禁的一样_已经出去的原样留着() {
        let parent = parent();
        let mut sub_room = crowded(&parent);
        // 只能看的人先被这一间封禁了：这一条不跟着父房间变。
        sub_room
            .ban_member(parent.owner, parent.viewer)
            .expect("房主能封禁");
        let managers = BTreeSet::from([parent.member]);

        let mut changed = parent.room.clone();
        changed
            .remove_member(parent.owner, parent.member)
            .expect("房主能移出");
        changed
            .ban_member(parent.owner, parent.admin)
            .expect("房主能封禁");
        changed
            .remove_member(parent.owner, parent.viewer)
            .expect("房主能移出");
        let followed = follow_parent_room(&changed, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");

        let removed = (PrivateRoomMembershipStatus::Removed, 0);
        assert_eq!(standing(&followed, parent.member), removed);
        assert_eq!(
            standing(&followed, parent.admin),
            removed,
            "封禁只记在父房间里"
        );
        assert_eq!(
            standing(&followed, parent.viewer),
            (PrivateRoomMembershipStatus::Banned, 0)
        );
        assert_eq!(followed.owner_principal_id(), parent.owner);
        assert!(followed.member(parent.owner).is_some_and(|owner| {
            owner.has_joined() && owner.permissions() == PrivateRoomPermissions::ALL
        }));
    }

    #[test]
    fn 权限照父房间里的重算_管理给父房间的管理员和这一间的管理员() {
        let parent = parent();
        let sub_room = crowded(&parent);
        let managers = BTreeSet::from([parent.member]);
        let view = permissions(&[PrivateRoomCapability::View]);

        let mut changed = parent.room.clone();
        // 成员在父房间里改成只能看：子房间里跟着，他是这一间的管理员，“管理”还在。
        changed
            .update_permissions(parent.owner, parent.member, view)
            .expect("房主能改");
        // 管理员在父房间里不当了：子房间里的“管理”跟着没有。
        changed
            .update_permissions(parent.owner, parent.admin, permissions(&SPEAKER))
            .expect("房主能改");
        // 只能看的人当上了父房间的管理员：进了的子房间里也管。
        let admin = permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ]);
        changed
            .update_permissions(parent.owner, parent.viewer, admin)
            .expect("房主能设管理员");
        let followed = follow_parent_room(&changed, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");

        let joined = PrivateRoomMembershipStatus::Joined;
        assert_eq!(
            standing(&followed, parent.member),
            (
                joined,
                permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Manage]).bits()
            )
        );
        assert_eq!(
            standing(&followed, parent.admin),
            (joined, permissions(&SPEAKER).bits())
        );
        assert_eq!(
            standing(&followed, parent.viewer),
            (
                joined,
                permissions(&[
                    PrivateRoomCapability::View,
                    PrivateRoomCapability::Speak,
                    PrivateRoomCapability::Manage,
                ])
                .bits()
            ),
            "邀请不带进子房间"
        );

        // 不再是这一间的管理员以后，“管理”也没有了。
        let demoted = follow_parent_room(&changed, &followed, &BTreeSet::new())
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(standing(&demoted, parent.member), (joined, view.bits()));
    }

    #[test]
    fn 父房间转让了_子房间一起转_新房主没进来过也在里面_封禁着的也一样() {
        let parent = parent();
        let mut sub_room = crowded(&parent);
        sub_room
            .ban_member(parent.owner, parent.viewer)
            .expect("房主能封禁");
        let managers = BTreeSet::from([parent.member]);
        let joined = PrivateRoomMembershipStatus::Joined;
        let all = PrivateRoomPermissions::ALL.bits();

        // 转给在这一间里被封禁着的人：子房间拦不住这间房自己的房主。原来的房主留下当成员。
        let mut to_banned = parent.room.clone();
        to_banned
            .transfer_ownership(parent.owner, parent.viewer, permissions(&SPEAKER))
            .expect("房主能转让");
        let followed = follow_parent_room(&to_banned, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(followed.owner_principal_id(), parent.viewer);
        assert_eq!(standing(&followed, parent.viewer), (joined, all));
        assert_eq!(
            standing(&followed, parent.owner),
            (joined, permissions(&SPEAKER).bits())
        );

        // 转给从没进过这一间的人：名单里添上他。
        let newcomer = principal();
        let mut to_newcomer = parent.room.clone();
        to_newcomer
            .invite(parent.owner, newcomer, permissions(&SPEAKER))
            .expect("房主能邀请");
        to_newcomer.accept_invitation(newcomer).expect("能接受");
        to_newcomer
            .transfer_ownership(
                parent.owner,
                newcomer,
                permissions(&[PrivateRoomCapability::View]),
            )
            .expect("房主能转让");
        let followed = follow_parent_room(&to_newcomer, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(followed.owner_principal_id(), newcomer);
        assert_eq!(standing(&followed, newcomer), (joined, all));
        assert_eq!(
            standing(&followed, parent.owner),
            (joined, permissions(&[PrivateRoomCapability::View]).bits())
        );
        assert_eq!(
            follow_parent_room(&to_newcomer, &followed, &managers),
            Ok(None)
        );
    }

    #[test]
    fn 父房间归档了_子房间跟着归档_子房间自己归档的不跟着回来() {
        let parent = parent();
        let sub_room = crowded(&parent);
        let managers = BTreeSet::new();

        let mut closed = parent.room.clone();
        closed.archive(parent.owner).expect("房主能归档");
        let followed = follow_parent_room(&closed, &sub_room, &managers)
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(followed.status(), PrivateRoomLifecycleStatus::Archived);

        // 子房间自己归档了、父房间还在用：那是它自己的事。归档了的照样跟着父房间的名单变。
        let mut archived = sub_room.clone();
        archived.archive(parent.owner).expect("房主能归档");
        let mut changed = parent.room.clone();
        changed
            .remove_member(parent.owner, parent.viewer)
            .expect("房主能移出");
        let followed = follow_parent_room(&changed, &archived, &managers)
            .expect("算得出")
            .expect("有要改的");
        assert_eq!(followed.status(), PrivateRoomLifecycleStatus::Archived);
        assert_eq!(
            standing(&followed, parent.viewer),
            (PrivateRoomMembershipStatus::Removed, 0)
        );
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
