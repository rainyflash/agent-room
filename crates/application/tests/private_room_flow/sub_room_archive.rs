//! 子房间第 2e-2 步（`specs/redesign/sub-rooms.md` 的“归档和恢复”）：管得了这一间的人把它收起来、
//! 拿回来。归档以后只能看、说不了，名字和名额让出来，在等的申请作废；恢复以后原来的人接着说，被禁言
//! 着的照旧说不了。聊天服务器上锁住、解开排进子房间的任务表，请求结束前先做一遍。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份，存储的规则照真实存储的写；真实存储自己的测试在
//! `postgres-adapter` 的 `tests/private_rooms/sub_room_archive.rs`。

use agent_room_application::{
    ports::{SubRoomArchival, SubRoomArchiveOutcome, SubRoomArchiveStore, SubRoomRestoration},
    private_rooms::{ArchiveSubRoom, RequestSubRoomJoin, RestoreSubRoom},
};
use agent_room_domain::room_join_requests::JoinRequestStatus;

use super::{
    membership::{refusal, sub_room},
    sync::{agent_in, room},
    *,
};

impl MemoryRooms {
    /// 父房间在用、这一间挂在它下面时，它的父子关系。
    fn usable_link(&self, catalog: RoomCatalogId, parent: RoomCatalogId) -> Option<StoredLink> {
        let parent_active = self
            .rooms
            .lock()
            .expect("房间锁正常")
            .get(&parent)
            .is_some_and(|snapshot| snapshot.room().status() == PrivateRoomLifecycleStatus::Active);
        if !parent_active {
            return None;
        }
        self.links
            .lock()
            .expect("关系锁正常")
            .iter()
            .find(|stored| {
                stored.link.catalog_id == catalog && stored.link.parent_catalog_id == parent
            })
            .cloned()
    }

    /// 读到以后又被存过的，和真实存储一样报冲突。
    fn same_version(&self, room: &PrivateRoom, expected: AggregateVersion) -> RepositoryResult<()> {
        let current = self.rooms.lock().expect("房间锁正常")[&room.catalog_id()]
            .room()
            .version();
        if current == expected {
            Ok(())
        } else {
            Err(repository_error(
                "test.sub_room.archive",
                RepositoryErrorKind::Conflict,
            ))
        }
    }

    fn set_archive_record(&self, catalog: RoomCatalogId, record: Option<SubRoomArchiveRecord>) {
        for stored in self.links.lock().expect("关系锁正常").iter_mut() {
            if stored.link.catalog_id == catalog {
                stored.mark_archived(record);
            }
        }
    }

    /// 摆出别的原因归档的样子（自动归档、跟着父房间归档）。
    fn archived_because(&self, catalog: u128, reason: SubRoomArchiveReason) {
        self.set_archive_record(catalog_id(catalog), Some(archive_record(reason, None)));
    }
}

