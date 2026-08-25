use agent_room_domain::{
    DomainError, DomainResult,
    ids::RoomCatalogId,
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogKind, RoomCatalogStatus, RoomInstance,
        RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

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

    use super::PrivateRoomSnapshot;

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
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
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
