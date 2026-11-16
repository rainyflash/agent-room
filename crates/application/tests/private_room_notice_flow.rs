//! 房主和管理员改房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：查“管理”这一位、
//! 房间在使用中，以应用服务写进 Matrix，写的是改的人和改的时间；空的是撤掉；太长的不收；
//! Matrix 写不进就报错；每个人一小时有上限。子房间里父房间须知的副本在 `parent_copies.rs`。

#[path = "private_room_notice_flow/parent_copies.rs"]
mod parent_copies;

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        ParentNoticeCopy, ParentNoticeGateway, ParentNoticeSyncStore, ParentNoticeSyncTask,
        PortFuture, PrivateRoomNoticeGateway, PrivateRoomSnapshot, PrivateRoomStore,
        RoomNoticeWrite, SubRoomSyncScope,
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
    homeserver: Arc<Recorder>,
    queue: Arc<MemoryQueue>,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        Self::with_rooms(vec![snapshot(room)])
    }

    /// 几间房：子房间的快照上记着挂在哪间房下面，排副本时照它找。
    fn with_rooms(rooms: Vec<PrivateRoomSnapshot>) -> Self {
        let homeserver = Arc::new(Recorder::default());
        let queue = Arc::new(MemoryQueue::new(&rooms));
        let clock = Arc::new(TestClock(Mutex::new(NOW)));
        let service = PrivateRoomNoticeService::new(PrivateRoomNoticeDependencies {
            rooms: Arc::new(MemoryRooms(rooms)),
            matrix: homeserver.clone(),
            copy_queue: queue.clone(),
            copies: homeserver.clone(),
            clock: clock.clone(),
        });
        Self {
            service,
            writes: homeserver.writes.clone(),
            matrix_fails: homeserver.matrix_fails.clone(),
            homeserver,
            queue,
            clock,
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
    snapshot_in(room, "!club:matrix.test")
}

/// 一间房的快照，`matrix_room` 是它在 Matrix 上的房间号。
fn snapshot_in(room: PrivateRoom, matrix_room: &str) -> PrivateRoomSnapshot {
    let archived = room.status().as_str() == "archived";
    let catalog_id = room.catalog_id();
    let catalog = RoomCatalog::new(
        catalog_id,
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
        RoomInstanceId::from_uuid(Uuid::from_u128(catalog_id.as_uuid().as_u128() + 100)),
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: MatrixRoomReference::new(matrix_room.to_owned()).unwrap(),
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

/// 从 `NOW` 起，测试能往后拨。
struct TestClock(Mutex<i64>);

impl TestClock {
    fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(*self.0.lock().unwrap())
    }
}

/// 假的聊天服务器。记下写进 Matrix 的每一版须知（房间、正文、改的人、时间），各房间此刻的须知和子房间
/// 里的副本；`copy_failures` 里的房间读写副本时照那一种失败。
#[derive(Default)]
struct Recorder {
    writes: Arc<Mutex<Vec<String>>>,
    matrix_fails: Arc<Mutex<bool>>,
    notices: Mutex<HashMap<String, RoomNoticeWrite>>,
    copies: Mutex<HashMap<String, ParentNoticeCopy>>,
    copy_failures: Mutex<HashMap<String, MatrixFailureKind>>,
    /// 读写副本那几样一共被叫了几次。
    copy_calls: Mutex<usize>,
}

impl Recorder {
    fn copy_of(&self, room: &str) -> Option<ParentNoticeCopy> {
        self.copies.lock().unwrap().get(room).cloned()
    }

    fn fail_copies_in(&self, room: &str, kind: MatrixFailureKind) {
        self.copy_failures
            .lock()
            .unwrap()
            .insert(room.to_owned(), kind);
    }

    /// 记一次调用；这间房设了失败就照它失败。
    fn copy_call(&self, room: &MatrixRoomId, operation: MatrixOperation) -> MatrixResult<()> {
        *self.copy_calls.lock().unwrap() += 1;
        match self.copy_failures.lock().unwrap().get(room.as_str()) {
            Some(kind) => Err(MatrixFailure::new(operation, *kind)),
            None => Ok(()),
        }
    }
}

impl ParentNoticeGateway for Recorder {
    fn room_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<RoomNoticeWrite>>> {
        let result = self
            .copy_call(room_id, MatrixOperation::ReadRoomState)
            .map(|()| self.notices.lock().unwrap().get(room_id.as_str()).cloned());
        Box::pin(async move { result })
    }

    fn parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<ParentNoticeCopy>>> {
        let result = self
            .copy_call(room_id, MatrixOperation::ReadRoomState)
            .map(|()| self.copy_of(room_id.as_str()));
        Box::pin(async move { result })
    }

    fn set_parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        copy: &'a ParentNoticeCopy,
    ) -> PortFuture<'a, MatrixResult<()>> {
        let result = self
            .copy_call(room_id, MatrixOperation::SendStateEvent)
            .map(|()| {
                self.copies
                    .lock()
                    .unwrap()
                    .insert(room_id.as_str().to_owned(), copy.clone());
            });
        Box::pin(async move { result })
    }
}

