//! 房主和管理员置顶房间里的一句（`specs/redesign/brief-and-pins.md`“置顶”）：查“管理”这一位、房间在
//! 使用中、那一句是这间房里的消息；取消置顶不问那一句；满了照实说；次数和须知合起来算。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PinTarget, PortFuture, PrivateRoomNoticeGateway, PrivateRoomPinsGateway,
        PrivateRoomSnapshot, PrivateRoomStore, RoomNoticeWrite, RoomPinsChange,
        RoomPinsWriteFailure,
    },
    private_rooms::{
        PinsFailureKind, PrivateRoomNoticeDependencies, PrivateRoomNoticeService,
        PrivateRoomNoticeUseCases, PrivateRoomPinsDependencies, PrivateRoomPinsService,
        PrivateRoomPinsUseCases, ROOM_STATE_WRITES_PER_HOUR, RoomPinView, RoomStateWriteLimit,
        UpdateRoomNotice, UpdateRoomPins,
    },
};
use agent_room_domain::{
    DomainError,
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_pins::{RoomPin, RoomPins},
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
async fn 房主和管理员能置顶_新的在最前面_取消以后不在了() {
    let fixture = Fixture::new(room());

    fixture
        .service
        .update_pins(update(owner(), "$first", true))
        .await
        .expect("房主能置顶");
    let pinned = fixture
        .service
        .update_pins(update(manager(), "$second", true))
        .await
        .expect("管理员能置顶");
    assert_eq!(
        pinned,
        [
            RoomPinView {
                event_id: "$second".to_owned(),
                pinned_by: matrix_user(manager()),
                pinned_at: time(NOW),
            },
            RoomPinView {
                event_id: "$first".to_owned(),
                pinned_by: matrix_user(owner()),
                pinned_at: time(NOW),
            },
        ]
    );
    let left = fixture
        .service
        .update_pins(update(manager(), "$first", false))
        .await
        .expect("能取消");
    assert_eq!(
        left.iter()
            .map(|pin| pin.event_id.as_str())
            .collect::<Vec<_>>(),
        ["$second"]
    );
    assert_eq!(
        *fixture.matrix.checked.lock().unwrap(),
        ["$first", "$second"]
    );
}

#[tokio::test]
async fn 普通成员不能_不在房间里的看不到_事件_id_不对_归档了的不行() {
    let fixture = Fixture::new(room());
    for (actor, event_id, expected) in [
        (member(), "$first", PinsFailureKind::Forbidden),
        (stranger(), "$first", PinsFailureKind::NotFound),
        (owner(), "first", PinsFailureKind::InvalidRequest),
        (owner(), "$has space", PinsFailureKind::InvalidRequest),
    ] {
        let failure = fixture
            .service
            .update_pins(update(actor, event_id, true))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), expected, "{actor:?} {event_id}");
    }

    let mut archived = room();
    archived.archive(owner()).unwrap();
    let failure = Fixture::new(archived)
        .service
        .update_pins(update(owner(), "$first", true))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PinsFailureKind::Conflict);
    assert!(fixture.matrix.pins.lock().unwrap().entries().is_empty());
}

#[tokio::test]
async fn 不是这间房里的消息不收_取消置顶不问那一句() {
    let fixture = Fixture::new(room());
    let failure = fixture
        .service
        .update_pins(update(owner(), "$gone", true))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PinsFailureKind::InvalidEvent);
    assert!(fixture.matrix.pins.lock().unwrap().entries().is_empty());

    // 那一句置顶以后被删了：照样能取消，不先问它还在不在。
    fixture
        .matrix
        .pins
        .lock()
        .unwrap()
        .pin(pin("$gone"))
        .unwrap();
    fixture.matrix.checked.lock().unwrap().clear();
    let left = fixture
        .service
        .update_pins(update(owner(), "$gone", false))
        .await
        .expect("删了的那句也能取消");
    assert!(left.is_empty());
    assert!(fixture.matrix.checked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 满了照实说_matrix_写不进报暂时不可用_登录过期的不行() {
    let fixture = Fixture::new(room());
    for n in 1..=5 {
        fixture
            .service
            .update_pins(update(owner(), &format!("${n}"), true))
            .await
            .expect("五条以内");
    }
    let failure = fixture
        .service
        .update_pins(update(owner(), "$6", true))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PinsFailureKind::Full);

    *fixture.matrix.fails.lock().unwrap() = true;
    let failure = fixture
        .service
        .update_pins(update(owner(), "$1", false))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PinsFailureKind::DependencyUnavailable);

    let expired = UpdateRoomPins {
        actor: AuthenticatedPrincipal {
            expires_at: time(NOW),
            ..principal(owner())
        },
        ..update(owner(), "$1", false)
    };
    let failure = fixture.service.update_pins(expired).await.unwrap_err();
    assert_eq!(failure.kind(), PinsFailureKind::Forbidden);
}

