//! 谁读得到身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：房主和管理员，归档了也读得到；
//! 成员读不到，不在房间里的和没有这个房间一样回答。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, PortFuture, PrivateRoomSnapshot, PrivateRoomStore, RoomRoleChangeLog,
        RoomRoleChangeRecord,
    },
    room_role_changes::{
        ListRoomRoleChanges, MAX_ROLE_CHANGES, RoomRoleChangeDependencies, RoomRoleChangeFailure,
        RoomRoleChangeService, RoomRoleChangeUseCases,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    room_role_changes::RoomStanding,
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;
const OWNER: u128 = 1;
const ADMIN: u128 = 2;
const MEMBER: u128 = 3;
const INVITED_ADMIN: u128 = 4;

#[tokio::test]
async fn 房主和管理员读得到_新的在前_最多一百条() {
    for reader in [OWNER, ADMIN] {
        let (service, log) = build(Some(room(false)));

        let changes = service
            .recent(request(reader))
            .await
            .expect("管整间房的人读得到");

        assert_eq!(changes, vec![record()]);
        assert_eq!(*log.asked.lock().unwrap(), vec![(catalog_id(), 100)]);
        assert_eq!(MAX_ROLE_CHANGES, 100);
    }
}

#[tokio::test]
async fn 归档了的房间照样读得到() {
    let (service, _log) = build(Some(room(true)));

    for reader in [OWNER, ADMIN] {
        service
            .recent(request(reader))
            .await
            .expect("归档以后还能查谁动过谁");
    }
}

#[tokio::test]
async fn 成员读不到_不在房间里的和没有这个房间一样() {
    let (service, log) = build(Some(room(false)));

    let member = service.recent(request(MEMBER)).await.expect_err("成员");
    assert_eq!(member, RoomRoleChangeFailure::ManagersOnly);
    // 受邀还没进来的管理员先不给看：进来以后才算房间里的人。
    for outsider in [INVITED_ADMIN, 99] {
        let failure = service.recent(request(outsider)).await.expect_err("外人");
        assert_eq!(failure, RoomRoleChangeFailure::NotFound);
    }

    let (missing, missing_log) = build(None);
    let failure = missing.recent(request(OWNER)).await.expect_err("没有房间");
    assert_eq!(failure, RoomRoleChangeFailure::NotFound);

    let mut expired = request(OWNER);
    expired.actor.expires_at = time(NOW);
    let failure = service.recent(expired).await.expect_err("登录过期");
    assert_eq!(failure, RoomRoleChangeFailure::Forbidden);

    assert!(log.asked.lock().unwrap().is_empty(), "被拒的都没去读记录");
    assert!(missing_log.asked.lock().unwrap().is_empty());
}

#[derive(Default)]
struct 记录 {
    asked: Mutex<Vec<(RoomCatalogId, u16)>>,
}

impl RoomRoleChangeLog for 记录 {
    fn recent_role_changes(
        &self,
        catalog_id: RoomCatalogId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomRoleChangeRecord>>> {
        self.asked.lock().unwrap().push((catalog_id, limit));
        Box::pin(async { Ok(vec![record()]) })
    }
}

struct 固定私人房间(Option<PrivateRoomSnapshot>);

impl PrivateRoomStore for 固定私人房间 {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }

    fn find_by_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("只读一个房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }
}

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

fn build(private: Option<PrivateRoomSnapshot>) -> (RoomRoleChangeService, Arc<记录>) {
    let log = Arc::new(记录::default());
    let service = RoomRoleChangeService::new(RoomRoleChangeDependencies {
        log: log.clone(),
        private_rooms: Arc::new(固定私人房间(private)),
        clock: Arc::new(固定时钟),
    });
    (service, log)
}

/// 房主 1 号，2 号是已加入的管理员，3 号是已加入的成员，4 号是受邀还没进来的管理员。
fn room(archived: bool) -> PrivateRoomSnapshot {
    let owner = principal(OWNER);
    let mut room = PrivateRoom::create(catalog_id(), owner);
    for (target, permissions, joins) in [
        (ADMIN, admin(), true),
        (MEMBER, member(), true),
        (INVITED_ADMIN, admin(), false),
    ] {
        room.invite(owner, principal(target), permissions)
            .expect("邀请成功");
        if joins {
            room.accept_invitation(principal(target))
                .expect("接受邀请成功");
        }
    }
    if archived {
        room.archive(owner).expect("房主能归档");
    }
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "项目室".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: if archived {
                RoomCatalogStatus::Archived
            } else {
                RoomCatalogStatus::Active
            },
        },
    )
    .expect("目录有效");
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(11)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!project:matrix.test".to_owned())
                .expect("房间标识有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            allocated_slots: 0,
            state: if archived {
                RoomInstanceState::Archived
            } else {
                RoomInstanceState::Active
            },
        },
    )
    .expect("私人实例有效");
    PrivateRoomSnapshot::new(catalog, instance, room).expect("快照一致")
}

fn record() -> RoomRoleChangeRecord {
    let standing = |permissions| RoomStanding {
        status: PrivateRoomMembershipStatus::Joined,
        owner: false,
        permissions,
    };
    RoomRoleChangeRecord {
        id: 7,
        actor: Some(principal(OWNER)),
        target: principal(ADMIN),
        previous: Some(standing(member())),
        next: standing(admin()),
        changed_at: time(NOW - 5_000),
    }
}

fn request(reader: u128) -> ListRoomRoleChanges {
    ListRoomRoleChanges {
        actor: AuthenticatedPrincipal {
            principal_id: principal(reader),
            matrix_user_id: "@user:matrix.test".to_owned(),
            display_name: "User".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(NOW - 1_000),
            expires_at: time(NOW + 60_000),
            recently_authenticated: false,
        },
        catalog_id: catalog_id(),
    }
}

fn admin() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Invite,
        PrivateRoomCapability::Manage,
    ])
    .expect("权限有效")
}

fn member() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("权限有效")
}

fn principal(value: u128) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(value))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("时间有效")
}