/// 排着的一间：和 `private_room_parent_notice_sync` 的一行一样。
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuedCopy {
    generation: i64,
    attempts: u32,
    next_attempt_at: Option<i64>,
    failure_code: Option<&'static str>,
}

/// 内存里的副本待办，照存储的规矩领、做完、往后排。
struct MemoryQueue {
    /// 每间子房间挂在哪间房下面。
    parents: HashMap<RoomCatalogId, RoomCatalogId>,
    rows: Mutex<BTreeMap<Uuid, QueuedCopy>>,
    /// 为哪几间父房间排过。
    queued_under: Mutex<Vec<RoomCatalogId>>,
}

impl MemoryQueue {
    fn new(rooms: &[PrivateRoomSnapshot]) -> Self {
        Self {
            parents: rooms
                .iter()
                .filter_map(|room| Some((room.catalog().id(), room.parent_catalog_id()?)))
                .collect(),
            rows: Mutex::new(BTreeMap::new()),
            queued_under: Mutex::new(Vec::new()),
        }
    }

    /// 排一间（已经排着的重排）；新建子房间时存储在建房的事务里就是这样排的。
    fn queue(&self, sub_room: RoomCatalogId) {
        let mut rows = self.rows.lock().unwrap();
        let generation = rows
            .get(&sub_room.as_uuid())
            .map_or(1, |row| row.generation + 1);
        rows.insert(
            sub_room.as_uuid(),
            QueuedCopy {
                generation,
                attempts: 0,
                next_attempt_at: None,
                failure_code: None,
            },
        );
    }

    fn row(&self, sub_room: RoomCatalogId) -> Option<QueuedCopy> {
        self.rows.lock().unwrap().get(&sub_room.as_uuid()).cloned()
    }

    fn is_empty(&self) -> bool {
        self.rows.lock().unwrap().is_empty()
    }

    /// 领到的那一版已经不在了：子房间还在就再排一次。
    fn queue_again(&self, task: ParentNoticeSyncTask) {
        if self.parents.contains_key(&task.catalog_id) {
            self.queue(task.catalog_id);
        }
    }
}

impl ParentNoticeSyncStore for MemoryQueue {
    fn queue_parent_notice_sync(
        &self,
        parent: RoomCatalogId,
        _queued_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        self.queued_under.lock().unwrap().push(parent);
        let under: Vec<_> = self
            .parents
            .iter()
            .filter(|(_, above)| **above == parent)
            .map(|(sub_room, _)| *sub_room)
            .collect();
        for sub_room in &under {
            self.queue(*sub_room);
        }
        let queued = u64::try_from(under.len()).unwrap();
        Box::pin(async move { Ok(queued) })
    }

    fn claim_parent_notice_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ParentNoticeSyncTask>>> {
        let mut rows = self.rows.lock().unwrap();
        let mut claimed = Vec::new();
        for (id, row) in rows.iter_mut() {
            let catalog_id = RoomCatalogId::from_uuid(*id);
            let in_scope = match scope {
                SubRoomSyncScope::Everywhere => true,
                SubRoomSyncScope::Under(parent) => self.parents.get(&catalog_id) == Some(&parent),
            };
            let due = row.next_attempt_at.is_none_or(|at| at <= now.value());
            if !in_scope || !due || claimed.len() >= usize::from(limit) {
                continue;
            }
            row.attempts += 1;
            row.next_attempt_at = Some(lease_until.value());
            claimed.push(ParentNoticeSyncTask {
                catalog_id,
                generation: row.generation,
                attempt: row.attempts,
            });
        }
        Box::pin(async move { Ok(claimed) })
    }

    fn complete_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let removed = {
            let mut rows = self.rows.lock().unwrap();
            let current = rows
                .get(&task.catalog_id.as_uuid())
                .is_some_and(|row| row.generation == task.generation);
            current && rows.remove(&task.catalog_id.as_uuid()).is_some()
        };
        if !removed {
            self.queue_again(task);
        }
        Box::pin(async { Ok(()) })
    }

    fn retry_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let postponed = {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&task.catalog_id.as_uuid()) {
                Some(row) if row.generation == task.generation => {
                    row.next_attempt_at = Some(retry_at.value());
                    row.failure_code = Some(failure_code);
                    true
                }
                _ => false,
            }
        };
        if !postponed {
            self.queue_again(task);
        }
        Box::pin(async { Ok(()) })
    }
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
            self.notices
                .lock()
                .unwrap()
                .insert(room_id.as_str().to_owned(), notice.clone());
            Ok(())
        };
        Box::pin(async move { result })
    }
}

struct MemoryRooms(Vec<PrivateRoomSnapshot>);

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
        let found = self
            .0
            .iter()
            .find(|room| room.catalog().id() == catalog_id)
            .cloned();
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
