use agent_room_domain::{
    DomainError, DomainResult,
    direct_sessions::{DirectContactPolicy, DirectSession, DirectSessionLifecycle},
    ids::{AgentId, ContentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomInstance, RoomInstanceFields, RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixCreateRoom, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomEncryption,
        MatrixRoomId, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, PortFuture,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectAgentProfile {
    pub agent_id: AgentId,
    pub matrix_user_id: MatrixUserId,
    pub display_name: String,
    pub avatar_content_id: Option<ContentId>,
}

/// 从权威 Agent 目录解析可联系目标，不能信任客户端自报的 Matrix 用户标识。
pub trait DirectSessionAgentDirectory: Send + Sync {
    /// 仅返回当前可被新建会话或新建联系事实引用的 Agent。
    fn find_contactable(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectAgentProfile>>>;

    /// 返回当前可联系目标，或与主体已有直接会话、屏蔽事实、所有权关系的 Agent。
    /// 该读取只用于恢复既有关系，不能替代新建会话时的可发现性检查。
    fn find_known_contact(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectAgentProfile>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMatrixRoomCreation {
    request: MatrixCreateRoom,
    alias: MatrixRoomAliasLocalpart,
    creator: MatrixUserId,
    peer: MatrixUserId,
}

impl DirectMatrixRoomCreation {
    /// 固化双人直接会话的 Matrix 建房边界。
    ///
    /// # Errors
    ///
    /// 请求不是私密、直接、稳定别名且仅邀请另一方时返回错误。
    pub fn new(
        request: MatrixCreateRoom,
        alias: MatrixRoomAliasLocalpart,
        creator: MatrixUserId,
        peer: MatrixUserId,
    ) -> DomainResult<Self> {
        let valid = creator != peer
            && request.visibility() == MatrixRoomVisibility::Private
            && request.preset() == MatrixRoomPreset::TrustedPrivateChat
            && request.encryption() == MatrixRoomEncryption::EndToEnd
            && request.direct()
            && request.alias_localpart() == Some(&alias)
            && request.invite() == [peer.clone()];
        if !valid {
            return Err(invariant(
                "direct_matrix_room_creation",
                "直接 Matrix 房间必须私密、双人、端到端加密、带稳定别名并标记为直接会话",
            ));
        }
        Ok(Self {
            request,
            alias,
            creator,
            peer,
        })
    }

    pub const fn request(&self) -> &MatrixCreateRoom {
        &self.request
    }

    pub const fn alias(&self) -> &MatrixRoomAliasLocalpart {
        &self.alias
    }

    pub const fn creator(&self) -> &MatrixUserId {
        &self.creator
    }

    pub const fn peer(&self) -> &MatrixUserId {
        &self.peer
    }
}

/// 代表受管目标 Agent 创建或对账 Matrix DM，并同步该 Agent 的 `m.direct` 账户数据。
pub trait DirectSessionMatrixProvisioner: Send + Sync {
    fn create<'a>(
        &'a self,
        creation: &'a DirectMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSessionRecord {
    catalog: RoomCatalog,
    instance: Option<RoomInstance>,
    session: DirectSession,
}

impl DirectSessionRecord {
    /// 组合直接会话目录、可选 Matrix 实例与会话聚合。
    ///
    /// # Errors
    ///
    /// 对象不属于同一会话，或生命周期投影互相矛盾时返回错误。
    pub fn new(
        catalog: RoomCatalog,
        instance: Option<RoomInstance>,
        session: DirectSession,
    ) -> DomainResult<Self> {
        if catalog.kind() != RoomCatalogKind::Direct
            || catalog.id() != session.catalog_id()
            || catalog.owner_principal_id() != Some(session.principal_id())
            || instance
                .as_ref()
                .is_some_and(|instance| instance.catalog_id() != session.catalog_id())
        {
            return Err(invariant(
                "direct_session_record",
                "组成对象必须属于同一直接会话",
            ));
        }
        let states_match = match session.lifecycle() {
            DirectSessionLifecycle::Provisioning => {
                catalog.status() == RoomCatalogStatus::Frozen && instance.is_none()
            }
            DirectSessionLifecycle::Active => {
                catalog.status() == RoomCatalogStatus::Active
                    && instance
                        .as_ref()
                        .is_some_and(|instance| instance.state() == RoomInstanceState::Active)
            }
            DirectSessionLifecycle::Failed => {
                catalog.status() == RoomCatalogStatus::Archived && instance.is_none()
            }
        };
        if !states_match {
            return Err(invariant(
                "direct_session_record",
                "目录和实例必须与直接会话生命周期一致",
            ));
        }
        Ok(Self {
            catalog,
            instance,
            session,
        })
    }

    pub const fn catalog(&self) -> &RoomCatalog {
        &self.catalog
    }

    pub const fn instance(&self) -> Option<&RoomInstance> {
        self.instance.as_ref()
    }

    pub const fn session(&self) -> &DirectSession {
        &self.session
    }

    /// 用已创建的 Matrix Room 激活预留记录。
    ///
    /// # Errors
    ///
    /// 会话状态无效或房间实例不满足领域约束时返回错误。
    pub fn activate(
        self,
        instance_id: RoomInstanceId,
        matrix_room_id: MatrixRoomReference,
    ) -> DomainResult<Self> {
        let mut session = self.session;
        session.activate()?;
        let catalog = RoomCatalog::new(
            self.catalog.id(),
            RoomCatalogFields {
                kind: RoomCatalogKind::Direct,
                slug: None,
                name: self.catalog.name().to_owned(),
                description: self.catalog.description().to_owned(),
                language: self.catalog.language().cloned(),
                matrix_space_id: None,
                owner_principal_id: Some(session.principal_id()),
                visibility: self.catalog.visibility(),
                retention_days: self.catalog.retention_days(),
                status: RoomCatalogStatus::Active,
            },
        )?;
        let instance = RoomInstance::restore(
            instance_id,
            RoomInstanceFields {
                catalog_id: session.catalog_id(),
                matrix_room_id,
                region: None,
                capacity: RoomCapacity::new(2, 3)?,
                projected_member_count: 1,
                allocated_slots: 0,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )?;
        Self::new(catalog, Some(instance), session)
    }
}

/// 持久化会话索引和屏蔽策略；消息时间线绝不能进入该端口。
pub trait DirectSessionStore: Send + Sync {
    fn reserve<'a>(
        &'a self,
        record: &'a DirectSessionRecord,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>>;

    fn activate<'a>(
        &'a self,
        record: &'a DirectSessionRecord,
        expected_version: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>>;

    fn find_by_participants(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>>;

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>>;

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<DirectSessionRecord>>>;

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<DirectSessionRecord>>>;

    fn contact_policy(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>>;

    fn set_principal_block(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
        blocked: bool,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>>;
}

const fn invariant(entity: &'static str, rule: &'static str) -> DomainError {
    DomainError::InvariantViolation { entity, rule }
}

#[cfg(test)]
mod tests {
    use crate::ports::{
        DirectMatrixRoomCreation, MatrixCreateRoom, MatrixRoomAliasLocalpart, MatrixRoomPreset,
        MatrixRoomVisibility, MatrixUserId,
    };

    #[test]
    fn 直接建房约束拒绝群聊或错误邀请对象() {
        let creator = MatrixUserId::new("@_agent_a:matrix.test").expect("创建者有效");
        let peer = MatrixUserId::new("@user_a:matrix.test").expect("对端有效");
        let alias = MatrixRoomAliasLocalpart::new("agent-room-direct-a").expect("别名有效");
        let group = MatrixCreateRoom::new(
            None,
            None,
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::TrustedPrivateChat,
            true,
            vec![
                peer.clone(),
                MatrixUserId::new("@third:matrix.test").expect("第三方有效"),
            ],
        )
        .expect("基础请求有效")
        .with_end_to_end_encryption()
        .with_alias_localpart(alias.clone());

        assert!(DirectMatrixRoomCreation::new(group, alias, creator, peer).is_err());
    }
}
