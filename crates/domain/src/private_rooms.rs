use std::collections::BTreeMap;

use crate::{
    DomainError, DomainResult,
    ids::{AgentId, PrincipalId, RoomCatalogId},
    version::AggregateVersion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomLifecycleStatus {
    Active,
    Archived,
}

impl PrivateRoomLifecycleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

impl TryFrom<&str> for PrivateRoomLifecycleStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            _ => Err(validation(
                "private_room_lifecycle_status",
                "包含未知生命周期状态",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomMembershipStatus {
    Invited,
    Joined,
    Declined,
    Removed,
    Banned,
}

impl PrivateRoomMembershipStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invited => "invited",
            Self::Joined => "joined",
            Self::Declined => "declined",
            Self::Removed => "removed",
            Self::Banned => "banned",
        }
    }

    pub const fn carries_permissions(self) -> bool {
        matches!(self, Self::Invited | Self::Joined)
    }
}

impl TryFrom<&str> for PrivateRoomMembershipStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "invited" => Ok(Self::Invited),
            "joined" => Ok(Self::Joined),
            "declined" => Ok(Self::Declined),
            "removed" => Ok(Self::Removed),
            "banned" => Ok(Self::Banned),
            _ => Err(validation(
                "private_room_membership_status",
                "包含未知成员状态",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomCapability {
    View,
    Speak,
    Invite,
    Manage,
    Automate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRoomPermissions {
    bits: u8,
}

impl PrivateRoomPermissions {
    const VIEW_BIT: u8 = 1;
    const SPEAK_BIT: u8 = 1 << 1;
    const INVITE_BIT: u8 = 1 << 2;
    const MANAGE_BIT: u8 = 1 << 3;
    const AUTOMATE_BIT: u8 = 1 << 4;
    const KNOWN_BITS: u8 =
        Self::VIEW_BIT | Self::SPEAK_BIT | Self::INVITE_BIT | Self::MANAGE_BIT | Self::AUTOMATE_BIT;

    pub const NONE: Self = Self { bits: 0 };
    pub const ALL: Self = Self {
        bits: Self::KNOWN_BITS,
    };
    /// 凭口令进来的 Agent 成员：只能查看和发言。
    pub const AGENT_MEMBER: Self = Self {
        bits: Self::VIEW_BIT | Self::SPEAK_BIT,
    };
    /// Agent 替主体邀请的人：查看、发言和自动发言，邀请和管理留给房主。私人房间里 Agent 发言要它
    /// 主人有自动发言，不给的话这个人请进来的 Agent 只进得来、说不了话（维护者 2026-10-12 定）。
    pub const INVITED_BY_AGENT: Self = Self {
        bits: Self::VIEW_BIT | Self::SPEAK_BIT | Self::AUTOMATE_BIT,
    };

    /// 创建相互一致的私人房间权限集合。
    ///
    /// # Errors
    ///
    /// 能力集合包含未知位、没有查看权限却携带其他权限，或允许自动发送却禁止发言时返回错误。
    pub const fn from_bits(bits: u8) -> DomainResult<Self> {
        if bits & !Self::KNOWN_BITS != 0 {
            return Err(validation("private_room_permissions", "包含未知权限位"));
        }
        let permissions = Self { bits };
        if bits != 0 && !permissions.view() {
            return Err(validation(
                "private_room_permissions",
                "任何房间能力都必须以查看权限为前提",
            ));
        }
        if permissions.automate() && !permissions.speak() {
            return Err(validation(
                "private_room_permissions",
                "自动发送必须同时具备发言权限",
            ));
        }
        Ok(permissions)
    }

    /// 从显式能力集合创建权限，重复能力会自然去重。
    ///
    /// # Errors
    ///
    /// 能力依赖关系不完整时返回错误。
    pub fn from_capabilities(
        capabilities: impl IntoIterator<Item = PrivateRoomCapability>,
    ) -> DomainResult<Self> {
        let bits = capabilities.into_iter().fold(0, |bits, capability| {
            bits | Self::capability_bit(capability)
        });
        Self::from_bits(bits)
    }

    pub const fn view(self) -> bool {
        self.bits & Self::VIEW_BIT != 0
    }

    pub const fn speak(self) -> bool {
        self.bits & Self::SPEAK_BIT != 0
    }

    pub const fn invite(self) -> bool {
        self.bits & Self::INVITE_BIT != 0
    }

    pub const fn manage(self) -> bool {
        self.bits & Self::MANAGE_BIT != 0
    }

    pub const fn automate(self) -> bool {
        self.bits & Self::AUTOMATE_BIT != 0
    }

    pub const fn allows(self, capability: PrivateRoomCapability) -> bool {
        self.bits & Self::capability_bit(capability) != 0
    }

    pub const fn bits(self) -> u8 {
        self.bits
    }

    const fn can_be_assigned_to_member(self) -> bool {
        self.view()
    }

    /// `self` 比 `previous` 多出来的权限。
    const fn added_since(self, previous: Self) -> Self {
        Self {
            bits: self.bits & !previous.bits,
        }
    }

    /// 两边都有的权限。查看是一切权限的前提，自动发言要有发言，两边都满足时交集也满足。
    const fn intersection(self, other: Self) -> Self {
        Self {
            bits: self.bits & other.bits,
        }
    }

    const fn contains(self, other: Self) -> bool {
        other.bits & !self.bits == 0
    }

    const fn capability_bit(capability: PrivateRoomCapability) -> u8 {
        match capability {
            PrivateRoomCapability::View => Self::VIEW_BIT,
            PrivateRoomCapability::Speak => Self::SPEAK_BIT,
            PrivateRoomCapability::Invite => Self::INVITE_BIT,
            PrivateRoomCapability::Manage => Self::MANAGE_BIT,
            PrivateRoomCapability::Automate => Self::AUTOMATE_BIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomMember {
    principal_id: PrincipalId,
    status: PrivateRoomMembershipStatus,
    permissions: PrivateRoomPermissions,
}

impl PrivateRoomMember {
    /// 恢复成员事实并验证状态与权限是否一致。
    ///
    /// # Errors
    ///
    /// 邀请或已加入成员没有查看权限，或终态成员仍保留权限时返回错误。
    pub const fn restore(
        principal_id: PrincipalId,
        status: PrivateRoomMembershipStatus,
        permissions: PrivateRoomPermissions,
    ) -> DomainResult<Self> {
        if status.carries_permissions() != permissions.can_be_assigned_to_member() {
            return Err(invariant(
                "private_room_member",
                "邀请和已加入成员必须有权限，终态成员不得保留权限",
            ));
        }
        Ok(Self {
            principal_id,
            status,
            permissions,
        })
    }

    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    pub const fn status(&self) -> PrivateRoomMembershipStatus {
        self.status
    }

    pub const fn permissions(&self) -> PrivateRoomPermissions {
        self.permissions
    }

    pub const fn has_joined(&self) -> bool {
        matches!(self.status, PrivateRoomMembershipStatus::Joined)
    }

    const fn allows(&self, capability: PrivateRoomCapability) -> bool {
        self.has_joined() && self.permissions.allows(capability)
    }

    fn replace(
        &mut self,
        status: PrivateRoomMembershipStatus,
        permissions: PrivateRoomPermissions,
    ) {
        self.status = status;
        self.permissions = permissions;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoom {
    catalog_id: RoomCatalogId,
    owner_principal_id: PrincipalId,
    status: PrivateRoomLifecycleStatus,
    members: BTreeMap<PrincipalId, PrivateRoomMember>,
    version: AggregateVersion,
    created_by_agent: Option<AgentId>,
}

impl PrivateRoom {
    pub fn create(catalog_id: RoomCatalogId, owner_principal_id: PrincipalId) -> Self {
        let owner = PrivateRoomMember {
            principal_id: owner_principal_id,
            status: PrivateRoomMembershipStatus::Joined,
            permissions: PrivateRoomPermissions::ALL,
        };
        Self {
            catalog_id,
            owner_principal_id,
            status: PrivateRoomLifecycleStatus::Active,
            members: BTreeMap::from([(owner_principal_id, owner)]),
            version: AggregateVersion::INITIAL,
            created_by_agent: None,
        }
    }

    /// Agent 替房主建的房间（`specs/agent-rooms/design.md`）。房主照旧是主体：本机 Agent 替
    /// 账号建时是人，网络 Agent 建时是它自己的合成主体；记下是哪个 Agent 建的，Agent 只管这种房间。
    pub fn create_by_agent(
        catalog_id: RoomCatalogId,
        owner_principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> Self {
        Self {
            created_by_agent: Some(agent_id),
            ..Self::create(catalog_id, owner_principal_id)
        }
    }

    /// 从持久化事实恢复私人房间聚合。
    ///
    /// # Errors
    ///
    /// 成员重复，或房主不是唯一具有完整权限的已加入成员时返回错误。
    pub fn restore(
        catalog_id: RoomCatalogId,
        owner_principal_id: PrincipalId,
        status: PrivateRoomLifecycleStatus,
        members: impl IntoIterator<Item = PrivateRoomMember>,
        version: AggregateVersion,
        created_by_agent: Option<AgentId>,
    ) -> DomainResult<Self> {
        let mut indexed_members = BTreeMap::new();
        for member in members {
            if indexed_members
                .insert(member.principal_id(), member)
                .is_some()
            {
                return Err(invariant(
                    "private_room",
                    "同一主体不能重复出现在成员集合中",
                ));
            }
        }
        let owner = indexed_members
            .get(&owner_principal_id)
            .ok_or_else(|| invariant("private_room", "房主必须存在于成员集合中"))?;
        if !owner.has_joined() || owner.permissions() != PrivateRoomPermissions::ALL {
            return Err(invariant(
                "private_room",
                "房主必须是具备完整权限的已加入成员",
            ));
        }
        Ok(Self {
            catalog_id,
            owner_principal_id,
            status,
            members: indexed_members,
            version,
            created_by_agent,
        })
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn owner_principal_id(&self) -> PrincipalId {
        self.owner_principal_id
    }

    /// 建这个房间的 Agent；人建的为空。
    pub const fn created_by_agent(&self) -> Option<AgentId> {
        self.created_by_agent
    }

    pub const fn status(&self) -> PrivateRoomLifecycleStatus {
        self.status
    }

    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    pub fn members(&self) -> impl ExactSizeIterator<Item = &PrivateRoomMember> {
        self.members.values()
    }

    pub fn member(&self, principal_id: PrincipalId) -> Option<&PrivateRoomMember> {
        self.members.get(&principal_id)
    }

    /// 这个人此刻是不是管理员：不是房主，受邀或已加入，身上有“管理”
    /// （`specs/redesign/room-roles.md`）。
    #[must_use]
    pub fn is_admin(&self, principal_id: PrincipalId) -> bool {
        principal_id != self.owner_principal_id
            && self.members.get(&principal_id).is_some_and(|member| {
                member.status.carries_permissions() && member.permissions.manage()
            })
    }

    /// 移出、封禁、禁言一个人，或者撤回这些动作之前：对象是管理员时，动手的必须是房主。管理员之间
    /// 谁也动不了谁，自己也解不了房主给自己的禁言。动手的人管不管得了这间房由调用的一方另外核。
    ///
    /// # Errors
    ///
    /// 对象是管理员而动手的不是房主时返回 [`DomainError::OwnerOnly`]。
    pub fn authorize_acting_on(
        &self,
        actor: PrincipalId,
        target: PrincipalId,
        action: &'static str,
    ) -> DomainResult<()> {
        if actor != self.owner_principal_id && self.is_admin(target) {
            return Err(DomainError::OwnerOnly { action });
        }
        Ok(())
    }

    /// 计算当前产品策略是否授予某项能力。
    ///
    /// 归档房间保留历史查看能力，但关闭发言、邀请、治理和自动发送能力。
    pub fn allows(&self, principal_id: PrincipalId, capability: PrivateRoomCapability) -> bool {
        if self.status == PrivateRoomLifecycleStatus::Archived
            && capability != PrivateRoomCapability::View
        {
            return false;
        }
        self.members
            .get(&principal_id)
            .is_some_and(|member| member.allows(capability))
    }

    /// 判断代表某个主体行动的 Agent 能否进入这个私人房间。
    ///
    /// 参数是 Agent 此次代表的主体，即驱动该实例的设备所属账号，而不是 Agent 的任一共同所有者。
    /// Agent 不是房间成员，它随该主体入场且能力不超过它：该主体必须处于已加入状态并具备发言能力，
    /// 仅被邀请、已拒绝、已离开、已移除和被封禁都不满足，归档房间关闭发言因此也不再接纳 Agent。
    ///
    /// 由成员资格单点裁决，移除或封禁一个成员会立即让其全部 Agent 失去房间，不需要另一套级联
    /// 清理，也不会出现 Agent 仍在而其主体已离开的矛盾状态。
    #[must_use]
    pub fn admits_agent_of(&self, principal_id: PrincipalId) -> bool {
        self.member(principal_id)
            .is_some_and(PrivateRoomMember::has_joined)
            && self.allows(principal_id, PrivateRoomCapability::Speak)
    }

    /// 判断代表某个主体行动的 Agent 能否在这个私人房间自主发送。
    ///
    /// 在入场条件之上还要求该主体具备自动发送能力。这只是房间侧的上限，接待仍然另外要求一份
    /// 范围受限且会过期的自动发言授权，两者都满足才会真正发送。
    #[must_use]
    pub fn admits_autonomous_agent_of(&self, principal_id: PrincipalId) -> bool {
        self.admits_agent_of(principal_id)
            && self.allows(principal_id, PrivateRoomCapability::Automate)
    }

    /// 判断凭口令进来的 Agent 成员能否入场：它本身仍是有效的 Agent 成员，且房间还在使用中。
    ///
    /// 这类 Agent 不随任何成员进出，它的主人也不因此成为房间成员；房主或管理员可以单独移出它。
    /// 归档房间关闭发言，同样不再接纳它。
    #[must_use]
    pub fn admits_agent_member(&self, is_agent_member: bool) -> bool {
        is_agent_member && self.status == PrivateRoomLifecycleStatus::Active
    }

    /// 生成、更换、停用 Agent 口令，或移出凭口令进来的 Agent：需要管理权限，房间还在使用中。
    ///
    /// # Errors
    ///
    /// 操作者没有管理权限，或房间已归档时返回错误。
    pub fn authorize_agent_access(&self, actor: PrincipalId) -> DomainResult<()> {
        self.ensure_active("agent_access_changed")?;
        self.require(actor, PrivateRoomCapability::Manage, "管理 Agent 口令")
    }

    /// Agent 替主体管口令、敲门和凭口令进来的 Agent：只管 Agent 建的房间，在此之上和人一样
    /// 要管理权限、房间还在使用中。人建的房间要不要对外开放由人决定（`specs/agent-rooms/design.md`）。
    ///
    /// # Errors
    ///
    /// 房间不是 Agent 建的、操作者没有管理权限，或房间已归档时返回错误。
    pub fn authorize_agent_access_by_agent(&self, actor: PrincipalId) -> DomainResult<()> {
        self.ensure_built_by_agent("Agent 只管 Agent 建的房间")?;
        self.authorize_agent_access(actor)
    }

    /// Agent 替主体邀请人：只在 Agent 建的房间里，被邀请的人能查看、发言和自动发言
    /// （`INVITED_BY_AGENT`），再不超过这个主体自己有的。Agent 只是想让这个人进来，所以他已经在
    /// 房间里也算成功，什么都不改。
    ///
    /// # Errors
    ///
    /// 房间不是 Agent 建的，或按 `invite` 的规则不能邀请时返回错误。
    pub fn invite_by_agent(
        &mut self,
        actor: PrincipalId,
        target: PrincipalId,
    ) -> DomainResult<bool> {
        self.ensure_built_by_agent("Agent 只在 Agent 建的房间里邀请人")?;
        if self
            .members
            .get(&target)
            .is_some_and(PrivateRoomMember::has_joined)
        {
            self.ensure_active("invited")?;
            self.require(actor, PrivateRoomCapability::Invite, "邀请私人房间成员")?;
            return Ok(false);
        }
        self.invite(actor, target, self.default_invitation_for(actor))
    }

    /// 邀请默认给的权限：查看、发言、自动发言，被邀请的人带进来的 Agent 能说话
    /// （`specs/room-people/design.md`）；不超过邀请人自己有的。
    #[must_use]
    pub fn default_invitation_for(&self, inviter: PrincipalId) -> PrivateRoomPermissions {
        PrivateRoomPermissions::INVITED_BY_AGENT.intersection(self.permissions_of(inviter))
    }

    /// 生成、再看、停用邀请链接：已加入、有邀请权限、房间还在使用中。链接以创建人的名义邀请，
    /// 所以创建人不再满足这一条时，他的链接也不管用了。
    ///
    /// # Errors
    ///
    /// 不是已加入的成员、没有邀请权限或房间已归档时返回错误。
    pub fn authorize_invite_link(&self, actor: PrincipalId) -> DomainResult<()> {
        self.ensure_active("invite_link_changed")?;
        if !self
            .members
            .get(&actor)
            .is_some_and(PrivateRoomMember::has_joined)
        {
            return Err(DomainError::Forbidden {
                action: "管理私人房间邀请链接",
            });
        }
        self.require(actor, PrivateRoomCapability::Invite, "管理私人房间邀请链接")
    }

    /// 新链接带的权限：在 [`Self::authorize_invite_link`] 之上，能给成员、不超过自己有的；带“管理”
    /// 的只有房主给得出。
    ///
    /// # Errors
    ///
    /// 不能管理邀请链接、权限不能给成员、给了自己没有的权限，或者房主以外的人带上了“管理”时返回错误。
    pub fn authorize_invite_link_permissions(
        &self,
        actor: PrincipalId,
        permissions: PrivateRoomPermissions,
    ) -> DomainResult<()> {
        self.authorize_invite_link(actor)?;
        require_assignable_permissions(permissions)?;
        self.require_owner_to_appoint(actor, permissions, "邀请链接带上私人房间的管理权限")?;
        self.require_grantable(actor, PrivateRoomPermissions::NONE, permissions)
    }

    fn ensure_built_by_agent(&self, action: &'static str) -> DomainResult<()> {
        if self.created_by_agent.is_some() {
            Ok(())
        } else {
            Err(DomainError::Forbidden { action })
        }
    }

    /// 邀请成员进入活跃房间，给的权限不超过邀请人自己有的；一进来就是管理员的只有房主请得了。
    ///
    /// # Errors
    ///
    /// 操作者无邀请权限、给了自己没有的权限、房主以外的人带上了“管理”、房间已归档、目标已加入或
    /// 被封禁时返回错误。
    pub fn invite(
        &mut self,
        actor: PrincipalId,
        target: PrincipalId,
        permissions: PrivateRoomPermissions,
    ) -> DomainResult<bool> {
        self.ensure_active("invited")?;
        self.require(actor, PrivateRoomCapability::Invite, "邀请私人房间成员")?;
        require_assignable_permissions(permissions)?;
        self.require_owner_to_appoint(actor, permissions, "邀请时设置私人房间管理员")?;
        self.require_grantable(actor, PrivateRoomPermissions::NONE, permissions)?;

        match self.members.get(&target) {
            Some(member) if member.status == PrivateRoomMembershipStatus::Invited => {
                if member.permissions == permissions {
                    return Ok(false);
                }
                return Err(invalid_member_transition(
                    "invited",
                    "invited_with_new_permissions",
                ));
            }
            Some(member)
                if !matches!(
                    member.status,
                    PrivateRoomMembershipStatus::Declined | PrivateRoomMembershipStatus::Removed
                ) =>
            {
                return Err(invalid_member_transition(member.status.as_str(), "invited"));
            }
            None | Some(_) => {}
        }
        let next_version = self.next_version()?;
        match self.members.get_mut(&target) {
            None => {
                self.members.insert(
                    target,
                    PrivateRoomMember {
                        principal_id: target,
                        status: PrivateRoomMembershipStatus::Invited,
                        permissions,
                    },
                );
            }
            Some(member) => {
                member.replace(PrivateRoomMembershipStatus::Invited, permissions);
            }
        }
        self.version = next_version;
        Ok(true)
    }

    /// 接受当前主体收到的邀请。
    ///
    /// # Errors
    ///
    /// 房间已归档或主体没有待接受邀请时返回错误。
    pub fn accept_invitation(&mut self, principal_id: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("joined")?;
        let status = self
            .members
            .get(&principal_id)
            .map(PrivateRoomMember::status)
            .ok_or_else(|| invalid_member_transition("absent", "joined"))?;
        match status {
            PrivateRoomMembershipStatus::Invited => {
                let next_version = self.next_version()?;
                self.members
                    .get_mut(&principal_id)
                    .ok_or_else(|| invariant("private_room", "已验证的邀请成员必须存在"))?
                    .status = PrivateRoomMembershipStatus::Joined;
                self.version = next_version;
                Ok(true)
            }
            PrivateRoomMembershipStatus::Joined => Ok(false),
            status => Err(invalid_member_transition(status.as_str(), "joined")),
        }
    }

    /// 拒绝当前主体收到的邀请。
    ///
    /// # Errors
    ///
    /// 房间已归档或主体没有待处理邀请时返回错误。
    pub fn decline_invitation(&mut self, principal_id: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("declined")?;
        let status = self
            .members
            .get(&principal_id)
            .map(PrivateRoomMember::status)
            .ok_or_else(|| invalid_member_transition("absent", "declined"))?;
        match status {
            PrivateRoomMembershipStatus::Invited => {
                let next_version = self.next_version()?;
                self.members
                    .get_mut(&principal_id)
                    .ok_or_else(|| invariant("private_room", "已验证的邀请成员必须存在"))?
                    .replace(
                        PrivateRoomMembershipStatus::Declined,
                        PrivateRoomPermissions::NONE,
                    );
                self.version = next_version;
                Ok(true)
            }
            PrivateRoomMembershipStatus::Declined => Ok(false),
            status => Err(invalid_member_transition(status.as_str(), "declined")),
        }
    }

    /// 主动离开私人房间；房主必须先转移所有权。
    ///
    /// # Errors
    ///
    /// 房主尝试直接离开、房间已归档或主体不是已加入成员时返回错误。
    pub fn leave(&mut self, principal_id: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("removed")?;
        if principal_id == self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "房主必须先转移所有权才能离开私人房间",
            });
        }
        self.terminate_membership(
            principal_id,
            PrivateRoomMembershipStatus::Removed,
            "removed",
        )
    }

    /// 移除已加入或待邀请成员。管理员只有房主移得动。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、目标是房主、目标是管理员而操作者不是房主，或房间已归档时返回错误。
    pub fn remove_member(&mut self, actor: PrincipalId, target: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("removed")?;
        self.require(actor, PrivateRoomCapability::Manage, "移除私人房间成员")?;
        self.require_non_owner_target(target, "移除私人房间房主")?;
        self.authorize_acting_on(actor, target, "移除私人房间管理员")?;
        self.terminate_membership(target, PrivateRoomMembershipStatus::Removed, "removed")
    }

    /// 封禁主体并立即撤销其全部产品权限。管理员只有房主封得了。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、目标是房主、目标是管理员而操作者不是房主，或房间已归档时返回错误。
    pub fn ban_member(&mut self, actor: PrincipalId, target: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("banned")?;
        self.require(actor, PrivateRoomCapability::Manage, "封禁私人房间成员")?;
        self.require_non_owner_target(target, "封禁私人房间房主")?;
        self.authorize_acting_on(actor, target, "封禁私人房间管理员")?;

        if self
            .members
            .get(&target)
            .is_some_and(|member| member.status == PrivateRoomMembershipStatus::Banned)
        {
            return Ok(false);
        }
        let next_version = self.next_version()?;
        match self.members.get_mut(&target) {
            Some(member) => member.replace(
                PrivateRoomMembershipStatus::Banned,
                PrivateRoomPermissions::NONE,
            ),
            None => {
                self.members.insert(
                    target,
                    PrivateRoomMember {
                        principal_id: target,
                        status: PrivateRoomMembershipStatus::Banned,
                        permissions: PrivateRoomPermissions::NONE,
                    },
                );
            }
        }
        self.version = next_version;
        Ok(true)
    }

    /// 撤销封禁：这个人回到“已移出”，之后可以再邀请。没有封着的什么都不改。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限或房间已归档时返回错误。
    pub fn unban_member(&mut self, actor: PrincipalId, target: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("removed")?;
        self.require(actor, PrivateRoomCapability::Manage, "解封私人房间成员")?;
        if !self
            .members
            .get(&target)
            .is_some_and(|member| member.status == PrivateRoomMembershipStatus::Banned)
        {
            return Ok(false);
        }
        let next_version = self.next_version()?;
        self.members
            .get_mut(&target)
            .ok_or_else(|| invariant("private_room", "已验证的封禁成员必须存在"))?
            .replace(
                PrivateRoomMembershipStatus::Removed,
                PrivateRoomPermissions::NONE,
            );
        self.version = next_version;
        Ok(true)
    }

    /// 更新已加入或待邀请成员的产品权限。加上的权限不超过操作者自己有的，收回不限。
    ///
    /// 设、撤管理员（给出或拿掉“管理”）只有房主能做，管理员身上的任何一项也只有房主能改；例外是
    /// 管理员自己不当了，把自己的“管理”拿掉（`specs/redesign/room-roles.md`）。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、加上了自己没有的权限、目标是房主、动了“管理”或管理员而操作者不是房主、
    /// 目标不在活跃成员态或权限为空时返回错误。
    pub fn update_permissions(
        &mut self,
        actor: PrincipalId,
        target: PrincipalId,
        permissions: PrivateRoomPermissions,
    ) -> DomainResult<bool> {
        self.ensure_active("permissions_updated")?;
        self.require(actor, PrivateRoomCapability::Manage, "修改私人房间权限")?;
        self.require_non_owner_target(target, "削弱私人房间房主权限")?;
        require_assignable_permissions(permissions)?;

        let member = self
            .members
            .get(&target)
            .ok_or_else(|| invalid_member_transition("absent", "permissions_updated"))?;
        if !member.status.carries_permissions() {
            return Err(invalid_member_transition(
                member.status.as_str(),
                "permissions_updated",
            ));
        }
        if member.permissions == permissions {
            return Ok(false);
        }
        self.require_owner_for_role_change(actor, target, member.permissions, permissions)?;
        self.require_grantable(actor, member.permissions, permissions)?;
        let next_version = self.next_version()?;
        self.members
            .get_mut(&target)
            .ok_or_else(|| invariant("private_room", "已验证的成员必须存在"))?
            .permissions = permissions;
        self.version = next_version;
        Ok(true)
    }

    /// 原子转移房主身份，并显式设置原房主转移后的权限。
    ///
    /// # Errors
    ///
    /// 操作者不是当前房主、目标未加入、房间已归档或原房主权限为空时返回错误。
    pub fn transfer_ownership(
        &mut self,
        actor: PrincipalId,
        target: PrincipalId,
        former_owner_permissions: PrivateRoomPermissions,
    ) -> DomainResult<bool> {
        self.ensure_active("ownership_transferred")?;
        if actor != self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "转移私人房间所有权",
            });
        }
        if target == self.owner_principal_id {
            return Ok(false);
        }
        require_assignable_permissions(former_owner_permissions)?;
        if !self
            .members
            .get(&target)
            .is_some_and(PrivateRoomMember::has_joined)
        {
            return Err(invalid_member_transition(
                self.members
                    .get(&target)
                    .map_or("absent", |member| member.status.as_str()),
                "owner",
            ));
        }
        if !self.members.contains_key(&actor) {
            return Err(invariant("private_room", "房主成员事实缺失"));
        }
        let next_version = self.next_version()?;

        let mut target_member = self
            .members
            .remove(&target)
            .ok_or_else(|| invariant("private_room", "已验证的新房主成员必须存在"))?;
        let Some(former_owner) = self.members.get_mut(&actor) else {
            self.members.insert(target, target_member);
            return Err(invariant("private_room", "房主成员事实缺失"));
        };
        former_owner.permissions = former_owner_permissions;
        target_member.permissions = PrivateRoomPermissions::ALL;
        self.members.insert(target, target_member);
        self.owner_principal_id = target;
        self.version = next_version;
        Ok(true)
    }

    /// 只有房主能给还在使用中的房间改名。
    ///
    /// # Errors
    ///
    /// 不是房主，或房间已归档时返回错误。
    pub fn authorize_rename(&self, actor: PrincipalId) -> DomainResult<()> {
        if actor != self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "修改私人房间名称",
            });
        }
        self.ensure_active("renamed")
    }

    /// 只有房主能给还在使用中的房间改简介，和改名一样（`specs/public-rooms/design.md`）。
    ///
    /// # Errors
    ///
    /// 不是房主，或房间已归档时返回错误。
    pub fn authorize_description_change(&self, actor: PrincipalId) -> DomainResult<()> {
        if actor != self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "修改私人房间简介",
            });
        }
        self.ensure_active("described")
    }

    /// 写、改、撤常驻公告（`specs/public-rooms/design.md`“常驻公告”）：房主和管理员（有“管理”这一位
    /// 的已加入成员）都能，不用最近登录过，这是日常管理。房间归档以后公告还看得到，不能再改。
    ///
    /// # Errors
    ///
    /// 房间已归档，或操作者没有管理权限时返回错误。
    pub fn authorize_notice_change(&self, actor: PrincipalId) -> DomainResult<()> {
        self.ensure_active("notice_changed")?;
        self.require(actor, PrivateRoomCapability::Manage, "修改房间公告")
    }

    /// 把房间交给 Agent 管、看和撤这些授权（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：
    /// 房主和管理员，房间还要在使用中。
    ///
    /// # Errors
    ///
    /// 房间已归档，或操作者没有管理权限时返回错误。
    pub fn authorize_moderation_grants(&self, actor: PrincipalId) -> DomainResult<()> {
        self.ensure_active("moderation_grants_changed")?;
        self.require(actor, PrivateRoomCapability::Manage, "把房间交给 Agent 管")
    }

    /// 切换公开和私密、改加入方式（`specs/public-rooms/design.md`）：只有房主，管理员不能，
    /// 这是房间的性质，不是日常管理。房间还要在使用中。
    ///
    /// # Errors
    ///
    /// 不是房主，或房间已归档时返回错误。
    pub fn authorize_listing_change(&self, actor: PrincipalId) -> DomainResult<()> {
        if actor != self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "修改私人房间的公开设置",
            });
        }
        self.ensure_active("listing_changed")
    }

    /// 归档房间，重复归档保持幂等。
    ///
    /// # Errors
    ///
    /// 只有当前房主可以归档房间。
    pub fn archive(&mut self, actor: PrincipalId) -> DomainResult<bool> {
        if actor != self.owner_principal_id {
            return Err(DomainError::Forbidden {
                action: "归档私人房间",
            });
        }
        if self.status == PrivateRoomLifecycleStatus::Archived {
            return Ok(false);
        }
        let next_version = self.next_version()?;
        self.status = PrivateRoomLifecycleStatus::Archived;
        self.version = next_version;
        Ok(true)
    }

    fn terminate_membership(
        &mut self,
        principal_id: PrincipalId,
        terminal_status: PrivateRoomMembershipStatus,
        transition: &'static str,
    ) -> DomainResult<bool> {
        let member = self
            .members
            .get(&principal_id)
            .ok_or_else(|| invalid_member_transition("absent", transition))?;
        if member.status == terminal_status {
            return Ok(false);
        }
        if !member.status.carries_permissions() {
            return Err(invalid_member_transition(
                member.status.as_str(),
                transition,
            ));
        }
        let next_version = self.next_version()?;
        self.members
            .get_mut(&principal_id)
            .ok_or_else(|| invariant("private_room", "已验证的成员必须存在"))?
            .replace(terminal_status, PrivateRoomPermissions::NONE);
        self.version = next_version;
        Ok(true)
    }

    fn ensure_active(&self, transition: &'static str) -> DomainResult<()> {
        if self.status != PrivateRoomLifecycleStatus::Active {
            return Err(DomainError::InvalidTransition {
                entity: "private_room",
                from: self.status.as_str(),
                to: transition,
            });
        }
        Ok(())
    }

    fn require(
        &self,
        actor: PrincipalId,
        capability: PrivateRoomCapability,
        action: &'static str,
    ) -> DomainResult<()> {
        if !self.allows(actor, capability) {
            return Err(DomainError::Forbidden { action });
        }
        Ok(())
    }

    fn permissions_of(&self, principal_id: PrincipalId) -> PrivateRoomPermissions {
        self.members
            .get(&principal_id)
            .map_or(PrivateRoomPermissions::NONE, |member| member.permissions)
    }

    /// 房主以外的人只能给出自己有的权限（`specs/room-people/design.md`）。邀请和改权限都管：
    /// 不然没有自动发言的管理者先邀请、再改权限，照样能给出自动发言。房主什么权限都有。
    fn require_grantable(
        &self,
        actor: PrincipalId,
        previous: PrivateRoomPermissions,
        next: PrivateRoomPermissions,
    ) -> DomainResult<()> {
        if self
            .permissions_of(actor)
            .contains(next.added_since(previous))
        {
            Ok(())
        } else {
            Err(DomainError::Forbidden {
                action: "给出自己没有的私人房间权限",
            })
        }
    }

    fn require_non_owner_target(
        &self,
        target: PrincipalId,
        action: &'static str,
    ) -> DomainResult<()> {
        if target == self.owner_principal_id {
            return Err(DomainError::Forbidden { action });
        }
        Ok(())
    }

    /// 一进来就带“管理”的邀请和邀请链接：只有房主给得出。
    fn require_owner_to_appoint(
        &self,
        actor: PrincipalId,
        permissions: PrivateRoomPermissions,
        action: &'static str,
    ) -> DomainResult<()> {
        if permissions.manage() && actor != self.owner_principal_id {
            return Err(DomainError::OwnerOnly { action });
        }
        Ok(())
    }

    /// 改权限时：设、撤管理员只有房主能做，管理员身上的任何一项也只有房主能改。管理员自己不当了
    /// （把自己的“管理”拿掉）不用房主点头。
    fn require_owner_for_role_change(
        &self,
        actor: PrincipalId,
        target: PrincipalId,
        previous: PrivateRoomPermissions,
        next: PrivateRoomPermissions,
    ) -> DomainResult<()> {
        if actor == self.owner_principal_id {
            return Ok(());
        }
        if previous.manage() {
            if actor == target && !next.manage() {
                return Ok(());
            }
            return Err(DomainError::OwnerOnly {
                action: "修改私人房间管理员的权限",
            });
        }
        if next.manage() {
            return Err(DomainError::OwnerOnly {
                action: "设置私人房间管理员",
            });
        }
        Ok(())
    }

    const fn next_version(&self) -> DomainResult<AggregateVersion> {
        self.version.next()
    }
}

