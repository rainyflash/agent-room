use std::collections::BTreeMap;

use crate::{
    DomainError, DomainResult,
    ids::{PrincipalId, RoomCatalogId},
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
        })
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn owner_principal_id(&self) -> PrincipalId {
        self.owner_principal_id
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

    /// 邀请成员进入活跃房间。
    ///
    /// # Errors
    ///
    /// 操作者无邀请权限、房间已归档、目标已加入或被封禁时返回错误。
    pub fn invite(
        &mut self,
        actor: PrincipalId,
        target: PrincipalId,
        permissions: PrivateRoomPermissions,
    ) -> DomainResult<bool> {
        self.ensure_active("invited")?;
        self.require(actor, PrivateRoomCapability::Invite, "邀请私人房间成员")?;
        require_assignable_permissions(permissions)?;

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

    /// 移除已加入或待邀请成员。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、目标是房主或房间已归档时返回错误。
    pub fn remove_member(&mut self, actor: PrincipalId, target: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("removed")?;
        self.require(actor, PrivateRoomCapability::Manage, "移除私人房间成员")?;
        self.require_non_owner_target(target, "移除私人房间房主")?;
        self.terminate_membership(target, PrivateRoomMembershipStatus::Removed, "removed")
    }

    /// 封禁主体并立即撤销其全部产品权限。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、目标是房主或房间已归档时返回错误。
    pub fn ban_member(&mut self, actor: PrincipalId, target: PrincipalId) -> DomainResult<bool> {
        self.ensure_active("banned")?;
        self.require(actor, PrivateRoomCapability::Manage, "封禁私人房间成员")?;
        self.require_non_owner_target(target, "封禁私人房间房主")?;

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

    /// 更新已加入或待邀请成员的产品权限。
    ///
    /// # Errors
    ///
    /// 操作者无治理权限、目标是房主、目标不在活跃成员态或权限为空时返回错误。
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
        ids::{PrincipalId, RoomCatalogId},
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
        )
        .expect("房间事实有效");

        assert!(
            room.invite(principal(1), principal(2), ordinary_permissions())
                .is_err()
        );
        assert!(room.member(principal(2)).is_none());
        assert_eq!(room.version().value(), i64::MAX);
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

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(100))
    }

    fn principal(sequence: u128) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(sequence))
    }
}
