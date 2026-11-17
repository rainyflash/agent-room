//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7a）：放着实况的房间里已加入的人都能
//! 分享开了实况以后、连着的最多 30 句；只存是哪几句；每人每小时 20 段；自己分享的自己看、自己删，房主和
//! 管理员看这间房的全部、能删别人的；每天清一次。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, ExcerptMessageRef, ExcerptRoomRecord, ExcerptTimelineRead,
        ExcerptTimelineUnavailable, InsertRoomExcerptOutcome, MatrixEventId, MatrixRoomId,
        PortFuture, PrivateRoomSnapshot, PrivateRoomStore, RoomExcerptListing, RoomExcerptRecord,
        RoomExcerptStore, RoomExcerptTimeline, SecretDigest, SecretFactory,
        SecretGenerationFailure, SecretValue,
    },
    room_excerpts::{
        DEFAULT_RETENTION_DAYS, DeleteExcerpt, ExcerptFailureKind, ExcerptRangeProblem,
        ListExcerpts, RoomExcerptCleanupService, RoomExcerptCleanupUseCases,
        RoomExcerptDependencies, RoomExcerptService, RoomExcerptUseCases, SHARES_PER_HOUR,
        ShareExcerpt,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
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
/// 实况一小时前打开。
const ENABLED_AT: i64 = NOW - 3_600_000;
const ROOM: &str = "!club:matrix.test";
const SHARE_CODE: &str = "Club01";

#[tokio::test]
async fn 成员分享连着的几句_只记事件和第一句的时间() {
    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(6, ENABLED_AT + 1_000));

    let shared = fixture
        .service
        .share(share(member(), "$msg1", "$msg3"))
        .await
        .expect("成员能分享");

    assert_eq!(shared.message_count, 3);
    assert_eq!(shared.share_code.as_str(), SHARE_CODE);
    let stored = fixture.excerpts.stored();
    assert_eq!(stored.len(), 1);
    let record = &stored[0];
    assert_eq!(record.code, shared.code);
    assert_eq!(record.catalog_id, catalog_id());
    assert_eq!(
        record
            .event_ids
            .iter()
            .map(MatrixEventId::as_str)
            .collect::<Vec<_>>(),
        ["$msg1", "$msg2", "$msg3"]
    );
    assert_eq!(record.first_event_at, time(ENABLED_AT + 2_000));
    assert_eq!(record.shared_by, member());
    assert_eq!(record.shared_at, time(NOW));
    assert_eq!(
        *fixture.excerpts.inserted_with.lock().unwrap(),
        [time(ENABLED_AT)]
    );
    assert_eq!(*fixture.timeline.asked.lock().unwrap(), ["$msg1"]);

    let single = fixture
        .service
        .share(share(owner(), "$msg4", "$msg4"))
        .await
        .expect("一句也行");
    assert_eq!(single.message_count, 1);
}

#[tokio::test]
async fn 选的几句不对时说是哪一种_什么都不记() {
    let cases = [
        (
            messages(5, ENABLED_AT + 1_000),
            ("$msg3", "$msg1"),
            ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound),
        ),
        (
            messages(5, ENABLED_AT + 1_000),
            ("$msg1", "$unknown"),
            ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound),
        ),
        (
            messages(5, ENABLED_AT + 1_000),
            ("$unknown", "$msg1"),
            ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound),
        ),
        (
            messages(40, ENABLED_AT + 1_000),
            ("$msg0", "$msg30"),
            ExcerptFailureKind::TooLong,
        ),
        (
            messages(5, ENABLED_AT - 2_500),
            ("$msg0", "$msg3"),
            ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::BeforeLive),
        ),
        (
            ExcerptTimelineRead::NotReachable,
            ("$msg0", "$msg1"),
            ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound),
        ),
        (
            ExcerptTimelineRead::Encrypted,
            ("$msg0", "$msg1"),
            ExcerptFailureKind::NotLive,
        ),
    ];
    for (read, (first, last), expected) in cases {
        let fixture = Fixture::new(room());
        fixture.timeline.answer(read);
        let failure = fixture
            .service
            .share(share(member(), first, last))
            .await
            .expect_err("这几句分享不了");
        assert_eq!(failure.kind(), expected, "{first}..{last}");
        assert!(fixture.excerpts.stored().is_empty());
    }

    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(40, ENABLED_AT + 1_000));
    let thirty = fixture
        .service
        .share(share(member(), "$msg0", "$msg29"))
        .await
        .expect("正好 30 句可以");
    assert_eq!(thirty.message_count, 30);
}

