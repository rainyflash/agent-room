//! 房主改房间简介（`specs/public-rooms/design.md`）：只有房主，房间要在使用中；先写 Matrix 房间的
//! 话题再存；去掉首尾空白，没变的不写；太长的不收；Matrix 写不进就不存。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PortFuture, PrivateRoomDescriptionStore, PrivateRoomSnapshot, PrivateRoomStore,
        PrivateRoomTopicGateway,
    },
    private_rooms::{
        DescribePrivateRoom, DescriptionFailureKind, PrivateRoomDescriptionDependencies,
        PrivateRoomDescriptionService, PrivateRoomDescriptionUseCases,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 房主改简介_先写话题再存_去掉首尾空白() {
    let fixture = Fixture::new(room());

    let described = fixture
        .service
        .describe(describe(owner(), "  每周读一本书\n周五晚上聊  "))
        .await
        .expect("房主能改");

    assert_eq!(described, "每周读一本书\n周五晚上聊");
    assert_eq!(
        *fixture.calls.lock().unwrap(),
        [
            "topic !club:matrix.test 每周读一本书\n周五晚上聊".to_owned(),
            format!("store 每周读一本书\n周五晚上聊 {NOW}"),
        ]
    );
}

#[tokio::test]
async fn 和原来一样的不写_清空也算改() {
    let fixture = Fixture::new(room());
    let unchanged = fixture
        .service
        .describe(describe(owner(), "  原来的简介 "))
        .await
        .unwrap();
    assert_eq!(unchanged, "原来的简介");
    assert!(fixture.calls.lock().unwrap().is_empty());

    let cleared = fixture
        .service
        .describe(describe(owner(), "   "))
        .await
        .unwrap();
    assert_eq!(cleared, "");
    assert_eq!(fixture.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 只有房主能改_不在房间里的看不到_太长的不收_归档了的不行() {
    let fixture = Fixture::new(room());
    for (actor, description, expected) in [
        (manager(), "管理员想改", DescriptionFailureKind::Forbidden),
        (stranger(), "路人想改", DescriptionFailureKind::NotFound),
        (
            owner(),
            &"字".repeat(2_049),
            DescriptionFailureKind::InvalidRequest,
        ),
    ] {
        let failure = fixture
            .service
            .describe(describe(actor, description))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), expected, "{actor:?}");
    }

    let mut archived = room();
    archived.archive(owner()).unwrap();
    let failure = Fixture::new(archived)
        .service
        .describe(describe(owner(), "归档以后再改"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), DescriptionFailureKind::Conflict);
    assert!(fixture.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 话题写不进就不存_登录过期的不行() {
    let fixture = Fixture::new(room());
    *fixture.matrix_fails.lock().unwrap() = true;
    let failure = fixture
        .service
        .describe(describe(owner(), "新简介"))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        DescriptionFailureKind::DependencyUnavailable
    );
    assert_eq!(fixture.calls.lock().unwrap().len(), 1, "只写过话题");

    let expired = DescribePrivateRoom {
        actor: AuthenticatedPrincipal {
            expires_at: time(NOW),
            ..principal(owner())
        },
        ..describe(owner(), "新简介")
    };
    let failure = fixture.service.describe(expired).await.unwrap_err();
    assert_eq!(failure.kind(), DescriptionFailureKind::Forbidden);
}

struct Fixture {
    service: PrivateRoomDescriptionService,
    calls: Arc<Mutex<Vec<String>>>,
    matrix_fails: Arc<Mutex<bool>>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let matrix_fails = Arc::new(Mutex::new(false));
        let recorder = Arc::new(Recorder {
            calls: calls.clone(),
            matrix_fails: matrix_fails.clone(),
        });
        let service = PrivateRoomDescriptionService::new(PrivateRoomDescriptionDependencies {
            rooms: Arc::new(MemoryRooms(room)),
            descriptions: recorder.clone(),
            matrix: recorder,
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            calls,
            matrix_fails,
        }
    }
}

fn describe(actor: PrincipalId, description: &str) -> DescribePrivateRoom {
    DescribePrivateRoom {
        actor: principal(actor),
        catalog_id: catalog_id(),
        description: description.to_owned(),
    }
}

/// 房主、一位管理员。
fn room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(owner(), manager(), PrivateRoomPermissions::ALL)
        .unwrap();
    room.accept_invitation(manager()).unwrap();
    room
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
    let archived = room.status().as_str() == "archived";
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "读书会".to_owned(),
            description: "原来的简介".to_owned(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: if archived {
                RoomCatalogStatus::Archived
            } else {
                RoomCatalogStatus::Active
            },
        },
    )
    .unwrap();
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(20)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!club:matrix.test".to_owned()).unwrap(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: if archived {
                RoomInstanceState::Archived
            } else {
                RoomInstanceState::Active
            },
        },
    )
    .unwrap();
    PrivateRoomSnapshot::new(catalog, instance, room).unwrap()
}

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: "@someone:matrix.test".to_owned(),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: false,
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn manager() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn stranger() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

/// 按先后记下写话题和存简介。
struct Recorder {
    calls: Arc<Mutex<Vec<String>>>,
    matrix_fails: Arc<Mutex<bool>>,
}

impl PrivateRoomTopicGateway for Recorder {
    fn set_topic<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        topic: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("topic {} {topic}", room_id.as_str()));
        let result = if *self.matrix_fails.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::SendStateEvent,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(())
        };
        Box::pin(async move { result })
    }
}

impl PrivateRoomDescriptionStore for Recorder {
    fn describe<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        description: &'a str,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        assert_eq!(catalog_id, self::catalog_id());
        self.calls
            .lock()
            .unwrap()
            .push(format!("store {description} {}", changed_at.value()));
        Box::pin(async { Ok(()) })
    }
}

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改简介不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = (catalog_id == self.0.catalog_id()).then(|| snapshot(self.0.clone()));
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("改简介不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("改简介按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改简介不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改简介不改名") })
    }
}
