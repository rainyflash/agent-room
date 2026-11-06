use agent_room_domain::{
    DomainError, DomainResult,
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus},
    room_notices::RoomNoticeText,
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomInstance, RoomInstanceFields, RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixCreateRoom, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomEncryption,
        MatrixRoomId, MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId,
        PortFuture,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateMatrixMembership {
    Invited,
    Joined,
    Left,
    Banned,
    Knocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateMatrixSpeakingAssignment {
    user_id: MatrixUserId,
    allowed: bool,
}

impl PrivateMatrixSpeakingAssignment {
    pub const fn new(user_id: MatrixUserId, allowed: bool) -> Self {
        Self { user_id, allowed }
    }

    pub const fn user_id(&self) -> &MatrixUserId {
        &self.user_id
    }

    pub const fn allowed(&self) -> bool {
        self.allowed
    }
}

impl PrivateMatrixMembership {
    pub const fn is_joined(self) -> bool {
        matches!(self, Self::Joined)
    }
}

/// 从受信任账户投影解析 Matrix 用户，拒绝接受客户端自报的 Matrix 身份。
pub trait PrivateRoomPrincipalDirectory: Send + Sync {
    fn matrix_user_id(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>>;
}

/// 建房的 Agent：名字，以及它是网络 Agent 时替它记账的合成主体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomAgentDescription {
    pub display_name: String,
    pub network_principal_id: Option<PrincipalId>,
}

/// 一个主体名下（他是主人或操作人、还没撤的）的 Agent。`network` 为真时这个主体是替一个网络 Agent
/// 记账的合成主体，名下只有这个网络 Agent。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrincipalAgents {
    pub agent_ids: Vec<AgentId>,
    pub network: bool,
}