impl SubRoomArchiveStore for MemoryRooms {
    fn archive_sub_room<'a>(
        &'a self,
        archival: &'a SubRoomArchival<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>> {
        Box::pin(async move {
            self.record("sub_rooms.archive");
            let catalog = archival.room.catalog_id();
            let usable = self
                .usable_link(catalog, archival.parent_catalog_id)
                .is_some_and(|stored| !stored.archived);
            if !usable {
                return Ok(SubRoomArchiveOutcome::Unavailable);
            }
            self.same_version(archival.room, archival.expected_version)?;
            self.replace(archival.room);
            self.set_archive_record(
                catalog,
                Some(SubRoomArchiveRecord {
                    at: archival.at,
                    by: archival.by,
                    reason: archival.reason,
                }),
            );
            self.void_requests(catalog, archival.at);
            self.queue_sync(catalog, None);
            Ok(SubRoomArchiveOutcome::Done)
        })
    }

    fn restore_sub_room<'a>(
        &'a self,
        restoration: &'a SubRoomRestoration<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>> {
        Box::pin(async move {
            self.record("sub_rooms.restore");
            let catalog = restoration.room.catalog_id();
            let parent = restoration.parent_catalog_id;
            let Some(stored) = self.usable_link(catalog, parent) else {
                return Ok(SubRoomArchiveOutcome::Unavailable);
            };
            let with_parent = stored
                .link
                .archived
                .is_some_and(|record| record.reason == SubRoomArchiveReason::Parent);
            if !stored.archived || with_parent {
                return Ok(SubRoomArchiveOutcome::Unavailable);
            }
            let open: Vec<StoredLink> = self
                .links
                .lock()
                .expect("关系锁正常")
                .iter()
                .filter(|other| {
                    other.link.parent_catalog_id == parent
                        && other.link.catalog_id != catalog
                        && !other.archived
                })
                .cloned()
                .collect();
            if open
                .iter()
                .any(|other| other.link.name_key == stored.link.name_key)
            {
                return Ok(SubRoomArchiveOutcome::NameTaken);
            }
            if open.len() >= usize::from(MAXIMUM_OPEN_SUB_ROOMS) {
                return Ok(SubRoomArchiveOutcome::LimitReached);
            }
            self.same_version(restoration.room, restoration.expected_version)?;
            self.replace(restoration.room);
            self.set_archive_record(catalog, None);
            self.queue_sync(catalog, None);
            Ok(SubRoomArchiveOutcome::Done)
        })
    }
}

impl Rooms {
    async fn archive_sub_room(
        &self,
        actor: u128,
        catalog: u128,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .archive_sub_room(ArchiveSubRoom {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
            })
            .await
    }

    async fn restore_sub_room(
        &self,
        actor: u128,
        catalog: u128,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .restore_sub_room(RestoreSubRoom {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
            })
            .await
    }

    fn status_of(&self, catalog: u128) -> PrivateRoomLifecycleStatus {
        self.store.room(catalog).status()
    }
}

/// 20 号子房间：成员进来了、是这一间的管理员，只能看的人也进来了。
async fn release_managed_by_member() -> Rooms {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.join(MEMBER, 20).await.expect("成员能进");
    rooms
        .set_manager(OWNER, MEMBER, true)
        .await
        .expect("房主能设管理员");
    rooms.join(VIEWER, 20).await.expect("只能看的也能进");
    rooms.matrix.clear_changes();
    rooms.clear_events();
    rooms
}

#[tokio::test]
async fn 这一间的管理员归档_只能看说不了_也进不来了_记下谁为什么() {
    let rooms = release_managed_by_member().await;

    assert_eq!(
        refusal(rooms.archive_sub_room(VIEWER, 20).await),
        sub_room(SubRoomRefusal::ManagersOnly),
        "在里面但管不了这一间的人归档不了"
    );
    let archived = rooms
        .archive_sub_room(MEMBER, 20)
        .await
        .expect("这一间的管理员能归档");

    assert_eq!(
        archived.room().status(),
        PrivateRoomLifecycleStatus::Archived
    );
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Archived);
    assert!(
        rooms.store.room(20).members().count() >= 3,
        "成员原样留着，恢复以后接着说"
    );
    assert_eq!(
        rooms.store.link(20).link.archived,
        Some(SubRoomArchiveRecord {
            at: UtcMillis::new(1_000).expect("时间有效"),
            by: Some(principal_id(MEMBER)),
            reason: SubRoomArchiveReason::Manual,
        })
    );
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["archive"],
        "请求里就在聊天服务器上锁住了"
    );
    assert!(rooms.store.queued_sync().is_empty(), "做完的不留着");
    assert_eq!(
        refusal(rooms.join(ADMIN, 20).await),
        sub_room(SubRoomRefusal::Archived),
        "归档了的谁都进不来"
    );
    assert_eq!(
        refusal(rooms.revise(MEMBER, 20, Some("release-2"), None).await),
        sub_room(SubRoomRefusal::Archived)
    );

    // 再归档一次：原样回，不再存，也不再锁。
    rooms.matrix.clear_changes();
    rooms.clear_events();
    rooms
        .archive_sub_room(MEMBER, 20)
        .await
        .expect("归档过的再归档也算成功");
    assert!(!rooms.events().contains(&"sub_rooms.archive"));
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
}