fn require_assignable_permissions(permissions: PrivateRoomPermissions) -> DomainResult<()> {
    if !permissions.can_be_assigned_to_member() {
        return Err(validation(
            "private_room_permissions",
            "活跃成员必须至少具备查看权限",
        ));
    }
    Ok(())
}

const fn validation(field: &'static str, reason: &'static str) -> DomainError {
    DomainError::Validation { field, reason }
}

const fn invariant(entity: &'static str, rule: &'static str) -> DomainError {
    DomainError::InvariantViolation { entity, rule }
}

const fn invalid_member_transition(from: &'static str, to: &'static str) -> DomainError {
    DomainError::InvalidTransition {
        entity: "private_room_member",
        from,
        to,
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomMember,
        PrivateRoomMembershipStatus, PrivateRoomPermissions,
    };
    use crate::{
        ids::{AgentId, PrincipalId, RoomCatalogId},
        version::AggregateVersion,
    };

    #[test]
    fn 新房间只有房主且房主具备全部权限() {
        let room = PrivateRoom::create(catalog_id(), principal(1));

        assert_eq!(room.owner_principal_id(), principal(1));
        assert_eq!(room.members().len(), 1);
        for capability in [
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
            PrivateRoomCapability::Automate,
        ] {
            assert!(room.allows(principal(1), capability));
        }
    }

    #[test]
    fn 权限依赖关系拒绝不可执行组合() {
        assert!(PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::Speak]).is_err());
        assert!(
            PrivateRoomPermissions::from_capabilities([
                PrivateRoomCapability::View,
                PrivateRoomCapability::Automate,
            ])
            .is_err()
        );
        assert!(
            PrivateRoomPermissions::from_capabilities([
                PrivateRoomCapability::View,
                PrivateRoomCapability::Speak,
                PrivateRoomCapability::Automate,
            ])
            .is_ok()
        );
    }

    #[test]
    fn 邀请接受和重复操作保持幂等() {
        let mut room = room();
        let permissions = ordinary_permissions();

        assert!(
            room.invite(principal(1), principal(2), permissions)
                .expect("房主可邀请")
        );
        assert!(
            !room
                .invite(principal(1), principal(2), permissions)
                .expect("重复邀请幂等")
        );
        assert!(!room.allows(principal(2), PrivateRoomCapability::View));
        assert!(room.accept_invitation(principal(2)).expect("受邀者可接受"));
        assert!(!room.accept_invitation(principal(2)).expect("重复接受幂等"));
        assert!(room.allows(principal(2), PrivateRoomCapability::View));
        assert_eq!(room.version().value(), 2);
    }

    #[test]
    fn 拒绝和移除会立即清空全部权限() {
        let mut room = room();
        room.invite(principal(1), principal(2), ordinary_permissions())
            .expect("邀请成功");
        room.decline_invitation(principal(2)).expect("拒绝成功");
        assert_eq!(
            room.member(principal(2))
                .expect("成员事实保留")
                .permissions(),
            PrivateRoomPermissions::NONE
        );

        room.invite(principal(1), principal(2), ordinary_permissions())
            .expect("拒绝后可重邀");
        room.accept_invitation(principal(2)).expect("接受成功");
        room.remove_member(principal(1), principal(2))
            .expect("房主可移除");
        assert!(!room.allows(principal(2), PrivateRoomCapability::View));
        assert_eq!(
            room.member(principal(2))
                .expect("成员事实保留")
                .permissions(),
            PrivateRoomPermissions::NONE
        );
    }

    #[test]
    fn 封禁是显式终态且不能通过邀请绕过() {
        let mut room = room();
        assert!(
            room.ban_member(principal(1), principal(2))
                .expect("可直接封禁主体")
        );
        assert!(
            !room
                .ban_member(principal(1), principal(2))
                .expect("重复封禁幂等")
        );
        assert!(
            room.invite(principal(1), principal(2), ordinary_permissions())
                .is_err()
        );
    }

    #[test]
    fn 解封以后回到已移出_能再邀请() {
        let mut room = room();
        join(&mut room, principal(2));
        room.ban_member(principal(1), principal(2))
            .expect("房主可封禁");

        assert!(
            room.unban_member(principal(2), principal(2)).is_err(),
            "没有管理权限不能解封"
        );
        assert!(
            room.unban_member(principal(1), principal(2))
                .expect("房主可解封")
        );
        let member = room.member(principal(2)).expect("成员事实保留");
        assert_eq!(member.status(), PrivateRoomMembershipStatus::Removed);
        assert_eq!(member.permissions(), PrivateRoomPermissions::NONE);
        assert!(
            !room
                .unban_member(principal(1), principal(2))
                .expect("没有封着的不改")
        );
        assert!(
            !room
                .unban_member(principal(1), principal(9))
                .expect("不认识的人不改")
        );
        assert!(
            room.invite(principal(1), principal(2), ordinary_permissions())
                .expect("解封后能再邀请")
        );
    }

    #[test]
    fn 邀请链接要已加入且有邀请权限_默认权限不超过自己的() {
        let mut room = room();
        assert_eq!(
            room.default_invitation_for(principal(1)),
            PrivateRoomPermissions::INVITED_BY_AGENT,
            "房主默认给查看、发言、自动发言"
        );
        let inviter = PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
        ])
        .expect("权限组合有效");
        room.invite(principal(1), principal(2), inviter)
            .expect("房主邀请");
        assert!(
            room.authorize_invite_link(principal(2)).is_err(),
            "还没接受邀请的不能管链接"
        );
        room.accept_invitation(principal(2)).expect("接受邀请成功");
        room.authorize_invite_link(principal(2))
            .expect("已加入、有邀请权限");
        assert_eq!(
            room.default_invitation_for(principal(2)),
            ordinary_permissions(),
            "没有自动发言的人默认只给查看和发言"
        );
        assert!(
            room.authorize_invite_link_permissions(
                principal(2),
                PrivateRoomPermissions::INVITED_BY_AGENT
            )
            .is_err()
        );
        join(&mut room, principal(3));
        assert!(
            room.authorize_invite_link(principal(3)).is_err(),
            "没有邀请权限"
        );
        room.archive(principal(1)).expect("房主可归档");
        assert!(
            room.authorize_invite_link(principal(1)).is_err(),
            "归档以后"
        );
    }

    #[test]
    fn 房主以外的人给不出自己没有的权限() {
        let mut room = room();
        let manager = PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ])
        .expect("权限组合有效");
        room.invite(principal(1), principal(2), manager)
            .expect("房主邀请管理者");
        room.accept_invitation(principal(2)).expect("接受邀请成功");

        let automate = PrivateRoomPermissions::INVITED_BY_AGENT;
        assert!(
            room.invite(principal(2), principal(3), automate).is_err(),
            "没有自动发言的人邀请时给不出自动发言"
        );
        assert!(
            room.invite(principal(2), principal(3), ordinary_permissions())
                .expect("自己有的可以给")
        );
        room.accept_invitation(principal(3)).expect("接受邀请成功");
        assert!(
            room.update_permissions(principal(2), principal(3), automate)
                .is_err(),
            "先邀请再改权限也给不出自动发言"
        );
        assert!(
            room.update_permissions(principal(1), principal(3), automate)
                .expect("房主什么都能给")
        );
        let view_only = PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View])
            .expect("只能查看有效");
        assert!(
            room.update_permissions(principal(2), principal(3), view_only)
                .expect("收回自己没有的也行")
        );
    }

    #[test]
    fn 只有房主能设撤管理员_管理员之间谁也动不了谁() {
        let mut room = room();
        let admin = admin_permissions();
        room.invite(principal(1), principal(2), admin)
            .expect("房主能请一位管理员进来");
        room.accept_invitation(principal(2)).expect("接受邀请成功");
        join(&mut room, principal(3));
        join(&mut room, principal(4));
        assert!(room.is_admin(principal(2)));
        assert!(!room.is_admin(principal(1)), "房主不算管理员");
        assert!(!room.is_admin(principal(3)));

        // 管理员设不了管理员：改权限、邀请、邀请链接三条路都不行。
        assert_eq!(
            room.update_permissions(principal(2), principal(3), admin),
            Err(owner_only("设置私人房间管理员"))
        );
        assert_eq!(
            room.invite(principal(2), principal(5), admin),
            Err(owner_only("邀请时设置私人房间管理员"))
        );
        assert_eq!(
            room.authorize_invite_link_permissions(principal(2), admin),
            Err(owner_only("邀请链接带上私人房间的管理权限"))
        );
        room.authorize_invite_link_permissions(principal(1), admin)
            .expect("房主的链接能带管理");

        // 房主再设一位。两位管理员互相改不了、移不出、封不了。
        assert!(
            room.update_permissions(principal(1), principal(3), admin)
                .expect("房主设管理员")
        );
        assert_eq!(
            room.update_permissions(principal(2), principal(3), ordinary_permissions()),
            Err(owner_only("修改私人房间管理员的权限"))
        );
        assert_eq!(
            room.remove_member(principal(2), principal(3)),
            Err(owner_only("移除私人房间管理员"))
        );
        assert_eq!(
            room.ban_member(principal(2), principal(3)),
            Err(owner_only("封禁私人房间管理员"))
        );
        assert_eq!(
            room.authorize_acting_on(principal(2), principal(3), "禁言私人房间管理员"),
            Err(owner_only("禁言私人房间管理员"))
        );
        assert_eq!(
            room.authorize_acting_on(principal(3), principal(3), "撤回私人房间管理员的禁言"),
            Err(owner_only("撤回私人房间管理员的禁言")),
            "房主给管理员的禁言，他自己解不了"
        );
        assert!(room.is_admin(principal(3)), "被拒以后什么都没变");

        // 管理员照旧管得了普通成员。
        room.authorize_acting_on(principal(2), principal(4), "禁言私人房间成员")
            .expect("普通成员管理员管得了");
        let view_only = PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View])
            .expect("只能查看有效");
        assert!(
            room.update_permissions(principal(2), principal(4), view_only)
                .expect("管理员能把成员设成只能看")
        );
        assert!(
            room.remove_member(principal(2), principal(4))
                .expect("管理员能移出成员")
        );

        // 房主什么都做得了。
        assert!(
            room.update_permissions(principal(1), principal(3), ordinary_permissions())
                .expect("房主撤管理员")
        );
        assert!(
            room.remove_member(principal(1), principal(2))
                .expect("房主移出管理员")
        );
    }

    #[test]
    fn 管理员自己可以不当了_别的改不了自己() {
        let mut room = room();
        room.invite(principal(1), principal(2), admin_permissions())
            .expect("房主能请一位管理员进来");
        room.accept_invitation(principal(2)).expect("接受邀请成功");

        let without_invite = PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Manage,
        ])
        .expect("权限组合有效");
        assert_eq!(
            room.update_permissions(principal(2), principal(2), without_invite),
            Err(owner_only("修改私人房间管理员的权限")),
            "还当着管理员时自己的别的权限也改不了"
        );
        assert!(
            room.update_permissions(
                principal(2),
                principal(2),
                PrivateRoomPermissions::INVITED_BY_AGENT
            )
            .is_err(),
            "不当了也不能顺手给自己加上原来没有的自动发言"
        );
        assert!(
            room.update_permissions(principal(2), principal(2), ordinary_permissions())
                .expect("自己把自己降成成员")
        );
        assert!(!room.is_admin(principal(2)));
        assert!(
            room.update_permissions(principal(2), principal(2), admin_permissions())
                .is_err(),
            "降了以后升不回来"
        );
    }

    #[test]
    fn 受邀还没进来的管理员也只有房主动得了() {
        let mut room = room();
        room.invite(principal(1), principal(2), admin_permissions())
            .expect("房主能请一位管理员进来");
        room.accept_invitation(principal(2)).expect("接受邀请成功");
        room.invite(principal(1), principal(3), admin_permissions())
            .expect("房主再请一位");

        assert!(room.is_admin(principal(3)), "受邀的也算");
        assert_eq!(
            room.remove_member(principal(2), principal(3)),
            Err(owner_only("移除私人房间管理员"))
        );
        assert!(
            room.remove_member(principal(1), principal(3))
                .expect("房主收回邀请")
        );
        assert!(!room.is_admin(principal(3)), "移出以后不再是管理员");
        room.authorize_acting_on(principal(2), principal(3), "封禁私人房间成员")
            .expect("已经移出的人管理员管得了");
    }

    #[test]
    fn 只有治理者可以修改权限但不能削弱房主() {
        let mut room = room();
        join(&mut room, principal(2));
        join(&mut room, principal(3));

        assert!(
            room.update_permissions(principal(2), principal(3), PrivateRoomPermissions::ALL,)
                .is_err()
        );
        assert!(
            room.update_permissions(principal(1), principal(1), ordinary_permissions(),)
                .is_err()
        );
        assert!(
            room.update_permissions(
                principal(1),
                principal(2),
                PrivateRoomPermissions::from_capabilities([
                    PrivateRoomCapability::View,
                    PrivateRoomCapability::Speak,
                    PrivateRoomCapability::Invite,
                ])
                .expect("权限组合有效"),
            )
            .expect("房主可委派邀请权限")
        );
        assert!(room.allows(principal(2), PrivateRoomCapability::Invite));
    }

    #[test]
    fn 房主转移同时更新新旧房主权限() {
        let mut room = room();
        join(&mut room, principal(2));

        assert!(
            room.transfer_ownership(principal(1), principal(2), ordinary_permissions())
                .expect("房主可转移给已加入成员")
        );
        assert_eq!(room.owner_principal_id(), principal(2));
        assert!(!room.allows(principal(1), PrivateRoomCapability::Manage));
        assert!(room.allows(principal(2), PrivateRoomCapability::Manage));
        assert!(room.leave(principal(2)).is_err());
        assert!(room.leave(principal(1)).expect("原房主现在可以离开"));
    }

    #[test]
    fn 归档后保留历史查看但关闭所有变更能力() {
        let mut room = room();
        join(&mut room, principal(2));

        assert!(room.archive(principal(1)).expect("房主可归档"));
        assert!(room.allows(principal(2), PrivateRoomCapability::View));
        assert!(!room.allows(principal(1), PrivateRoomCapability::Speak));
        assert!(
            room.invite(principal(1), principal(3), ordinary_permissions())
                .is_err()
        );
        assert!(!room.archive(principal(1)).expect("重复归档幂等"));
    }

    #[test]
    fn agent_建的房间记下是哪个_agent_房主照旧是主体() {
        let agent = AgentId::from_uuid(Uuid::from_u128(77));
        let room = PrivateRoom::create_by_agent(catalog_id(), principal(1), agent);
        assert_eq!(room.created_by_agent(), Some(agent));
        assert_eq!(room.owner_principal_id(), principal(1));
        assert!(room.admits_agent_of(principal(1)));
        assert_eq!(
            PrivateRoom::create(catalog_id(), principal(1)).created_by_agent(),
            None
        );

        let owner = room.member(principal(1)).expect("房主是成员").clone();
        let restored = PrivateRoom::restore(
            catalog_id(),
            principal(1),
            PrivateRoomLifecycleStatus::Active,
            [owner],
            AggregateVersion::INITIAL,
            Some(agent),
        )
        .expect("房间事实有效");
        assert_eq!(restored, room);
    }

    #[test]
    fn agent_只管_agent_建的房间() {
        let agent = AgentId::from_uuid(Uuid::from_u128(78));
        let mut built = PrivateRoom::create_by_agent(catalog_id(), principal(1), agent);
        assert!(built.authorize_agent_access_by_agent(principal(1)).is_ok());
        assert!(
            built.authorize_agent_access_by_agent(principal(2)).is_err(),
            "不是成员、没有管理权限"
        );
        assert!(
            built
                .invite_by_agent(principal(1), principal(2))
                .expect("能邀请")
        );
        assert_eq!(
            built.member(principal(2)).expect("受邀").permissions(),
            PrivateRoomPermissions::INVITED_BY_AGENT
        );
        assert!(built.archive(principal(1)).expect("房主可归档"));
        assert!(
            built.authorize_agent_access_by_agent(principal(1)).is_err(),
            "归档的房间不再管"
        );

        let mut joined = PrivateRoom::create_by_agent(catalog_id(), principal(1), agent);
        assert!(joined.invite_by_agent(principal(1), principal(2)).unwrap());
        assert!(joined.accept_invitation(principal(2)).unwrap());
        assert!(
            !joined
                .invite_by_agent(principal(1), principal(2))
                .expect("已经在房间里也算成功"),
            "什么都不改"
        );
        assert!(
            !joined
                .invite_by_agent(principal(1), principal(1))
                .expect("房主自己也在里面")
        );
        assert_eq!(
            joined.member(principal(2)).unwrap().status(),
            PrivateRoomMembershipStatus::Joined
        );
        assert!(
            joined.invite_by_agent(principal(2), principal(1)).is_err(),
            "受邀进来的人没有邀请权限，照旧拒绝"
        );
        assert!(
            joined.allows(principal(2), PrivateRoomCapability::Automate),
            "受邀进来的人有自动发言：他请进来的 Agent 也能说话"
        );
        assert!(!joined.allows(principal(2), PrivateRoomCapability::Manage));
        assert!(joined.archive(principal(1)).unwrap());
        assert!(
            joined.invite_by_agent(principal(1), principal(2)).is_err(),
            "关了的房间照旧拒绝"
        );

        let mut person_built = PrivateRoom::create(catalog_id(), principal(1));
        assert!(person_built.authorize_agent_access(principal(1)).is_ok());
        assert!(
            person_built
                .authorize_agent_access_by_agent(principal(1))
                .is_err(),
            "人建的房间 Agent 碰不到"
        );
        assert!(
            person_built
                .invite_by_agent(principal(1), principal(2))
                .is_err()
        );
        assert!(person_built.member(principal(2)).is_none());
    }

    #[test]
    fn 恢复时拒绝重复成员和损坏的房主事实() {
        let owner = PrivateRoomMember::restore(
            principal(1),
            PrivateRoomMembershipStatus::Joined,
            PrivateRoomPermissions::ALL,
        )
        .expect("房主事实有效");
        assert!(
            PrivateRoom::restore(
                catalog_id(),
                principal(1),
                PrivateRoomLifecycleStatus::Active,
                [owner.clone(), owner],
                AggregateVersion::INITIAL,
                None,
            )
            .is_err()
        );

        let weak_owner = PrivateRoomMember::restore(
            principal(1),
            PrivateRoomMembershipStatus::Joined,
            ordinary_permissions(),
        )
        .expect("普通成员事实有效");
        assert!(
            PrivateRoom::restore(
                catalog_id(),
                principal(1),
                PrivateRoomLifecycleStatus::Active,
                [weak_owner],
                AggregateVersion::INITIAL,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn 版本耗尽时拒绝变更且不留下半写入成员() {
        let owner = PrivateRoomMember::restore(
            principal(1),
            PrivateRoomMembershipStatus::Joined,
            PrivateRoomPermissions::ALL,
        )
        .expect("房主事实有效");
        let mut room = PrivateRoom::restore(
            catalog_id(),
            principal(1),
            PrivateRoomLifecycleStatus::Active,
            [owner],
            AggregateVersion::new(i64::MAX).expect("最大版本有效"),
            None,
        )
        .expect("房间事实有效");

        assert!(
            room.invite(principal(1), principal(2), ordinary_permissions())
                .is_err()
        );
        assert!(room.member(principal(2)).is_none());
        assert_eq!(room.version().value(), i64::MAX);
    }

    #[test]
    fn agent_随已加入且可发言的所有者入场() {
        let mut room = room();
        join(&mut room, principal(2));

        assert!(room.admits_agent_of(principal(1)), "房主可以带 Agent 进来");
        assert!(
            room.admits_agent_of(principal(2)),
            "已加入的发言成员可以带 Agent 进来"
        );
        assert!(
            !room.admits_agent_of(principal(9)),
            "非成员不能带 Agent 进来"
        );
    }

    #[test]
    fn 仅被邀请或已离场的所有者不能带_agent_进来() {
        let mut room = room();
        room.invite(principal(1), principal(2), ordinary_permissions())
            .expect("邀请成功");

        assert!(
            !room.admits_agent_of(principal(2)),
            "尚未接受邀请不算已加入"
        );

        room.accept_invitation(principal(2)).expect("接受邀请成功");
        assert!(room.admits_agent_of(principal(2)));

        room.leave(principal(2)).expect("离开成功");
        assert!(!room.admits_agent_of(principal(2)), "离开后立即失去房间");
    }

    #[test]
    fn 移除或封禁所有者会立即带走其_agent() {
        let mut room = room();
        join(&mut room, principal(2));
        join(&mut room, principal(3));

        room.remove_member(principal(1), principal(2))
            .expect("移除成功");
        assert!(!room.admits_agent_of(principal(2)));

        room.ban_member(principal(1), principal(3))
            .expect("封禁成功");
        assert!(!room.admits_agent_of(principal(3)));
    }

    #[test]
    fn 没有发言能力的成员不能带_agent_进来() {
        let mut room = room();
        let view_only = PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View])
            .expect("只读权限有效");
        room.invite(principal(1), principal(2), view_only)
            .expect("邀请成功");
        room.accept_invitation(principal(2)).expect("接受邀请成功");

        assert!(room.allows(principal(2), PrivateRoomCapability::View));
        assert!(
            !room.admits_agent_of(principal(2)),
            "只读成员不能带 Agent 发言"
        );
    }

    #[test]
    fn 自主发送额外要求所有者具备自动发送能力() {
        let mut room = room();
        join(&mut room, principal(2));

        assert!(room.admits_agent_of(principal(2)));
        assert!(
            !room.admits_autonomous_agent_of(principal(2)),
            "普通发言成员不足以让 Agent 自主发送"
        );
        assert!(
            room.admits_autonomous_agent_of(principal(1)),
            "房主具备自动发送能力"
        );
    }

    #[test]
    fn 归档房间不再接纳_agent() {
        let mut room = room();
        join(&mut room, principal(2));
        room.archive(principal(1)).expect("归档成功");

        assert!(
            room.allows(principal(2), PrivateRoomCapability::View),
            "仍可查看历史"
        );
        assert!(!room.admits_agent_of(principal(2)));
        assert!(!room.admits_autonomous_agent_of(principal(1)));
    }

    #[test]
    fn 凭口令进来的_agent_不随成员进出_归档后不再接纳() {
        let mut room = room();
        // 它的主人不是成员也能进；不是有效的 Agent 成员就不能。
        assert!(room.admits_agent_member(true));
        assert!(!room.admits_agent_member(false));
        assert!(!room.admits_agent_of(principal(9)));
        room.archive(principal(1)).expect("归档成功");
        assert!(!room.admits_agent_member(true));
    }

    #[test]
    fn 只有能管理的成员可以管理_agent_口令且房间须在使用中() {
        let mut room = room();
        join(&mut room, principal(2));
        assert!(room.authorize_agent_access(principal(1)).is_ok());
        assert!(matches!(
            room.authorize_agent_access(principal(2)),
            Err(crate::DomainError::Forbidden { .. })
        ));
        assert!(room.authorize_agent_access(principal(9)).is_err());
        room.update_permissions(principal(1), principal(2), PrivateRoomPermissions::ALL)
            .expect("房主可以授予管理权限");
        assert!(room.authorize_agent_access(principal(2)).is_ok());
        room.archive(principal(1)).expect("归档成功");
        assert!(matches!(
            room.authorize_agent_access(principal(1)),
            Err(crate::DomainError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn 房主和管理员能改公告_普通成员不能_归档后谁都不能() {
        let mut room = room();
        join(&mut room, principal(2));
        join(&mut room, principal(3));
        room.update_permissions(principal(1), principal(2), PrivateRoomPermissions::ALL)
            .expect("房主可以授予管理权限");
        assert!(room.authorize_notice_change(principal(1)).is_ok());
        assert!(room.authorize_notice_change(principal(2)).is_ok());
        assert!(matches!(
            room.authorize_notice_change(principal(3)),
            Err(crate::DomainError::Forbidden { .. })
        ));
        assert!(room.authorize_notice_change(principal(9)).is_err());
        room.archive(principal(1)).expect("归档成功");
        assert!(matches!(
            room.authorize_notice_change(principal(1)),
            Err(crate::DomainError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn 只有房主能改公开设置_管理员也不能_归档后谁都不能() {
        let mut room = room();
        join(&mut room, principal(2));
        room.update_permissions(principal(1), principal(2), PrivateRoomPermissions::ALL)
            .expect("房主可以把成员设成管理员");
        assert!(room.authorize_listing_change(principal(1)).is_ok());
        for actor in [principal(2), principal(9)] {
            assert!(matches!(
                room.authorize_listing_change(actor),
                Err(crate::DomainError::Forbidden { .. })
            ));
        }
        room.archive(principal(1)).expect("归档成功");
        assert!(matches!(
            room.authorize_listing_change(principal(1)),
            Err(crate::DomainError::InvalidTransition { .. })
        ));
    }

    fn room() -> PrivateRoom {
        PrivateRoom::create(catalog_id(), principal(1))
    }

    fn join(room: &mut PrivateRoom, principal_id: PrincipalId) {
        room.invite(principal(1), principal_id, ordinary_permissions())
            .expect("邀请成功");
        room.accept_invitation(principal_id).expect("接受邀请成功");
    }

    fn ordinary_permissions() -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
        ])
        .expect("普通权限有效")
    }

    fn admin_permissions() -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ])
        .expect("管理员权限有效")
    }

    const fn owner_only(action: &'static str) -> crate::DomainError {
        crate::DomainError::OwnerOnly { action }
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(100))
    }

    fn principal(sequence: u128) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(sequence))
    }
}
