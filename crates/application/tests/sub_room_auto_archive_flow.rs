//! 子房间 30 天没人说话自动归档（`specs/redesign/sub-rooms.md` 第 2j-2 步）：领走安静的子房间，动手前再
//! 问一次聊天服务器；确实安静的归档，有人说过话的记下时间不动，读不了的、存不进的下一轮再来；一批领满
//! 了接着领，领不满就停；库领不了整轮作罢。

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PortFuture, PublicRoomActivityMatrix, QuietSubRoom, SubRoomActivityStore,
    },
    private_rooms::{
        InactiveSubRoomArchival, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomResult,
    },
    sub_room_auto_archive::{
        SubRoomAutoArchiveDependencies, SubRoomAutoArchiveRound, SubRoomAutoArchiveService,
        SubRoomAutoArchiveUseCases,
    },
};
use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1000;

#[tokio::test]
async fn 确实安静的归档_有人说过话的记下时间_读不了的和存不进的下一轮再来() {
    let fake = Arc::new(Fake::with_batches([(0..6).map(quiet).collect()]));

    let round = service(&fake).archive_quiet().await.expect("这一轮做完");

    assert_eq!(
        round,
        SubRoomAutoArchiveRound {
            claimed: 6,
            archived: 2,
            still_talking: 1,
            unreadable: 1,
            failed: 1,
        }
    );
    assert_eq!(
        *fake.claims.lock().unwrap(),
        [(
            time(NOW - 30 * DAY),
            time(NOW),
            time(NOW + 10 * 60 * 1000),
            50
        )],
        "30 天以前就没人说话的，租 10 分钟，一次 50 间"
    );
    assert_eq!(
        *fake.archived.lock().unwrap(),
        [
            (parent(), sub_room(0)),
            (parent(), sub_room(3)),
            (parent(), sub_room(4)),
            (parent(), sub_room(5))
        ],
        "聊天服务器上也一样安静、没有消息的才去归档；4 已经不在了，5 存不进"
    );
    assert_eq!(
        *fake.recorded.lock().unwrap(),
        [(sub_room(1), time(NOW - DAY))],
        "前一天有人说过话：记下时间，不动它"
    );
}

#[tokio::test]
async fn 一批领满了接着领_领不满就停() {
    let fake = Arc::new(Fake::with_batches([
        (100..150).map(quiet).collect(),
        vec![quiet(150)],
    ]));
    let round = service(&fake).archive_quiet().await.expect("这一轮做完");
    assert_eq!(round.claimed, 51);
    assert_eq!(round.archived, 51);
    assert_eq!(fake.claims.lock().unwrap().len(), 2, "第二批没领满，不再领");
}

#[tokio::test]
async fn 库领不了整轮作罢() {
    let fake = Arc::new(Fake {
        claim_fails: true,
        ..Fake::default()
    });
    let failure = service(&fake).archive_quiet().await.unwrap_err();
    assert_eq!(failure.operation(), "test.claim");
    assert!(fake.archived.lock().unwrap().is_empty());
}

fn service(fake: &Arc<Fake>) -> SubRoomAutoArchiveService {
    SubRoomAutoArchiveService::new(SubRoomAutoArchiveDependencies {
        store: fake.clone(),
        matrix: fake.clone(),
        archival: fake.clone(),
        clock: Arc::new(TestClock),
    })
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn parent() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(1))
}

fn sub_room(index: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(1_000 + index))
}

fn quiet(index: u128) -> QuietSubRoom {
    QuietSubRoom {
        catalog_id: sub_room(index),
        parent_catalog_id: parent(),
        matrix_room: MatrixRoomId::new(format!("!sub{index}:matrix.test")).unwrap(),
    }
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

/// 聊天服务器上：0 最后一句在 40 天前；1 前一天有人说话；2 读不了；3 一句都没有；4 归档时已经不在了；
/// 5 存的时候撞上别人刚改过。再往后的都没有消息。
#[derive(Default)]
struct Fake {
    batches: Mutex<VecDeque<Vec<QuietSubRoom>>>,
    claim_fails: bool,
    claims: Mutex<Vec<(UtcMillis, UtcMillis, UtcMillis, u16)>>,
    recorded: Mutex<Vec<(RoomCatalogId, UtcMillis)>>,
    archived: Mutex<Vec<(RoomCatalogId, RoomCatalogId)>>,
}

impl Fake {
    fn with_batches<const N: usize>(batches: [Vec<QuietSubRoom>; N]) -> Self {
        Self {
            batches: Mutex::new(batches.into_iter().collect()),
            ..Self::default()
        }
    }
}

impl SubRoomActivityStore for Fake {
    fn sub_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn record_sub_room_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        self.recorded.lock().unwrap().push((catalog_id, at));
        Box::pin(async { Ok(()) })
    }

    fn claim_quiet_sub_rooms(
        &self,
        quiet_before: UtcMillis,
        now: UtcMillis,
        lease_until: UtcMillis,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<QuietSubRoom>>> {
        self.claims
            .lock()
            .unwrap()
            .push((quiet_before, now, lease_until, limit));
        let result = if self.claim_fails {
            Err(RepositoryError::new(
                "test.claim",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self.batches.lock().unwrap().pop_front().unwrap_or_default())
        };
        Box::pin(async move { result })
    }
}

impl PublicRoomActivityMatrix for Fake {
    fn latest_message_at<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<UtcMillis>>> {
        let answer = match room_id.as_str() {
            "!sub0:matrix.test" => Ok(Some(time(NOW - 40 * DAY))),
            "!sub1:matrix.test" => Ok(Some(time(NOW - DAY))),
            "!sub2:matrix.test" => Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::DependencyUnavailable,
            )),
            _ => Ok(None),
        };
        Box::pin(async move { answer })
    }
}

impl InactiveSubRoomArchival for Fake {
    fn archive_inactive_sub_room(
        &self,
        parent_catalog_id: RoomCatalogId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, PrivateRoomResult<bool>> {
        self.archived
            .lock()
            .unwrap()
            .push((parent_catalog_id, catalog_id));
        let answer = if catalog_id == sub_room(4) {
            Ok(false)
        } else if catalog_id == sub_room(5) {
            Err(PrivateRoomFailure::new(
                "test.archive",
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::Conflict,
            ))
        } else {
            Ok(true)
        };
        Box::pin(async move { answer })
    }
}