#[tokio::test]
async fn 没在放实况_不在房间里_登录过期_读不了房间都不记() {
    let fixture = Fixture::new(room());
    *fixture.excerpts.room.lock().unwrap() = None;
    let failure = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::NotLive);
    assert!(
        fixture.timeline.asked.lock().unwrap().is_empty(),
        "不读房间"
    );

    let fixture = Fixture::new(room());
    let failure = fixture
        .service
        .share(share(stranger(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::NotFound);

    let mut expired = principal(member());
    expired.expires_at = time(NOW);
    let failure = fixture
        .service
        .share(ShareExcerpt {
            actor: expired,
            ..share(member(), "$msg0", "$msg1")
        })
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::Forbidden);

    fixture.timeline.unavailable();
    let failure = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::DependencyUnavailable);

    // 读完房间、记下来之前实况关了。
    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(3, ENABLED_AT + 1_000));
    fixture
        .excerpts
        .outcomes
        .lock()
        .unwrap()
        .push(InsertRoomExcerptOutcome::NotLive);
    let failure = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::NotLive);
    assert!(fixture.excerpts.stored().is_empty());
}

#[tokio::test]
async fn 每人每小时最多分享_20_段_选错了的也算() {
    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(3, ENABLED_AT + 1_000));

    let wrong = fixture
        .service
        .share(share(member(), "$msg2", "$msg0"))
        .await
        .unwrap_err();
    assert_eq!(
        wrong.kind(),
        ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound)
    );
    for _ in 1..SHARES_PER_HOUR {
        fixture
            .service
            .share(share(member(), "$msg0", "$msg1"))
            .await
            .expect("还没到上限");
    }
    let asked = fixture.timeline.asked.lock().unwrap().len();
    let failure = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        ExcerptFailureKind::RateLimited {
            retry_after_seconds: 3_600
        }
    );
    assert_eq!(
        fixture.timeline.asked.lock().unwrap().len(),
        asked,
        "到上限的不读房间"
    );
    assert_eq!(fixture.excerpts.stored().len(), SHARES_PER_HOUR - 1);

    fixture
        .service
        .share(share(owner(), "$msg0", "$msg1"))
        .await
        .expect("别人的名额另算");
}