#[tokio::test]
async fn 父房间的房主和管理员没进来也能归档_名字和名额让出来() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));

    assert_eq!(
        refusal(rooms.archive_sub_room(OUTSIDER, 20).await),
        PrivateRoomFailureKind::NotFound,
        "不在房间里的人找不到这一间"
    );
    rooms
        .archive_sub_room(ADMIN, 20)
        .await
        .expect("父房间的管理员没进来也能归档");
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Archived);

    rooms
        .create(OWNER, 21, "Release")
        .await
        .expect("归档了的把名字让出来");
}

#[tokio::test]
async fn 归档时这一间在等的申请作废() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(20, "security")
    });
    rooms
        .service
        .request_sub_room_join(RequestSubRoomJoin {
            actor: person(MEMBER),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(20),
            message: String::new(),
        })
        .await
        .expect("能申请");

    rooms.archive_sub_room(OWNER, 20).await.expect("房主能归档");

    let stored = rooms.store.requests_of(20);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, JoinRequestStatus::Voided);
    assert_eq!(stored[0].decided_by, None, "不是谁拒了他");
}

#[tokio::test]
async fn 恢复以后原来的人和他们的_agent_接着说_被禁言着的照旧说不了() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.join(MEMBER, 20).await.expect("成员能进");
    rooms.join(ADMIN, 20).await.expect("管理员能进");
    rooms.join(VIEWER, 20).await.expect("只能看的也能进");
    agent_in(&rooms, MEMBER, "member-agent", &[PARENT, 20]);
    rooms.archive_sub_room(OWNER, 20).await.expect("房主能归档");
    // 归档这段时间里管理员在父房间里被禁言了：聊天服务器上他还是归档以前能说的级别。
    rooms.store.mute(PARENT, ADMIN);
    rooms.matrix.clear_changes();

    let restored = rooms
        .restore_sub_room(OWNER, 20)
        .await
        .expect("能归档的人能恢复");

    assert_eq!(restored.room().status(), PrivateRoomLifecycleStatus::Active);
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Active);
    assert_eq!(rooms.store.link(20).link.archived, None);
    let changes = rooms.matrix.changes_in(&room(20));
    assert_eq!(
        changes.len(),
        1,
        "门槛和每个人的级别同一次写回去：{changes:?}"
    );
    assert!(changes[0].starts_with("reopen "));
    for (user, speaks) in [
        ("@owner:matrix.test", true),
        ("@member:matrix.test", true),
        ("@member-agent:matrix.test", true),
        ("@admin:matrix.test", false),
        ("@viewer:matrix.test", false),
    ] {
        assert_eq!(rooms.matrix.speaking_allowed(user), Some(speaks), "{user}");
    }
    assert!(rooms.store.queued_sync().is_empty());
    rooms
        .revise(OWNER, 20, Some("release-2"), None)
        .await
        .expect("恢复以后照常能改名");
}

#[tokio::test]
async fn 恢复时撞了名字_满了_都恢复不了_什么都不动() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.archive_sub_room(OWNER, 20).await.expect("房主能归档");
    rooms
        .create(OWNER, 21, "Release")
        .await
        .expect("名字让出来了");
    rooms.matrix.clear_changes();

    assert_eq!(
        refusal(rooms.restore_sub_room(OWNER, 20).await),
        sub_room(SubRoomRefusal::NameTaken),
        "先给同名的那一间改个名"
    );
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Archived);

    rooms
        .revise(OWNER, 21, Some("release-new"), None)
        .await
        .expect("给另一间改名");
    for catalog in 100..199 {
        rooms
            .store
            .seed(&Seed::open(catalog, &format!("room-{catalog}")));
    }
    assert_eq!(
        refusal(rooms.restore_sub_room(OWNER, 20).await),
        sub_room(SubRoomRefusal::LimitReached),
        "没归档的已经满一百间"
    );
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Archived);
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
    assert!(rooms.store.queued_sync().is_empty());
}

