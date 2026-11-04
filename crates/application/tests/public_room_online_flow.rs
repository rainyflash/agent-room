//! 详情里的“现在几个在线”（`specs/public-rooms/design.md`）：离开的也算在线；60 秒内再看用上次的
//! 数；人多的只问 200 个、写“至少”；有几个没问到也写“至少”；一个都没问到、库读不了时不写数，
//! 详情照样给。

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixUserId,
        PortFuture, PublicRoomDetailRecord, PublicRoomDirectoryStore, PublicRoomPresenceMatrix,
        PublicRoomRecord, PublicRoomSearch, PublicRoomViewerState,
    },
    public_room_directory::{
        MAX_PRESENCE_CHECKS, ONLINE_COUNT_TTL_MILLIS, PublicRoomDirectoryDependencies,
        PublicRoomDirectoryService, PublicRoomDirectoryUseCases, PublicRoomOnline,
    },
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState,
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 在线和离开的都算_一分钟内再看用上次的数() {
    let fixture = Fixture::new(vec!["online", "unavailable", "offline"]);

    assert_eq!(fixture.online().await, Some(counted(2, false)));
    fixture.clock.advance(ONLINE_COUNT_TTL_MILLIS - 1);
    assert_eq!(fixture.online().await, Some(counted(2, false)));
    assert_eq!(fixture.directory.candidate_reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.presence.asked.load(Ordering::SeqCst), 3);

    fixture.clock.advance(1);
    fixture.online().await;
    assert_eq!(
        fixture.directory.candidate_reads.load(Ordering::SeqCst),
        2,
        "过了一分钟重新问"
    );
}

#[tokio::test]
async fn 人多的只问二百个_写至少() {
    let fixture = Fixture::new(vec!["online"; MAX_PRESENCE_CHECKS as usize + 1]);

    assert_eq!(
        fixture.online().await,
        Some(counted(MAX_PRESENCE_CHECKS, true))
    );
    assert_eq!(
        *fixture.directory.limits.lock().unwrap(),
        [MAX_PRESENCE_CHECKS + 1],
        "多拿一个看是不是没问全"
    );
    assert_eq!(
        fixture.presence.asked.load(Ordering::SeqCst),
        MAX_PRESENCE_CHECKS as usize
    );
}

#[tokio::test]
async fn 有几个没问到写至少_一个都没问到不写数() {
    let partly = Fixture::new(vec!["online", "fails", "offline"]);
    assert_eq!(partly.online().await, Some(counted(1, true)));

    let none = Fixture::new(vec!["fails", "fails"]);
    assert_eq!(none.online().await, None);

    let empty = Fixture::new(Vec::new());
    assert_eq!(empty.online().await, Some(counted(0, false)));
}

#[tokio::test]
async fn 库读不了不写数_详情照样给() {
    let fixture = Fixture::new(vec!["online"]);
    fixture
        .directory
        .candidates_fail
        .store(true, Ordering::SeqCst);

    let detail = fixture
        .service
        .detail(None, "Room00".to_owned())
        .await
        .expect("详情照样给");
    assert_eq!(detail.online, None);
    assert_eq!(fixture.presence.asked.load(Ordering::SeqCst), 0);
}

fn counted(count: u32, at_least: bool) -> PublicRoomOnline {
    PublicRoomOnline { count, at_least }
}

struct Fixture {
    service: PublicRoomDirectoryService,
    directory: Arc<OneRoom>,
    presence: Arc<ScriptedPresence>,
    clock: Arc<MovingClock>,
}

impl Fixture {
    /// 房间里每个人的在线状态按顺序给：`online`、`unavailable`、`offline` 或问不到的 `fails`。
    fn new(people: Vec<&'static str>) -> Self {
        let directory = Arc::new(OneRoom {
            people: people.len(),
            ..OneRoom::default()
        });
        let presence = Arc::new(ScriptedPresence {
            answers: people,
            asked: AtomicUsize::new(0),
        });
        let clock = Arc::new(MovingClock(AtomicI64::new(NOW)));
        let service = PublicRoomDirectoryService::new(PublicRoomDirectoryDependencies {
            directory: directory.clone(),
            presence: presence.clone(),
            clock: clock.clone(),
        });
        Self {
            service,
            directory,
            presence,
            clock,
        }
    }

    async fn online(&self) -> Option<PublicRoomOnline> {
        self.service
            .detail(None, "Room00".to_owned())
            .await
            .expect("有这间房")
            .online
    }
}

struct MovingClock(AtomicI64);

impl MovingClock {
    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for MovingClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

fn user(index: usize) -> MatrixUserId {
    MatrixUserId::new(format!("@member{index}:matrix.test")).unwrap()
}

/// 只有一间公开房间，里面 `people` 个人和 Agent。
#[derive(Default)]
struct OneRoom {
    people: usize,
    candidates_fail: AtomicBool,
    candidate_reads: AtomicUsize,
    limits: Mutex<Vec<u32>>,
}

impl PublicRoomDirectoryStore for OneRoom {
    fn search_public_rooms<'a>(
        &'a self,
        _search: &'a PublicRoomSearch,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicRoomRecord>>> {
        Box::pin(async { unreachable!("不翻目录") })
    }

    fn public_room_by_share_code<'a>(
        &'a self,
        _share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<PublicRoomDetailRecord>>> {
        Box::pin(async {
            Ok(Some(PublicRoomDetailRecord {
                room: PublicRoomRecord {
                    catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(100)),
                    name: "读书会".to_owned(),
                    description: String::new(),
                    owner_display_name: "阿明".to_owned(),
                    member_count: 3,
                    agent_count: 0,
                    join_mode: RoomJoinMode::Open,
                    end_to_end_encrypted: false,
                    share_code: RoomShareCode::parse("Room00").unwrap(),
                    created_at: UtcMillis::new(NOW - 10_000).unwrap(),
                    last_message_at: None,
                },
                admin_display_names: Vec::new(),
            }))
        })
    }

    fn public_room_viewer_states<'a>(
        &'a self,
        _viewer: PrincipalId,
        _rooms: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, PublicRoomViewerState)>>> {
        Box::pin(async { unreachable!("没登录的不查处境") })
    }

    fn public_room_presence_candidates(
        &self,
        _catalog_id: RoomCatalogId,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<MatrixUserId>>> {
        self.candidate_reads.fetch_add(1, Ordering::SeqCst);
        self.limits.lock().unwrap().push(limit);
        let result = if self.candidates_fail.load(Ordering::SeqCst) {
            Err(RepositoryError::new(
                "test.candidates",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok((0..self.people.min(limit as usize)).map(user).collect())
        };
        Box::pin(async move { result })
    }
}

struct ScriptedPresence {
    answers: Vec<&'static str>,
    asked: AtomicUsize,
}

impl PublicRoomPresenceMatrix for ScriptedPresence {
    fn presence<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixPresenceState>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let index: usize = user_id
            .as_str()
            .trim_start_matches("@member")
            .trim_end_matches(":matrix.test")
            .parse()
            .unwrap();
        let answer = match self.answers[index] {
            "online" => Ok(MatrixPresenceState::Online),
            "unavailable" => Ok(MatrixPresenceState::Unavailable),
            "offline" => Ok(MatrixPresenceState::Offline),
            _ => Err(MatrixFailure::new(
                MatrixOperation::ReadPresence,
                MatrixFailureKind::Forbidden,
            )),
        };
        Box::pin(async move { answer })
    }
}