#[tokio::test]
async fn 段码撞上了换一个_一直撞就算失败() {
    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(3, ENABLED_AT + 1_000));
    fixture.excerpts.outcomes.lock().unwrap().extend([
        InsertRoomExcerptOutcome::CodeTaken,
        InsertRoomExcerptOutcome::CodeTaken,
    ]);
    let shared = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .expect("第三次记下了");
    let tried = fixture.excerpts.tried_codes.lock().unwrap().clone();
    assert_eq!(tried.len(), 3);
    assert_eq!(tried[2], shared.code);
    assert_ne!(tried[0], tried[1]);
    assert_ne!(tried[1], tried[2]);

    let fixture = Fixture::new(room());
    fixture.timeline.answer(messages(3, ENABLED_AT + 1_000));
    fixture.excerpts.outcomes.lock().unwrap().extend([
        InsertRoomExcerptOutcome::CodeTaken,
        InsertRoomExcerptOutcome::CodeTaken,
        InsertRoomExcerptOutcome::CodeTaken,
    ]);
    let failure = fixture
        .service
        .share(share(member(), "$msg0", "$msg1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::Internal);
}

#[tokio::test]
async fn 成员只列自己的_房主和管理员列全部_都只说是不是自己的() {
    let fixture = Fixture::new(room());
    fixture.excerpts.seed([
        stored("Mine0001", member(), NOW - 1_000),
        stored("Theirs01", manager(), NOW - 2_000),
    ]);

    let mine = fixture.service.list(list(member())).await.unwrap();
    assert_eq!(mine.share_code.unwrap().as_str(), SHARE_CODE);
    assert_eq!(
        mine.excerpts
            .iter()
            .map(|excerpt| (excerpt.code.as_str(), excerpt.mine))
            .collect::<Vec<_>>(),
        [("Mine0001", true)]
    );
    assert_eq!(mine.excerpts[0].message_count, 2);
    assert_eq!(mine.excerpts[0].first_message_at, time(ENABLED_AT + 1_000));
    assert_eq!(mine.excerpts[0].shared_at, time(NOW - 1_000));

    for manages in [owner(), manager()] {
        let all = fixture.service.list(list(manages)).await.unwrap();
        assert_eq!(all.excerpts.len(), 2);
        assert_eq!(
            all.excerpts.iter().filter(|excerpt| excerpt.mine).count(),
            usize::from(manages == manager())
        );
    }
    assert_eq!(
        *fixture.excerpts.listed_for.lock().unwrap(),
        [Some(member()), None, None]
    );

    let failure = fixture.service.list(list(stranger())).await.unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::NotFound);
}

#[tokio::test]
async fn 分享的人离开了也能删自己的_管理员能删别人的_普通成员不能() {
    let mut left = room();
    left.leave(member()).unwrap();
    let fixture = Fixture::new(left);
    fixture.excerpts.seed([
        stored("Mine0001", member(), NOW - 1_000),
        stored("Theirs01", manager(), NOW - 2_000),
        stored("Owners01", owner(), NOW - 3_000),
    ]);

    fixture
        .service
        .delete(delete(member(), "Mine0001"))
        .await
        .expect("自己分享的，离开了也能删");
    let failure = fixture
        .service
        .delete(delete(member(), "Theirs01"))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        ExcerptFailureKind::NotFound,
        "不在房间里的人删别人的，和没有这一段一样"
    );
    fixture
        .service
        .delete(delete(manager(), "Owners01"))
        .await
        .expect("管理员能删别人的");

    let fixture = Fixture::new(room());
    fixture
        .excerpts
        .seed([stored("Theirs01", manager(), NOW - 2_000)]);
    let failure = fixture
        .service
        .delete(delete(member(), "Theirs01"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::Forbidden);
    let failure = fixture
        .service
        .delete(delete(member(), "Nothing1"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::NotFound);
    let mut expired = principal(manager());
    expired.expires_at = time(NOW);
    let failure = fixture
        .service
        .delete(DeleteExcerpt {
            actor: expired,
            ..delete(manager(), "Theirs01")
        })
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ExcerptFailureKind::Forbidden);
    assert_eq!(fixture.excerpts.stored().len(), 1, "都没删");
}

#[tokio::test]
async fn 每天清一次按现在和默认保留期() {
    let excerpts = Arc::new(MemoryExcerpts::default());
    let cleanup = RoomExcerptCleanupService::new(excerpts.clone(), Arc::new(TestClock));

    assert_eq!(cleanup.prune().await.unwrap(), 4);
    assert_eq!(
        *excerpts.pruned.lock().unwrap(),
        [(time(NOW), DEFAULT_RETENTION_DAYS)]
    );
    assert_eq!(DEFAULT_RETENTION_DAYS, 30);
}

