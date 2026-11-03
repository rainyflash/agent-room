//! 房主和管理员改房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：查“管理”这一位、
//! 房间在使用中，以应用服务写进 Matrix，写的是改的人和改的时间；空的是撤掉；太长的不收；
//! Matrix 写不进就报错；每个人一小时有上限。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PortFuture, PrivateRoomNoticeGateway, PrivateRoomSnapshot, PrivateRoomStore,
        RoomNoticeWrite,
    },
    private_rooms::{
        NOTICES_PER_HOUR, NoticeFailureKind, PrivateRoomNoticeDependencies,
        PrivateRoomNoticeService, PrivateRoomNoticeUseCases, RoomNotice, UpdateRoomNotice,
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
async fn 房主和管理员能写_写进去的是改的人和时间_空的是撤掉() {
    let fixture = Fixture::new(room());

    let written = fixture
        .service
        .update_notice(update(owner(), "  只在被点名时说话。\n回复要短。 "))
        .await
        .expect("房主能写");
    assert_eq!(
        written,
        RoomNotice {
            text: "只在被点名时说话。\n回复要短。".to_owned(),
            updated_at: time(NOW),
        }
    );
    let removed = fixture
        .service
        .update_notice(update(manager(), "  "))
        .await
        .expect("管理员能撤掉");
    assert_eq!(removed.text, "");

    assert_eq!(
        *fixture.writes.lock().unwrap(),
        [
            format!(
                "!club:matrix.test 只在被点名时说话。\n回复要短。 {} {NOW}",
                matrix_user(owner())
            ),
            format!("!club:matrix.test  {} {NOW}", matrix_user(manager())),
        ]
    );
}

#[tokio::test]
async fn 普通成员不能写_不在房间里的看不到_太长的不收_归档了的不行() {
    let fixture = Fixture::new(room());
    for (actor, text, expected) in [
        (member(), "成员想写", NoticeFailureKind::Forbidden),
        (stranger(), "路人想写", NoticeFailureKind::NotFound),
        (
            owner(),
            &"字".repeat(2_001),
            NoticeFailureKind::InvalidRequest,
        ),
        (owner(), "回车\r不收", NoticeFailureKind::InvalidRequest),
    ] {
        let failure = fixture
            .service
            .update_notice(update(actor, text))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), expected, "{actor:?}");
    }

    let mut archived = room();
    archived.archive(owner()).unwrap();
    let failure = Fixture::new(archived)
        .service
        .update_notice(update(owner(), "归档以后再写"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), NoticeFailureKind::Conflict);
    assert!(fixture.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 写不进_matrix_就报错_登录过期的不行() {
    let fixture = Fixture::new(room());
    *fixture.matrix_fails.lock().unwrap() = true;
    let failure = fixture
        .service
        .update_notice(update(owner(), "新公告"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), NoticeFailureKind::DependencyUnavailable);

    let expired = UpdateRoomNotice {
        actor: AuthenticatedPrincipal {
            expires_at: time(NOW),
            ..principal(owner())
        },
        ..update(owner(), "新公告")
    };
    let failure = fixture.service.update_notice(expired).await.unwrap_err();
    assert_eq!(failure.kind(), NoticeFailureKind::Forbidden);
}

#[tokio::test]
async fn 每个人一小时有上限_别人照样能写() {
    let fixture = Fixture::new(room());
    for _ in 0..NOTICES_PER_HOUR {
        fixture
            .service
            .update_notice(update(owner(), "改一改"))
            .await
            .expect("上限以内");
    }
    let failure = fixture
        .service
        .update_notice(update(owner(), "再改一次"))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        NoticeFailureKind::RateLimited {
            retry_at: time(NOW + 3_600_000)
        }
    );
    fixture
        .service
        .update_notice(update(manager(), "管理员来改"))
        .await
        .expect("按人算");
    assert_eq!(fixture.writes.lock().unwrap().len(), NOTICES_PER_HOUR + 1);
}

struct Fixture {
    service: PrivateRoomNoticeService,
    writes: Arc<Mutex<Vec<String>>>,
    matrix_fails: Arc<Mutex<bool>>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let matrix_fails = Arc::new(Mutex::new(false));
        let service = PrivateRoomNoticeService::new(PrivateRoomNoticeDependencies {
            rooms: Arc::new(MemoryRooms(room)),
            matrix: Arc::new(Recorder {
                writes: writes.clone(),
                matrix_fails: matrix_fails.clone(),
            }),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            writes,
            matrix_fails,
        }
    }
}

fn update(actor: PrincipalId, text: &str) -> UpdateRoomNotice {
    UpdateRoomNotice {
        actor: principal(actor),
        catalog_id: catalog_id(),
        text: text.to_owned(),
    }
}

/// 房主、一位管理员、一位能说话不能管理的成员。
fn room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(owner(), manager(), PrivateRoomPermissions::ALL)
        .unwrap();
    room.accept_invitation(manager()).unwrap();
    room.invite(owner(), member(), PrivateRoomPermissions::INVITED_BY_AGENT)
        .unwrap();
    room.accept_invitation(member()).unwrap();
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
            description: String::new(),
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
        matrix_user_id: matrix_user(principal_id),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: false,
    }
}

fn matrix_user(principal_id: PrincipalId) -> String {
    format!("@user-{}:matrix.test", principal_id.as_uuid().as_u128())
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn manager() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn member() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(3))
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

/// 记下写进 Matrix 的每一版：房间、正文、改的人、时间。
struct Recorder {
    writes: Arc<Mutex<Vec<String>>>,
    matrix_fails: Arc<Mutex<bool>>,
}

impl PrivateRoomNoticeGateway for Recorder {
    fn set_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        notice: &'a RoomNoticeWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        let result = if *self.matrix_fails.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::SendStateEvent,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            self.writes.lock().unwrap().push(format!(
                "{} {} {} {}",
                room_id.as_str(),
                notice.text.as_str(),
                notice.updated_by.as_str(),
                notice.updated_at.value()
            ));
            Ok(())
        };
        Box::pin(async move { result })
    }
}

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改公告不建房间") })
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
        Box::pin(async { unreachable!("改公告不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("改公告按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改公告不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("改公告不改名") })
    }
}