#[tokio::test]
async fn 手动归档的只有管得了的人能恢复_自动归档的原来在里面的人也能() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.join(MEMBER, 20).await.expect("成员能进");
    rooms.archive_sub_room(OWNER, 20).await.expect("房主能归档");

    assert_eq!(
        refusal(rooms.restore_sub_room(MEMBER, 20).await),
        sub_room(SubRoomRefusal::ManagersOnly),
        "有人决定收起来的，管不了这一间的人拿不回来"
    );

    rooms
        .store
        .archived_because(20, SubRoomArchiveReason::Inactive);
    assert_eq!(
        refusal(rooms.restore_sub_room(VIEWER, 20).await),
        sub_room(SubRoomRefusal::ManagersOnly),
        "没进过的拿不回来"
    );
    rooms
        .restore_sub_room(MEMBER, 20)
        .await
        .expect("30 天没人说话自动收起来的，原来在里面的人能恢复");
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Active);
}

#[tokio::test]
async fn 跟着父房间归档的回不来_父房间归档了的收不了也拿不回来() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(20, "old")
    });
    rooms.store.seed(&Seed::open(21, "release"));
    rooms
        .store
        .archived_because(20, SubRoomArchiveReason::Parent);

    assert_eq!(
        refusal(rooms.restore_sub_room(OWNER, 20).await),
        sub_room(SubRoomRefusal::Archived)
    );

    rooms
        .service
        .archive(ArchivePrivateRoom {
            actor: person(OWNER).into(),
            catalog_id: catalog_id(PARENT),
        })
        .await
        .expect("房主能归档整间房");
    assert_eq!(rooms.status_of(21), PrivateRoomLifecycleStatus::Archived);
    assert_eq!(
        refusal(rooms.restore_sub_room(OWNER, 21).await),
        sub_room(SubRoomRefusal::Archived),
        "父房间不能恢复，下面的子房间也回不来"
    );
    assert_eq!(
        refusal(rooms.archive_sub_room(OWNER, 21).await),
        sub_room(SubRoomRefusal::Archived)
    );
}

#[tokio::test]
async fn 聊天服务器上没锁成_请求照样成功_排着的事后台接着做_恢复也一样() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms
        .matrix
        .fail_in(&room(20), MatrixFailureKind::DependencyUnavailable);

    rooms
        .archive_sub_room(OWNER, 20)
        .await
        .expect("库里归档了就算成功");
    assert_eq!(rooms.status_of(20), PrivateRoomLifecycleStatus::Archived);
    let waiting = rooms.store.queued_sync();
    assert_eq!(waiting.len(), 1);
    assert_eq!((waiting[0].principal, waiting[0].attempts), (None, 1));
    assert_eq!(waiting[0].failure_code, Some("matrix.unavailable"));

    rooms.matrix.recover(&room(20));
    rooms.store.let_sync_come_due();
    assert_eq!(rooms.sync().await.done, 1);
    assert_eq!(rooms.matrix.changes_in(&room(20)), ["archive"]);

    rooms
        .matrix
        .fail_in(&room(20), MatrixFailureKind::DependencyUnavailable);
    rooms
        .restore_sub_room(OWNER, 20)
        .await
        .expect("库里恢复了就算成功");
    assert_eq!(rooms.store.queued_sync().len(), 1);
    rooms.matrix.recover(&room(20));
    rooms.store.let_sync_come_due();
    assert_eq!(rooms.sync().await.done, 1);
    let changes = rooms.matrix.changes_in(&room(20));
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes[1].starts_with("reopen "));
}

#[tokio::test]
async fn 恢复的事还排着时又归档了_照做的时候的样子锁住() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.archive_sub_room(OWNER, 20).await.expect("房主能归档");
    rooms.restore_sub_room(OWNER, 20).await.expect("房主能恢复");
    // 恢复的那件事排着还没做，这当口又归档了：这一件照现在的样子锁住。
    rooms.store.queue_sync(catalog_id(20), None);
    rooms.store.archive(20);
    rooms.matrix.clear_changes();

    assert_eq!(rooms.sync().await.done, 1);
    assert_eq!(rooms.matrix.changes_in(&room(20)), ["archive"]);
}