struct Fixture {
    service: RoomExcerptService,
    excerpts: Arc<MemoryExcerpts>,
    timeline: Arc<MemoryTimeline>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let excerpts = Arc::new(MemoryExcerpts::default());
        *excerpts.room.lock().unwrap() = Some(ExcerptRoomRecord {
            matrix_room_id: MatrixRoomId::new(ROOM).unwrap(),
            enabled_at: time(ENABLED_AT),
            share_code: RoomShareCode::parse(SHARE_CODE).unwrap(),
        });
        let timeline = Arc::new(MemoryTimeline::default());
        let service = RoomExcerptService::new(RoomExcerptDependencies {
            rooms: Arc::new(MemoryRooms { room }),
            excerpts: excerpts.clone(),
            timeline: timeline.clone(),
            secrets: Arc::new(CountingSecrets::default()),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            excerpts,
            timeline,
        }
    }
}

/// 读到的 `count` 句：`$m0`、`$m1`……第一句在 `first_at`，之后每句晚一秒。
fn messages(count: usize, first_at: i64) -> ExcerptTimelineRead {
    ExcerptTimelineRead::Found(
        (0..count)
            .map(|index| ExcerptMessageRef {
                event_id: MatrixEventId::new(format!("$msg{index}")).unwrap(),
                sent_at: time(first_at + i64::try_from(index).unwrap() * 1_000),
            })
            .collect(),
    )
}

fn stored(code: &str, shared_by: PrincipalId, shared_at: i64) -> RoomExcerptRecord {
    RoomExcerptRecord {
        code: RoomExcerptCode::parse(code).unwrap(),
        catalog_id: catalog_id(),
        event_ids: vec![
            MatrixEventId::new("$aa1").unwrap(),
            MatrixEventId::new("$aa2").unwrap(),
        ],
        first_event_at: time(ENABLED_AT + 1_000),
        shared_by,
        shared_at: time(shared_at),
    }
}

fn share(actor: PrincipalId, first: &str, last: &str) -> ShareExcerpt {
    ShareExcerpt {
        actor: principal(actor),
        catalog_id: catalog_id(),
        first_event_id: MatrixEventId::new(first).unwrap(),
        last_event_id: MatrixEventId::new(last).unwrap(),
    }
}