#[tokio::test]
async fn 须知和置顶的次数合起来算_按人算() {
    let fixture = Fixture::new(room());
    let notices = PrivateRoomNoticeService::new(PrivateRoomNoticeDependencies {
        rooms: Arc::new(MemoryRooms(room())),
        matrix: Arc::new(AcceptNotices),
        clock: Arc::new(TestClock),
        writes: fixture.writes.clone(),
    });
    for n in 0..ROOM_STATE_WRITES_PER_HOUR {
        if n % 2 == 0 {
            notices
                .update_notice(UpdateRoomNotice {
                    actor: principal(owner()),
                    catalog_id: catalog_id(),
                    text: format!("第 {n} 版"),
                })
                .await
                .expect("上限以内");
        } else {
            fixture
                .service
                .update_pins(update(owner(), "$1", n % 4 == 1))
                .await
                .expect("上限以内");
        }
    }
    let failure = fixture
        .service
        .update_pins(update(owner(), "$2", true))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        PinsFailureKind::RateLimited {
            retry_at: time(NOW + 3_600_000)
        }
    );
    assert!(
        notices
            .update_notice(UpdateRoomNotice {
                actor: principal(owner()),
                catalog_id: catalog_id(),
                text: "再改一次".to_owned(),
            })
            .await
            .is_err(),
        "须知也满了"
    );
    fixture
        .service
        .update_pins(update(manager(), "$2", true))
        .await
        .expect("别人照样能置顶");
}

struct Fixture {
    service: PrivateRoomPinsService,
    matrix: Arc<FakePins>,
    writes: Arc<RoomStateWriteLimit>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let matrix = Arc::new(FakePins::default());
        let writes = Arc::new(RoomStateWriteLimit::new());
        let service = PrivateRoomPinsService::new(PrivateRoomPinsDependencies {
            rooms: Arc::new(MemoryRooms(room)),
            matrix: matrix.clone(),
            clock: Arc::new(TestClock),
            writes: writes.clone(),
        });
        Self {
            service,
            matrix,
            writes,
        }
    }
}

fn update(actor: PrincipalId, event_id: &str, pinned: bool) -> UpdateRoomPins {
    UpdateRoomPins {
        actor: principal(actor),
        catalog_id: catalog_id(),
        event_id: event_id.to_owned(),
        pinned,
    }
}

fn pin(event_id: &str) -> RoomPin {
    RoomPin {
        event_id: agent_room_domain::room_pins::RoomPinEventId::parse(event_id).unwrap(),
        pinned_by: matrix_user(owner()),
        pinned_at: time(NOW - 1_000),
    }
}

/// 照领域规则改一份放在内存里的置顶；`$gone` 不是这间房里的消息。
#[derive(Default)]
struct FakePins {
    pins: Mutex<RoomPins>,
    checked: Mutex<Vec<String>>,
    fails: Mutex<bool>,
}

impl PrivateRoomPinsGateway for FakePins {
    fn pin_target<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a agent_room_domain::room_pins::RoomPinEventId,
    ) -> PortFuture<'a, MatrixResult<PinTarget>> {
        assert_eq!(room_id.as_str(), "!club:matrix.test");
        self.checked
            .lock()
            .unwrap()
            .push(event_id.as_str().to_owned());
        let target = if event_id.as_str() == "$gone" {
            PinTarget::NotMessage
        } else {
            PinTarget::Message
        };
        Box::pin(async move { Ok(target) })
    }

    fn change_pins<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        change: &'a RoomPinsChange,
    ) -> PortFuture<'a, Result<RoomPins, RoomPinsWriteFailure>> {
        assert_eq!(room_id.as_str(), "!club:matrix.test");
        let result = if *self.fails.lock().unwrap() {
            Err(RoomPinsWriteFailure::Matrix(MatrixFailure::new(
                MatrixOperation::SendStateEvent,
                MatrixFailureKind::DependencyUnavailable,
            )))
        } else {
            let mut pins = self.pins.lock().unwrap();
            let changed = if change.pinned {
                pins.pin(RoomPin {
                    event_id: change.event_id.clone(),
                    pinned_by: change.actor.as_str().to_owned(),
                    pinned_at: change.at,
                })
                .map_err(|error| {
                    assert!(matches!(error, DomainError::CapacityExceeded { .. }));
                    RoomPinsWriteFailure::Full
                })
            } else {
                Ok(pins.unpin(&change.event_id))
            };
            changed.map(|_| pins.clone())
        };
        Box::pin(async move { result })
    }
}

struct AcceptNotices;

impl PrivateRoomNoticeGateway for AcceptNotices {
    fn set_notice<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _notice: &'a RoomNoticeWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { Ok(()) })
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
            allocated_slots: 0,
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

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("置顶不建房间") })
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
        Box::pin(async { unreachable!("置顶不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("置顶按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("置顶不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("置顶不改名") })
    }
}