/// 列出某个主体名下全部 Agent 的 Matrix 用户。
///
/// Agent 不是私人房间成员，它随主体入场，在 Matrix 上的能力不能超过该主体。主体的发言被收回、
/// 被移除、被封禁或自行离开时，服务靠它找到要一起收紧的 Agent。
pub trait PrivateRoomAgentDirectory: Send + Sync {
    fn agent_matrix_users(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<MatrixUserId>>>;

    /// 这个 Agent 是不是该主体名下、还在用的 Agent。Agent 替主体建房时，用它核对建房的
    /// Agent 真是这个主体的。
    fn owns_active_agent(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>>;

    /// 建房的 Agent 叫什么、是不是网络 Agent：房间设置里写由谁建，网络 Agent 当房主的房间要提醒
    /// 服务器读得到。停用了的也照样回答；找不到这个 Agent 时为空。
    fn describe_agent(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentDescription>>>;

    /// 这个主体名下的 Agent，和它是不是网络 Agent 的合成主体：治理踢出、封禁他时连他凭口令进来的
    /// Agent 一起移出，网络 Agent 只移出它自己。
    fn principal_agents(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<PrincipalAgents>>;
}

/// 私人房间在 Matrix 上的最小硬边界能力。
///
/// 邀请、管理和自动发送仍由产品权限表裁决；实现不得把这些能力粗暴映射为 Matrix 管理员。
pub trait PrivateRoomMatrixGateway: Send + Sync {
    fn membership<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>>;

    fn invite<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn kick<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn ban<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;

    /// 解封。只解真封着的人：Synapse 不让解没封的人（`M_BAD_STATE`），重试时已经解开了也算成功。
    fn unban<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn set_speaking<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn set_speaking_batch<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn archive<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>>;

    /// 设置 Matrix 房间名（`m.room.name`），客户端里显示的房间名跟着变。重复设置同名幂等。
    fn set_name<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        name: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateMatrixRoomCreation {
    request: MatrixCreateRoom,
    alias: MatrixRoomAliasLocalpart,
}

impl PrivateMatrixRoomCreation {
    /// 固化邀请制、不可公开列出且由服务端治理的 Matrix 建房请求。
    ///
    /// # Errors
    ///
    /// 请求不满足私人房间硬边界时返回错误。
    pub fn new(request: MatrixCreateRoom, alias: MatrixRoomAliasLocalpart) -> DomainResult<Self> {
        if request.encryption() != MatrixRoomEncryption::EndToEnd {
            return Err(invariant(
                "private_matrix_room_creation",
                "私人 Matrix 房间必须端到端加密",
            ));
        }
        Self::managed(request, alias)
    }

    /// 新建时就公开的房间（`specs/public-rooms/design.md`）：不加密，别的和私人房间一样，在 Matrix
    /// 上照旧隐藏、邀请制，进门都经控制面。
    ///
    /// # Errors
    ///
    /// 请求加了密，或不满足私人房间的其他硬边界时返回错误。
    pub fn new_public(
        request: MatrixCreateRoom,
        alias: MatrixRoomAliasLocalpart,
    ) -> DomainResult<Self> {
        if request.encryption() != MatrixRoomEncryption::Unencrypted {
            return Err(invariant(
                "private_matrix_room_creation",
                "新建时就公开的房间不加密",
            ));
        }
        Self::managed(request, alias)
    }

    fn managed(request: MatrixCreateRoom, alias: MatrixRoomAliasLocalpart) -> DomainResult<Self> {
        let valid = request.visibility() == MatrixRoomVisibility::Private
            && request.preset() == MatrixRoomPreset::PrivateChat
            && request.power_profile() == MatrixRoomPowerProfile::ManagedPrivate
            && request.alias_localpart() == Some(&alias);
        if !valid {
            return Err(invariant(
                "private_matrix_room_creation",
                "私人 Matrix 房间必须隐藏、邀请制、受管且带稳定别名",
            ));
        }
        Ok(Self { request, alias })
    }

    pub const fn request(&self) -> &MatrixCreateRoom {
        &self.request
    }

    pub const fn alias(&self) -> &MatrixRoomAliasLocalpart {
        &self.alias
    }
}

/// 创建私人 Matrix Room，并在未知提交或别名冲突时按稳定别名对账。
pub trait PrivateRoomMatrixProvisioner: Send + Sync {
    fn create<'a>(
        &'a self,
        creation: &'a PrivateMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomSnapshot {
    catalog: RoomCatalog,
    instance: RoomInstance,
    room: PrivateRoom,
}

impl PrivateRoomSnapshot {
    /// 组合私人房间目录、Matrix 实例与权限聚合的同一事实快照。
    ///
    /// # Errors
    ///
    /// 三个对象不属于同一房间，或生命周期状态互相矛盾时返回错误。
    pub fn new(
        catalog: RoomCatalog,
        instance: RoomInstance,
        room: PrivateRoom,
    ) -> DomainResult<Self> {
        if catalog.kind() != RoomCatalogKind::PrivateRoom
            || catalog.id() != room.catalog_id()
            || instance.catalog_id() != room.catalog_id()
        {
            return Err(invariant(
                "private_room_snapshot",
                "组成对象必须属于同一私人房间",
            ));
        }
        if catalog.owner_principal_id() != Some(room.owner_principal_id()) {
            return Err(invariant(
                "private_room_snapshot",
                "目录房主必须与权限聚合房主一致",
            ));
        }
        let states_match = matches!(
            (catalog.status(), instance.state(), room.status()),
            (
                RoomCatalogStatus::Active,
                RoomInstanceState::Active,
                PrivateRoomLifecycleStatus::Active
            ) | (
                RoomCatalogStatus::Archived,
                RoomInstanceState::Archived,
                PrivateRoomLifecycleStatus::Archived
            )
        );
        if !states_match {
            return Err(invariant(
                "private_room_snapshot",
                "目录、实例与权限聚合的生命周期必须一致",
            ));
        }
        Ok(Self {
            catalog,
            instance,
            room,
        })
    }

    pub const fn catalog(&self) -> &RoomCatalog {
        &self.catalog
    }

    pub const fn instance(&self) -> &RoomInstance {
        &self.instance
    }

    pub const fn room(&self) -> &PrivateRoom {
        &self.room
    }

    /// 换上新名字的同一个房间。
    ///
    /// # Errors
    ///
    /// 新名字不满足房间名规则时返回错误。
    pub fn renaming(self, name: String) -> DomainResult<Self> {
        let catalog = self.catalog.renamed(name)?;
        Self::new(catalog, self.instance, self.room)
    }

    /// 换上新简介的同一个房间。
    ///
    /// # Errors
    ///
    /// 新简介不满足房间简介规则时返回错误。
    pub fn describing(self, description: String) -> DomainResult<Self> {
        let catalog = self.catalog.described(description)?;
        Self::new(catalog, self.instance, self.room)
    }

    /// 用新的权限聚合重建生命周期一致的组合快照。
    ///
    /// 房主与归档状态只有一个事实源：`PrivateRoom`。目录和实例只是同一事务中的持久化投影。
    ///
    /// # Errors
    ///
    /// 新聚合属于其他目录，或重建后的领域对象无效时返回错误。
    pub fn replacing_room(self, room: PrivateRoom) -> DomainResult<Self> {
        if room.catalog_id() != self.catalog.id() {
            return Err(invariant(
                "private_room_snapshot",
                "替换聚合必须属于原私人房间",
            ));
        }
        let archived = room.status() == PrivateRoomLifecycleStatus::Archived;
        let catalog = RoomCatalog::new(
            self.catalog.id(),
            RoomCatalogFields {
                kind: self.catalog.kind(),
                slug: self.catalog.slug().cloned(),
                name: self.catalog.name().to_owned(),
                description: self.catalog.description().to_owned(),
                language: self.catalog.language().cloned(),
                matrix_space_id: self.catalog.matrix_space_id().cloned(),
                owner_principal_id: Some(room.owner_principal_id()),
                visibility: self.catalog.visibility(),
                retention_days: self.catalog.retention_days(),
                status: if archived {
                    RoomCatalogStatus::Archived
                } else {
                    RoomCatalogStatus::Active
                },
            },
        )?;
        let instance = RoomInstance::restore(
            self.instance.id(),
            RoomInstanceFields {
                catalog_id: self.instance.catalog_id(),
                matrix_room_id: self.instance.matrix_room_id().clone(),
                region: self.instance.region().cloned(),
                capacity: self.instance.capacity(),
                allocated_slots: self.instance.allocated_slots(),
                state: if archived {
                    RoomInstanceState::Archived
                } else {
                    RoomInstanceState::Active
                },
            },
        )?;
        Self::new(catalog, instance, room)
    }
}

/// 持久化私人房间的权威产品权限事实。
///
/// Matrix 成员投影不能实现此端口；它只代表外部协议观测，不是产品授权事实源。
pub trait PrivateRoomStore: Send + Sync {
    fn create<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>>;

    /// 列出主体当前受邀或已加入的房间；被移除、拒绝、离开和封禁事实不得泄露。
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>>;

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>>;

    /// 以乐观版本锁原子保存成员、房主和归档状态。
    fn save<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected_version: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 改房间目录上的名字；与成员版本无关，不动成员与权限。
    fn rename<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        name: &'a str,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

/// 房主改房间简介（`specs/public-rooms/design.md`）：简介也是 Matrix 房间的话题。单独两个端口，
/// 不往 [`PrivateRoomStore`] 和 [`PrivateRoomMatrixGateway`] 里加：用到它们的地方太多。
pub trait PrivateRoomDescriptionStore: Send + Sync {
    /// 改房间目录上的简介；与成员版本无关，不动成员与权限。
    fn describe<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        description: &'a str,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

pub trait PrivateRoomTopicGateway: Send + Sync {
    /// 设置 Matrix 房间的话题（`m.room.topic`），空的就是去掉。重复设置同一段幂等。
    fn set_topic<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        topic: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

/// 写进 Matrix 的一版常驻公告（`room.notice.v1`）：正文（空的就是撤掉）、改的人、改的时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomNoticeWrite {
    pub text: RoomNoticeText,
    /// 改的人在 Matrix 上的 ID。状态事件是应用服务发的，读的一边按它说“某某更新了公告”。
    pub updated_by: MatrixUserId,
    pub updated_at: UtcMillis,
}

pub trait PrivateRoomNoticeGateway: Send + Sync {
    /// 以应用服务写 `io.github.rainyflash.agentroom.room.notice.v1`（`state_key` 为空），盖掉上一版。
    fn set_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        notice: &'a RoomNoticeWrite,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

const fn invariant(entity: &'static str, rule: &'static str) -> DomainError {
    DomainError::InvariantViolation { entity, rule }
}

#[cfg(test)]
mod tests {
    use agent_room_domain::{
        ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
        private_rooms::PrivateRoom,
        rooms::{
            MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
            RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
            RoomInstanceState,
        },
    };
    use uuid::Uuid;

    use crate::ports::{
        MatrixCreateRoom, MatrixRoomAliasLocalpart, MatrixRoomPowerProfile, MatrixRoomPreset,
        MatrixRoomVisibility,
    };

    use super::{PrivateMatrixRoomCreation, PrivateRoomSnapshot};

    #[test]
    fn 私人_matrix_建房约束拒绝公开或非受管请求() {
        let alias = MatrixRoomAliasLocalpart::new("agent-room-private-1").expect("别名有效");
        let public = MatrixCreateRoom::new(
            Some("项目室".to_owned()),
            None,
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("基础请求有效")
        .with_alias_localpart(alias.clone());
        assert!(PrivateMatrixRoomCreation::new(public, alias.clone()).is_err());

        let private = MatrixCreateRoom::new(
            Some("项目室".to_owned()),
            None,
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::PrivateChat,
            false,
            Vec::new(),
        )
        .expect("基础请求有效")
        .with_alias_localpart(alias.clone())
        .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
        assert!(
            PrivateMatrixRoomCreation::new(private.clone(), alias.clone()).is_err(),
            "私人房间要加密"
        );
        assert!(PrivateMatrixRoomCreation::new_public(private.clone(), alias.clone()).is_ok());
        let encrypted = private.with_end_to_end_encryption();
        assert!(PrivateMatrixRoomCreation::new(encrypted.clone(), alias.clone()).is_ok());
        assert!(
            PrivateMatrixRoomCreation::new_public(encrypted, alias).is_err(),
            "新建时就公开的不加密"
        );
    }

    #[test]
    fn 快照拒绝把不同房间的事实拼接到一起() {
        let room = PrivateRoom::create(catalog_id(1), principal_id());
        assert!(PrivateRoomSnapshot::new(catalog(1), instance(2), room).is_err());
    }

    #[test]
    fn 快照拒绝生命周期不一致() {
        let room = PrivateRoom::create(catalog_id(1), principal_id());
        let archived_instance = RoomInstance::restore(
            RoomInstanceId::from_uuid(Uuid::from_u128(2)),
            RoomInstanceFields {
                state: RoomInstanceState::Archived,
                ..instance_fields(1)
            },
        )
        .expect("归档实例有效");

        assert!(PrivateRoomSnapshot::new(catalog(1), archived_instance, room).is_err());
    }

    fn catalog(sequence: u128) -> RoomCatalog {
        RoomCatalog::new(
            catalog_id(sequence),
            RoomCatalogFields {
                kind: RoomCatalogKind::PrivateRoom,
                slug: None,
                name: "私人项目室".to_owned(),
                description: String::new(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(principal_id()),
                visibility: RoomCatalogVisibility::Private,
                retention_days: Some(30),
                status: RoomCatalogStatus::Active,
            },
        )
        .expect("私人目录有效")
    }

    fn instance(sequence: u128) -> RoomInstance {
        RoomInstance::restore(
            RoomInstanceId::from_uuid(Uuid::from_u128(sequence + 100)),
            instance_fields(sequence),
        )
        .expect("私人房间实例有效")
    }

    fn instance_fields(sequence: u128) -> RoomInstanceFields {
        RoomInstanceFields {
            catalog_id: catalog_id(sequence),
            matrix_room_id: MatrixRoomReference::new(format!("!private{sequence}:matrix.test"))
                .expect("Matrix 房间标识有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            allocated_slots: 0,
            state: RoomInstanceState::Active,
        }
    }

    fn catalog_id(sequence: u128) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(sequence))
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(99))
    }
}