fn list(actor: PrincipalId) -> ListExcerpts {
    ListExcerpts {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn delete(actor: PrincipalId, code: &str) -> DeleteExcerpt {
    DeleteExcerpt {
        actor: principal(actor),
        catalog_id: catalog_id(),
        code: RoomExcerptCode::parse(code).unwrap(),
    }
}

#[derive(Default)]
struct MemoryExcerpts {
    room: Mutex<Option<ExcerptRoomRecord>>,
    excerpts: Mutex<Vec<RoomExcerptRecord>>,
    /// 接下来几次记的时候怎么回答；用完了是记下。
    outcomes: Mutex<Vec<InsertRoomExcerptOutcome>>,
    tried_codes: Mutex<Vec<RoomExcerptCode>>,
    inserted_with: Mutex<Vec<UtcMillis>>,
    listed_for: Mutex<Vec<Option<PrincipalId>>>,
    pruned: Mutex<Vec<(UtcMillis, u16)>>,
}

impl MemoryExcerpts {
    fn stored(&self) -> Vec<RoomExcerptRecord> {
        self.excerpts.lock().unwrap().clone()
    }

    fn seed(&self, records: impl IntoIterator<Item = RoomExcerptRecord>) {
        self.excerpts.lock().unwrap().extend(records);
    }
}

impl RoomExcerptStore for MemoryExcerpts {
    fn excerpt_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ExcerptRoomRecord>>> {
        assert_eq!(catalog_id, self::catalog_id());
        let room = self.room.lock().unwrap().clone();
        Box::pin(async move { Ok(room) })
    }

    fn insert_excerpt<'a>(
        &'a self,
        record: &'a RoomExcerptRecord,
        enabled_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<InsertRoomExcerptOutcome>> {
        self.tried_codes.lock().unwrap().push(record.code.clone());
        self.inserted_with.lock().unwrap().push(enabled_at);
        let mut outcomes = self.outcomes.lock().unwrap();
        let outcome = if outcomes.is_empty() {
            self.excerpts.lock().unwrap().push(record.clone());
            InsertRoomExcerptOutcome::Inserted
        } else {
            outcomes.remove(0)
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn room_excerpts(
        &self,
        catalog_id: RoomCatalogId,
        shared_by: Option<PrincipalId>,
    ) -> PortFuture<'_, RepositoryResult<RoomExcerptListing>> {
        assert_eq!(catalog_id, self::catalog_id());
        self.listed_for.lock().unwrap().push(shared_by);
        let excerpts = self
            .excerpts
            .lock()
            .unwrap()
            .iter()
            .filter(|excerpt| shared_by.is_none_or(|sharer| excerpt.shared_by == sharer))
            .cloned()
            .collect();
        Box::pin(async move {
            Ok(RoomExcerptListing {
                share_code: Some(RoomShareCode::parse(SHARE_CODE).unwrap()),
                excerpts,
            })
        })
    }

    fn find_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomExcerptRecord>>> {
        let found = self
            .excerpts
            .lock()
            .unwrap()
            .iter()
            .find(|excerpt| excerpt.catalog_id == catalog_id && excerpt.code == *code)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn delete_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        let mut excerpts = self.excerpts.lock().unwrap();
        let before = excerpts.len();
        excerpts.retain(|excerpt| !(excerpt.catalog_id == catalog_id && excerpt.code == *code));
        let deleted = excerpts.len() < before;
        Box::pin(async move { Ok(deleted) })
    }

    fn prune_excerpts(
        &self,
        now: UtcMillis,
        default_retention_days: u16,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        self.pruned
            .lock()
            .unwrap()
            .push((now, default_retention_days));
        Box::pin(async { Ok(4) })
    }
}

#[derive(Default)]
struct MemoryTimeline {
    read: Mutex<Option<Result<ExcerptTimelineRead, ExcerptTimelineUnavailable>>>,
    asked: Mutex<Vec<String>>,
}

impl MemoryTimeline {
    fn answer(&self, read: ExcerptTimelineRead) {
        *self.read.lock().unwrap() = Some(Ok(read));
    }

    fn unavailable(&self) {
        *self.read.lock().unwrap() = Some(Err(ExcerptTimelineUnavailable));
    }
}

impl RoomExcerptTimeline for MemoryTimeline {
    fn visible_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<ExcerptTimelineRead, ExcerptTimelineUnavailable>> {
        assert_eq!(room_id.as_str(), ROOM);
        self.asked
            .lock()
            .unwrap()
            .push(first_event_id.as_str().to_owned());
        let read = self
            .read
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(ExcerptTimelineRead::NotReachable));
        Box::pin(async move { read })
    }
}

#[derive(Default)]
struct CountingSecrets(Mutex<u32>);

impl SecretFactory for CountingSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("secret-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (index, byte) in value.bytes().enumerate() {
            digest[index % 32] ^= byte.rotate_left(u32::try_from(index % 8).unwrap());
        }
        digest[0] ^= u8::try_from(value.len() % 256).unwrap();
        SecretDigest::from_array(digest)
    }
}

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
            status: RoomCatalogStatus::Active,
        },
    )
    .unwrap();
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(20)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new(ROOM.to_owned()).unwrap(),
            region: None,
            capacity: RoomCapacity::standard(),
            allocated_slots: 0,
            state: RoomInstanceState::Active,
        },
    )
    .unwrap();
    PrivateRoomSnapshot::new(catalog, instance, room).unwrap()
}

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: format!("@user-{}:matrix.test", principal_id.as_uuid()),
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

struct MemoryRooms {
    room: PrivateRoom,
}

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("分享一段不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = (catalog == catalog_id()).then(|| snapshot(self.room.clone()));
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("分享一段不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("分享一段按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("分享一段不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("分享一段不改名") })
    }
}
