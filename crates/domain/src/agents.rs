use crate::{
    DomainError, DomainResult,
    ids::{AdapterBindingId, AgentId, AgentInstanceId, DeviceId, PrincipalId},
    time::{DurationMillis, UtcMillis},
    version::AggregateVersion,
};

const ED25519_PUBLIC_KEY_LENGTH: usize = 32;
const SUBJECT_HASH_LENGTH: usize = 32;
const MAX_ADAPTER_TYPE_LENGTH: usize = 64;
const MAX_CAPABILITY_VERSION_LENGTH: usize = 64;
const MAX_MATRIX_DEVICE_ID_LENGTH: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Active,
    Suspended,
    Retired,
}

impl AgentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
        }
    }
}

impl TryFrom<&str> for AgentStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "retired" => Ok(Self::Retired),
            _ => Err(DomainError::Validation {
                field: "agent_status",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentVisibility {
    Public,
    Unlisted,
    Private,
}

impl AgentVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Private => "private",
        }
    }
}

impl TryFrom<&str> for AgentVisibility {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "public" => Ok(Self::Public),
            "unlisted" => Ok(Self::Unlisted),
            "private" => Ok(Self::Private),
            _ => Err(DomainError::Validation {
                field: "agent_visibility",
                reason: "包含未知可见性",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    id: AgentId,
    status: AgentStatus,
    version: AggregateVersion,
}

impl Agent {
    pub const fn register(id: AgentId) -> Self {
        Self {
            id,
            status: AgentStatus::Active,
            version: AggregateVersion::INITIAL,
        }
    }

    pub const fn restore(id: AgentId, status: AgentStatus, version: AggregateVersion) -> Self {
        Self {
            id,
            status,
            version,
        }
    }

    pub const fn id(&self) -> AgentId {
        self.id
    }

    pub const fn status(&self) -> AgentStatus {
        self.status
    }

    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    pub const fn restore_version(&mut self, version: AggregateVersion) {
        self.version = version;
    }

    /// 激活已登记的 Agent，重复激活保持幂等。
    ///
    /// # Errors
    ///
    /// 已暂停或退役的 Agent 不能直接激活。
    pub fn activate(&mut self) -> DomainResult<()> {
        match self.status {
            AgentStatus::Active => Ok(()),
            AgentStatus::Suspended | AgentStatus::Retired => Err(DomainError::InvalidTransition {
                entity: "agent",
                from: self.status.as_str(),
                to: "active",
            }),
        }
    }

    /// 暂停 Agent，重复暂停保持幂等。
    ///
    /// # Errors
    ///
    /// 已退役 Agent 不能进入暂停状态。
    pub fn suspend(&mut self) -> DomainResult<()> {
        match self.status {
            AgentStatus::Active | AgentStatus::Suspended => {
                self.status = AgentStatus::Suspended;
                Ok(())
            }
            AgentStatus::Retired => Err(DomainError::InvalidTransition {
                entity: "agent",
                from: self.status.as_str(),
                to: "suspended",
            }),
        }
    }

    pub fn retire(&mut self) {
        self.status = AgentStatus::Retired;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Owner,
    Operator,
    Viewer,
}

impl AgentRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    pub const fn can_manage_members(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub const fn can_register_instance(self) -> bool {
        matches!(self, Self::Owner | Self::Operator)
    }
}

impl TryFrom<&str> for AgentRole {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "owner" => Ok(Self::Owner),
            "operator" => Ok(Self::Operator),
            "viewer" => Ok(Self::Viewer),
            _ => Err(DomainError::Validation {
                field: "agent_role",
                reason: "包含未知角色",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMember {
    agent_id: AgentId,
    principal_id: PrincipalId,
    role: AgentRole,
    granted_by: PrincipalId,
    granted_at: UtcMillis,
    revoked_at: Option<UtcMillis>,
}

impl AgentMember {
    /// 从持久化记录恢复 Agent 成员。
    ///
    /// # Errors
    ///
    /// 撤销时间早于授权时间时失败。
    pub fn restore(
        agent_id: AgentId,
        principal_id: PrincipalId,
        role: AgentRole,
        granted_by: PrincipalId,
        granted_at: UtcMillis,
        revoked_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        if revoked_at.is_some_and(|value| value < granted_at) {
            return Err(DomainError::Validation {
                field: "agent_member_revoked_at",
                reason: "不能早于授权时间",
            });
        }
        Ok(Self {
            agent_id,
            principal_id,
            role,
            granted_by,
            granted_at,
            revoked_at,
        })
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    pub const fn role(&self) -> AgentRole {
        self.role
    }

    pub const fn granted_by(&self) -> PrincipalId {
        self.granted_by
    }

    pub const fn granted_at(&self) -> UtcMillis {
        self.granted_at
    }

    pub const fn revoked_at(&self) -> Option<UtcMillis> {
        self.revoked_at
    }

    pub const fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMemberships {
    agent_id: AgentId,
    members: Vec<AgentMember>,
}

impl AgentMemberships {
    pub fn with_initial_owner(
        agent_id: AgentId,
        owner_id: PrincipalId,
        granted_at: UtcMillis,
    ) -> Self {
        Self {
            agent_id,
            members: vec![AgentMember {
                agent_id,
                principal_id: owner_id,
                role: AgentRole::Owner,
                granted_by: owner_id,
                granted_at,
                revoked_at: None,
            }],
        }
    }

    /// 从持久化记录恢复成员聚合。
    ///
    /// # Errors
    ///
    /// 成员属于其他 Agent、时间逆序或缺少活跃 Owner 时返回不变式错误。
    pub fn restore(agent_id: AgentId, members: Vec<AgentMember>) -> DomainResult<Self> {
        if members.iter().any(|member| {
            member.agent_id != agent_id
                || member
                    .revoked_at
                    .is_some_and(|revoked_at| revoked_at < member.granted_at)
        }) {
            return Err(DomainError::InvariantViolation {
                entity: "agent_memberships",
                rule: "成员归属和时间顺序必须一致",
            });
        }
        let memberships = Self { agent_id, members };
        memberships.ensure_has_owner()?;
        Ok(memberships)
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub fn members(&self) -> &[AgentMember] {
        &self.members
    }

    pub fn role_of(&self, principal_id: PrincipalId) -> Option<AgentRole> {
        self.members
            .iter()
            .find(|member| member.principal_id == principal_id && member.is_active())
            .map(AgentMember::role)
    }

    /// 由 Owner 授予或更新成员角色。
    ///
    /// # Errors
    ///
    /// 操作人不是 Owner，或尝试降级最后一个 Owner 时失败。
    pub fn grant_role(
        &mut self,
        actor_id: PrincipalId,
        principal_id: PrincipalId,
        role: AgentRole,
        granted_at: UtcMillis,
    ) -> DomainResult<()> {
        self.ensure_member_manager(actor_id)?;
        let active_owner_count = self.active_owner_count();
        if let Some(member) = self
            .members
            .iter_mut()
            .find(|member| member.principal_id == principal_id)
        {
            if member.is_active()
                && member.role == AgentRole::Owner
                && role != AgentRole::Owner
                && active_owner_count == 1
            {
                return Err(last_owner_error());
            }
            if member.is_active() && member.role == role {
                return Ok(());
            }
            member.role = role;
            member.granted_by = actor_id;
            member.granted_at = granted_at;
            member.revoked_at = None;
            return Ok(());
        }

        self.members.push(AgentMember {
            agent_id: self.agent_id,
            principal_id,
            role,
            granted_by: actor_id,
            granted_at,
            revoked_at: None,
        });
        Ok(())
    }

    /// 由 Owner 撤销成员访问权。
    ///
    /// # Errors
    ///
    /// 操作人不是 Owner、撤销时间早于授权时间，或目标是最后一个 Owner 时失败。
    pub fn revoke(
        &mut self,
        actor_id: PrincipalId,
        principal_id: PrincipalId,
        revoked_at: UtcMillis,
    ) -> DomainResult<()> {
        self.ensure_member_manager(actor_id)?;
        let active_owner_count = self.active_owner_count();
        let member = self
            .members
            .iter_mut()
            .find(|member| member.principal_id == principal_id)
            .ok_or(DomainError::Forbidden {
                action: "revoke_unknown_agent_member",
            })?;
        if !member.is_active() {
            return Ok(());
        }
        if revoked_at < member.granted_at {
            return Err(DomainError::Validation {
                field: "agent_member_revoked_at",
                reason: "不能早于授权时间",
            });
        }
        if member.role == AgentRole::Owner && active_owner_count == 1 {
            return Err(last_owner_error());
        }
        member.revoked_at = Some(revoked_at);
        Ok(())
    }

    /// 校验成员是否可以代表 Agent 注册运行实例。
    ///
    /// # Errors
    ///
    /// 主体不是活跃 Owner 或 Operator 时返回无权操作错误。
    pub fn ensure_can_register_instance(&self, principal_id: PrincipalId) -> DomainResult<()> {
        if self
            .role_of(principal_id)
            .is_some_and(AgentRole::can_register_instance)
        {
            Ok(())
        } else {
            Err(DomainError::Forbidden {
                action: "register_agent_instance",
            })
        }
    }

    fn ensure_member_manager(&self, principal_id: PrincipalId) -> DomainResult<()> {
        if self
            .role_of(principal_id)
            .is_some_and(AgentRole::can_manage_members)
        {
            Ok(())
        } else {
            Err(DomainError::Forbidden {
                action: "manage_agent_members",
            })
        }
    }

    fn ensure_has_owner(&self) -> DomainResult<()> {
        if self.active_owner_count() == 0 {
            Err(last_owner_error())
        } else {
            Ok(())
        }
    }

    fn active_owner_count(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.is_active() && member.role == AgentRole::Owner)
            .count()
    }
}

fn last_owner_error() -> DomainError {
    DomainError::InvariantViolation {
        entity: "agent_memberships",
        rule: "活跃 Agent 必须保留至少一个 Owner",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterBindingState {
    Active,
    Disabled,
    Incompatible,
}

impl AdapterBindingState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Incompatible => "incompatible",
        }
    }
}

impl TryFrom<&str> for AdapterBindingState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "incompatible" => Ok(Self::Incompatible),
            _ => Err(DomainError::Validation {
                field: "adapter_binding_state",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSubjectHash([u8; SUBJECT_HASH_LENGTH]);

impl AdapterSubjectHash {
    /// 从不可逆主体摘要创建绑定标识。
    ///
    /// # Errors
    ///
    /// 摘要不是 32 字节时返回校验错误。
    pub fn new(bytes: Vec<u8>) -> DomainResult<Self> {
        let bytes =
            <[u8; SUBJECT_HASH_LENGTH]>::try_from(bytes).map_err(|_| DomainError::Validation {
                field: "adapter_subject_hash",
                reason: "必须是 32 字节摘要",
            })?;
        Ok(Self(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; SUBJECT_HASH_LENGTH] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterBinding {
    id: AdapterBindingId,
    agent_id: AgentId,
    adapter_type: String,
    external_subject_hash: Option<AdapterSubjectHash>,
    capability_version: String,
    state: AdapterBindingState,
}

impl AdapterBinding {
    /// 创建 Agent 的适配器绑定。
    ///
    /// # Errors
    ///
    /// 适配器类型或能力版本为空、超长或包含控制字符时失败。
    pub fn register(
        id: AdapterBindingId,
        agent_id: AgentId,
        adapter_type: String,
        external_subject_hash: Option<AdapterSubjectHash>,
        capability_version: String,
    ) -> DomainResult<Self> {
        validate_text("adapter_type", &adapter_type, MAX_ADAPTER_TYPE_LENGTH)?;
        validate_text(
            "adapter_capability_version",
            &capability_version,
            MAX_CAPABILITY_VERSION_LENGTH,
        )?;
        Ok(Self {
            id,
            agent_id,
            adapter_type,
            external_subject_hash,
            capability_version,
            state: AdapterBindingState::Active,
        })
    }

    /// 从持久化记录恢复适配器绑定。
    ///
    /// # Errors
    ///
    /// 适配器类型或能力版本无效时失败。
    pub fn restore(
        id: AdapterBindingId,
        agent_id: AgentId,
        adapter_type: String,
        external_subject_hash: Option<AdapterSubjectHash>,
        capability_version: String,
        state: AdapterBindingState,
    ) -> DomainResult<Self> {
        let mut binding = Self::register(
            id,
            agent_id,
            adapter_type,
            external_subject_hash,
            capability_version,
        )?;
        binding.state = state;
        Ok(binding)
    }

    pub const fn id(&self) -> AdapterBindingId {
        self.id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub fn adapter_type(&self) -> &str {
        &self.adapter_type
    }

    pub const fn external_subject_hash(&self) -> Option<&AdapterSubjectHash> {
        self.external_subject_hash.as_ref()
    }

    pub fn capability_version(&self) -> &str {
        &self.capability_version
    }

    pub const fn state(&self) -> AdapterBindingState {
        self.state
    }

    pub fn disable(&mut self) {
        self.state = AdapterBindingState::Disabled;
    }

    pub fn mark_incompatible(&mut self) {
        self.state = AdapterBindingState::Incompatible;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstancePublicSigningKey([u8; ED25519_PUBLIC_KEY_LENGTH]);

impl AgentInstancePublicSigningKey {
    /// 从 Ed25519 公钥字节创建 Agent 实例签名键。
    ///
    /// # Errors
    ///
    /// 字节长度不是 32 时返回校验错误。
    pub fn new(bytes: Vec<u8>) -> DomainResult<Self> {
        let bytes = <[u8; ED25519_PUBLIC_KEY_LENGTH]>::try_from(bytes).map_err(|_| {
            DomainError::Validation {
                field: "agent_instance_public_signing_key",
                reason: "必须是 32 字节 Ed25519 公钥",
            }
        })?;
        Ok(Self(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; ED25519_PUBLIC_KEY_LENGTH] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMatrixDeviceId(String);

impl AgentMatrixDeviceId {
    /// 创建 Matrix Device 标识。
    ///
    /// # Errors
    ///
    /// 标识为空、超长或包含控制字符时失败。
    pub fn new(value: String) -> DomainResult<Self> {
        validate_text(
            "agent_matrix_device_id",
            &value,
            MAX_MATRIX_DEVICE_ID_LENGTH,
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceStatus {
    Connecting,
    Online,
    Degraded,
    Offline,
    Revoked,
}

impl AgentInstanceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Online => "online",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
            Self::Revoked => "revoked",
        }
    }
}

impl TryFrom<&str> for AgentInstanceStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "connecting" => Ok(Self::Connecting),
            "online" => Ok(Self::Online),
            "degraded" => Ok(Self::Degraded),
            "offline" => Ok(Self::Offline),
            "revoked" => Ok(Self::Revoked),
            _ => Err(DomainError::Validation {
                field: "agent_instance_status",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    id: AgentInstanceId,
    agent_id: AgentId,
    device_id: DeviceId,
    adapter_binding_id: AdapterBindingId,
    public_signing_key: AgentInstancePublicSigningKey,
    matrix_device_id: AgentMatrixDeviceId,
    status: AgentInstanceStatus,
    lease_expires_at: Option<UtcMillis>,
}

impl AgentInstance {
    pub const fn register(
        id: AgentInstanceId,
        agent_id: AgentId,
        device_id: DeviceId,
        adapter_binding_id: AdapterBindingId,
        public_signing_key: AgentInstancePublicSigningKey,
        matrix_device_id: AgentMatrixDeviceId,
    ) -> Self {
        Self {
            id,
            agent_id,
            device_id,
            adapter_binding_id,
            public_signing_key,
            matrix_device_id,
            status: AgentInstanceStatus::Connecting,
            lease_expires_at: None,
        }
    }

    /// 从持久化记录恢复 Agent 实例。
    ///
    /// # Errors
    ///
    /// 在线状态和租约字段不一致时失败。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: AgentInstanceId,
        agent_id: AgentId,
        device_id: DeviceId,
        adapter_binding_id: AdapterBindingId,
        public_signing_key: AgentInstancePublicSigningKey,
        matrix_device_id: AgentMatrixDeviceId,
        status: AgentInstanceStatus,
        lease_expires_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        if matches!(status, AgentInstanceStatus::Online) != lease_expires_at.is_some() {
            return Err(DomainError::InvariantViolation {
                entity: "agent_instance",
                rule: "只有在线实例可以持有租约",
            });
        }
        Ok(Self {
            id,
            agent_id,
            device_id,
            adapter_binding_id,
            public_signing_key,
            matrix_device_id,
            status,
            lease_expires_at,
        })
    }

    pub const fn id(&self) -> AgentInstanceId {
        self.id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub const fn adapter_binding_id(&self) -> AdapterBindingId {
        self.adapter_binding_id
    }

    pub const fn public_signing_key(&self) -> &AgentInstancePublicSigningKey {
        &self.public_signing_key
    }

    pub const fn matrix_device_id(&self) -> &AgentMatrixDeviceId {
        &self.matrix_device_id
    }

    pub const fn status(&self) -> AgentInstanceStatus {
        self.status
    }

    pub const fn lease_expires_at(&self) -> Option<UtcMillis> {
        self.lease_expires_at
    }

    /// 连接实例并创建新的在线租约。
    ///
    /// # Errors
    ///
    /// 实例已撤销或租约截止时间溢出时返回错误。
    pub fn connect(&mut self, now: UtcMillis, lease: DurationMillis) -> DomainResult<()> {
        if self.status == AgentInstanceStatus::Revoked {
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: self.status.as_str(),
                to: "online",
            });
        }

        self.status = AgentInstanceStatus::Online;
        self.lease_expires_at = Some(now.checked_add(lease)?);
        Ok(())
    }

    /// 续租当前在线实例。
    ///
    /// # Errors
    ///
    /// 实例不在线、原租约已经过期或新截止时间溢出时返回错误。
    pub fn renew(&mut self, now: UtcMillis, lease: DurationMillis) -> DomainResult<()> {
        if self.status != AgentInstanceStatus::Online {
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: self.status.as_str(),
                to: "online",
            });
        }

        if self
            .lease_expires_at
            .is_some_and(|expires_at| expires_at < now)
        {
            self.status = AgentInstanceStatus::Offline;
            self.lease_expires_at = None;
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: "expired",
                to: "online",
            });
        }

        self.lease_expires_at = Some(now.checked_add(lease)?);
        Ok(())
    }

    /// 将实例标记为降级状态。
    ///
    /// # Errors
    ///
    /// 已撤销实例不能重新进入运行状态。
    pub fn degrade(&mut self) -> DomainResult<()> {
        if self.status == AgentInstanceStatus::Revoked {
            return Err(DomainError::InvalidTransition {
                entity: "agent_instance",
                from: self.status.as_str(),
                to: "degraded",
            });
        }
        self.status = AgentInstanceStatus::Degraded;
        self.lease_expires_at = None;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if self.status != AgentInstanceStatus::Revoked {
            self.status = AgentInstanceStatus::Offline;
            self.lease_expires_at = None;
        }
    }

    pub fn expire(&mut self, now: UtcMillis) -> bool {
        let should_expire = self.status == AgentInstanceStatus::Online
            && self
                .lease_expires_at
                .is_some_and(|expires_at| expires_at <= now);

        if should_expire {
            self.disconnect();
        }

        should_expire
    }

    pub fn revoke(&mut self) {
        self.status = AgentInstanceStatus::Revoked;
        self.lease_expires_at = None;
    }
}

fn validate_text(field: &'static str, value: &str, maximum_length: usize) -> DomainResult<()> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(DomainError::Validation {
            field,
            reason: "不能为空、超长或包含控制字符",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        AdapterBinding, AdapterSubjectHash, AgentInstance, AgentInstancePublicSigningKey,
        AgentInstanceStatus, AgentMatrixDeviceId, AgentMemberships, AgentRole,
    };
    use crate::{
        ids::{AdapterBindingId, AgentId, AgentInstanceId, DeviceId, PrincipalId},
        time::{DurationMillis, UtcMillis},
    };

    #[test]
    fn 最后一个_owner_无法被降级或撤销() {
        let owner = principal(1);
        let mut memberships = AgentMemberships::with_initial_owner(agent_id(), owner, time(1_000));

        assert!(
            memberships
                .grant_role(owner, owner, AgentRole::Operator, time(2_000))
                .is_err()
        );
        assert!(memberships.revoke(owner, owner, time(2_000)).is_err());
        assert_eq!(memberships.role_of(owner), Some(AgentRole::Owner));
    }

    #[test]
    fn 第二个_owner_建立后可安全转移控制权() {
        let first_owner = principal(1);
        let second_owner = principal(2);
        let mut memberships =
            AgentMemberships::with_initial_owner(agent_id(), first_owner, time(1_000));
        memberships
            .grant_role(first_owner, second_owner, AgentRole::Owner, time(2_000))
            .expect("可添加第二个 Owner");
        memberships
            .revoke(first_owner, first_owner, time(3_000))
            .expect("可撤销非最后 Owner");

        assert_eq!(memberships.role_of(first_owner), None);
        assert_eq!(memberships.role_of(second_owner), Some(AgentRole::Owner));
    }

    #[test]
    fn operator_可绑定实例而_viewer_不可以() {
        let owner = principal(1);
        let operator = principal(2);
        let viewer = principal(3);
        let mut memberships = AgentMemberships::with_initial_owner(agent_id(), owner, time(1_000));
        memberships
            .grant_role(owner, operator, AgentRole::Operator, time(2_000))
            .expect("可授予 Operator");
        memberships
            .grant_role(owner, viewer, AgentRole::Viewer, time(2_000))
            .expect("可授予 Viewer");

        assert!(memberships.ensure_can_register_instance(operator).is_ok());
        assert!(memberships.ensure_can_register_instance(viewer).is_err());
    }

    #[test]
    fn 适配器和实例公钥都拒绝宽松长度() {
        assert!(AdapterSubjectHash::new(vec![7; 31]).is_err());
        assert!(AdapterSubjectHash::new(vec![7; 32]).is_ok());
        assert!(AgentInstancePublicSigningKey::new(vec![7; 31]).is_err());
        assert!(AgentInstancePublicSigningKey::new(vec![7; 32]).is_ok());
        assert!(AgentInstancePublicSigningKey::new(vec![7; 33]).is_err());
        assert!(
            AdapterBinding::register(
                AdapterBindingId::from_uuid(Uuid::from_u128(4)),
                agent_id(),
                String::new(),
                None,
                "1".to_owned(),
            )
            .is_err()
        );
    }

    #[test]
    fn 租约到期后必须重新连接而不是续租() {
        let mut instance = instance();
        let now = time(1_000);
        let lease = DurationMillis::new(100).expect("测试租约有效");
        instance.connect(now, lease).expect("连接应成功");

        let late = time(1_101);
        assert!(instance.renew(late, lease).is_err());
        assert_eq!(instance.status(), AgentInstanceStatus::Offline);
    }

    #[test]
    fn 撤销实例是不可逆操作() {
        let mut instance = instance();
        instance.revoke();
        instance.revoke();

        let lease = DurationMillis::new(100).expect("测试租约有效");
        assert!(instance.connect(time(1_000), lease).is_err());
        assert!(instance.degrade().is_err());
    }

    fn instance() -> AgentInstance {
        AgentInstance::register(
            AgentInstanceId::from_uuid(Uuid::from_u128(1)),
            agent_id(),
            DeviceId::from_uuid(Uuid::from_u128(3)),
            AdapterBindingId::from_uuid(Uuid::from_u128(4)),
            AgentInstancePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
            AgentMatrixDeviceId::new("AGENT-INSTANCE-1".to_owned())
                .expect("测试 Matrix Device 有效"),
        )
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(Uuid::from_u128(2))
    }

    fn principal(value: u128) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(value))
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
