//! 发现页的“最近有人说话”（`specs/public-rooms/design.md`）：每间公开房间读最新一条消息的时间，
//! 比此刻还晚的按此刻记；没有消息的不写；单间读不了只算一间、接着读别的；库读不了整轮作罢。子房间
//! （`specs/redesign/sub-rooms.md` 第 2j-1 步）在同一轮里接着读，记到子房间那边。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PortFuture, PublicRoomActivityMatrix, PublicRoomActivityStore, QuietSubRoom,
        SubRoomActivityStore,
    },
    public_room_activity::{
        PublicRoomActivityDependencies, PublicRoomActivityRound, PublicRoomActivityService,
        PublicRoomActivityUseCases,
    },
};
use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 每间房记最新一条的时间_晚于此刻的按此刻_没有消息的不写_读不了的接着读别的() {
    let store = Arc::new(MemoryActivity::default());
    let service = service(store.clone());

    let round = service.refresh().await.expect("这一轮做完");

    assert_eq!(
        round,
        PublicRoomActivityRound {
            checked: 6,
            recorded: 3,
            unreadable: 2,
        },
        "公开房间四间、子房间两间一起数"
    );
    assert_eq!(
        *store.recorded.lock().unwrap(),
        [(room(0), time(NOW - 60_000)), (room(1), time(NOW)),],
        "别的服务器写的时间比此刻晚，按此刻记"
    );
    assert_eq!(
        *store.sub_rooms_recorded.lock().unwrap(),
        [(sub_room(0), time(NOW - 120_000))],
        "子房间的记到子房间那边"
    );
}

#[tokio::test]
async fn 子房间列不出来时公开房间这一轮照样记下了() {
    let store = Arc::new(MemoryActivity {
        sub_rooms_fail: true,
        ..MemoryActivity::default()
    });
    let failure = service(store.clone()).refresh().await.unwrap_err();
    assert_eq!(failure.operation(), "test.sub_rooms");
    assert_eq!(store.recorded.lock().unwrap().len(), 2);
    assert!(store.sub_rooms_recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 库读不了整轮作罢_写不进也停下() {
    let unreadable = Arc::new(MemoryActivity {
        list_fails: true,
        ..MemoryActivity::default()
    });
    let failure = service(unreadable).refresh().await.unwrap_err();
    assert_eq!(failure.kind(), RepositoryErrorKind::Unavailable);

    let unwritable = Arc::new(MemoryActivity {
        record_fails: true,
        ..MemoryActivity::default()
    });
    let failure = service(unwritable.clone()).refresh().await.unwrap_err();
    assert_eq!(failure.operation(), "test.record");
    assert_eq!(
        *unwritable.read.lock().unwrap(),
        [matrix_room(0)],
        "第一间写不进就不再往下读"
    );
}

fn service(store: Arc<MemoryActivity>) -> PublicRoomActivityService {
    PublicRoomActivityService::new(PublicRoomActivityDependencies {
        store: store.clone(),
        sub_rooms: store.clone(),
        matrix: store,
        clock: Arc::new(TestClock),
    })
}

fn room(index: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(100 + index))
}

fn sub_room(index: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(200 + index))
}

fn sub_room_matrix_room(index: u128) -> MatrixRoomId {
    MatrixRoomId::new(format!("!sub{index}:matrix.test")).unwrap()
}

fn matrix_room(index: u128) -> MatrixRoomId {
    MatrixRoomId::new(format!("!public{index}:matrix.test")).unwrap()
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

/// 四间公开房间：一分钟前有人说话；时间写在一小时以后；还没人说过话；Matrix 上读不了。两间子房间：
/// 两分钟前有人说话；Matrix 上读不了。
#[derive(Default)]
struct MemoryActivity {
    list_fails: bool,
    record_fails: bool,
    sub_rooms_fail: bool,
    read: Mutex<Vec<MatrixRoomId>>,
    recorded: Mutex<Vec<(RoomCatalogId, UtcMillis)>>,
    sub_rooms_recorded: Mutex<Vec<(RoomCatalogId, UtcMillis)>>,
}

impl SubRoomActivityStore for MemoryActivity {
    fn sub_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>> {
        let rooms = if self.sub_rooms_fail {
            Err(RepositoryError::new(
                "test.sub_rooms",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok((0..2)
                .map(|index| (sub_room(index), sub_room_matrix_room(index)))
                .collect())
        };
        Box::pin(async move { rooms })
    }

    fn record_sub_room_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        self.sub_rooms_recorded
            .lock()
            .unwrap()
            .push((catalog_id, at));
        Box::pin(async { Ok(()) })
    }

    fn claim_quiet_sub_rooms(
        &self,
        _quiet_before: UtcMillis,
        _now: UtcMillis,
        _lease_until: UtcMillis,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<QuietSubRoom>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

impl PublicRoomActivityStore for MemoryActivity {
    fn listed_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>> {
        let rooms = if self.list_fails {
            Err(RepositoryError::new(
                "test.list",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok((0..4)
                .map(|index| (room(index), matrix_room(index)))
                .collect())
        };
        Box::pin(async move { rooms })
    }

    fn record_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let result = if self.record_fails {
            Err(RepositoryError::new(
                "test.record",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            self.recorded.lock().unwrap().push((catalog_id, at));
            Ok(())
        };
        Box::pin(async move { result })
    }
}

impl PublicRoomActivityMatrix for MemoryActivity {
    fn latest_message_at<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<UtcMillis>>> {
        self.read.lock().unwrap().push(room_id.clone());
        let answer = match room_id.as_str() {
            "!public0:matrix.test" => Ok(Some(time(NOW - 60_000))),
            "!public1:matrix.test" => Ok(Some(time(NOW + 3_600_000))),
            "!public2:matrix.test" => Ok(None),
            "!sub0:matrix.test" => Ok(Some(time(NOW - 120_000))),
            _ => Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::Forbidden,
            )),
        };
        Box::pin(async move { answer })
    }
}
