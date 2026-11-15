//! 子房间归档和恢复的存储（`specs/redesign/sub-rooms.md` 第 2e-2 步）：房间的状态和父子关系上的记录
//! 一起写、恢复时一起清；归档时这一间在等的申请作废，聊天服务器上锁住、解开的事排上；恢复时名字和
//! 个数在锁着父房间的事务里数，撞了、满了什么都不写；父房间归档了收不了也拿不回来；审计只记两个房间号
//! 和原因。

use agent_room_application::ports::{
    SubRoomArchival, SubRoomArchiveOutcome, SubRoomArchiveRecord, SubRoomArchiveStore,
    SubRoomJoinRequestOutcome, SubRoomRestoration,
};
use agent_room_domain::{
    private_rooms::PrivateRoomLifecycleStatus,
    sub_room_archive::{SubRoomArchiveReason, archived_sub_room, restored_sub_room},
};
use serde_json::{Value, json};

use super::{
    follow::queued,
    join_requests::{guarded, request, statuses},
    *,
};

async fn archive_now(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    sub_room: RoomCatalogId,
    by: PrincipalId,
    at: UtcMillis,
) -> SubRoomArchiveOutcome {
    let before = load(repositories, sub_room).await;
    let archived = archived_sub_room(before.room())
        .expect("版本够用")
        .expect("在用的能归档");
    SubRoomArchiveStore::archive_sub_room(
        repositories,
        &SubRoomArchival {
            room: &archived,
            expected_version: before.room().version(),
            parent_catalog_id: parent,
            by: Some(by),
            reason: SubRoomArchiveReason::Manual,
            at,
        },
    )
    .await
    .expect("写入不报错")
}

async fn restore_now(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    sub_room: RoomCatalogId,
    by: PrincipalId,
    at: UtcMillis,
) -> SubRoomArchiveOutcome {
    let before = load(repositories, sub_room).await;
    let restored = restored_sub_room(before.room())
        .expect("版本够用")
        .expect("归档的能恢复");
    SubRoomArchiveStore::restore_sub_room(
        repositories,
        &SubRoomRestoration {
            room: &restored,
            expected_version: before.room().version(),
            parent_catalog_id: parent,
            by,
            at,
        },
    )
    .await
    .expect("写入不报错")
}

fn status(snapshot: &PrivateRoomSnapshot) -> PrivateRoomLifecycleStatus {
    snapshot.room().status()
}

/// 这间子房间的审计：谁、动作和附带的记录。
async fn audits(pool: &PgPool, sub_room: RoomCatalogId) -> Vec<(String, String, Value)> {
    sqlx::query_as(
        r"SELECT actor_kind, action, metadata
            FROM agent_room.audit_event
           WHERE target_kind = 'sub_room' AND target_reference = $1
           ORDER BY occurred_at, id",
    )
    .bind(sub_room.to_string())
    .fetch_all(pool)
    .await
    .expect("读审计")
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 归档_状态和记录一起写_在等的申请作废_锁住的事排上_审计只记号_归档过的不再写() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-archive-owner").await;
    let member = seed_principal(&database.runtime, "sub-room-archive-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    let door = (RoomJoinMode::Request, false);
    let sub_room = guarded(&repositories, parent, owner, "Release", door, time(10)).await;
    assert_eq!(
        request(&repositories, parent, sub_room, member, time(15)).await,
        SubRoomJoinRequestOutcome::Created
    );

    // 读到以后又被存过的：报冲突，什么都不写。
    let before = load(&repositories, sub_room).await;
    let mut stale = archived_sub_room(before.room())
        .expect("版本够用")
        .expect("能归档");
    stale = restored_sub_room(&stale)
        .expect("版本够用")
        .and_then(|room| archived_sub_room(&room).expect("版本够用"))
        .expect("版本往前走了两步");
    let conflicted = SubRoomArchiveStore::archive_sub_room(
        &repositories,
        &SubRoomArchival {
            room: &stale,
            expected_version: stale.version(),
            parent_catalog_id: parent,
            by: Some(owner),
            reason: SubRoomArchiveReason::Manual,
            at: time(20),
        },
    )
    .await
    .expect_err("版本对不上");
    assert_eq!(conflicted.kind(), RepositoryErrorKind::Conflict);
    assert_eq!(
        status(&load(&repositories, sub_room).await),
        PrivateRoomLifecycleStatus::Active
    );

    assert_eq!(
        archive_now(&repositories, parent, sub_room, owner, time(20)).await,
        SubRoomArchiveOutcome::Done
    );
    let archived = load(&repositories, sub_room).await;
    assert_eq!(status(&archived), PrivateRoomLifecycleStatus::Archived);
    assert!(archived.room().member(owner).is_some(), "成员原样留着");
    let record = Some(SubRoomArchiveRecord {
        at: time(20),
        by: Some(owner),
        reason: SubRoomArchiveReason::Manual,
    });
    assert_eq!(
        link(&repositories, sub_room)
            .await
            .expect("还挂在父房间下面")
            .archived,
        record
    );
    assert_eq!(
        statuses(&database.runtime, sub_room, member).await,
        ["voided"]
    );
    assert!(
        queued(&database.runtime, sub_room)
            .await
            .contains_key(&None)
    );
    let listed = &list(&repositories, parent, owner).await[0];
    assert!(listed.archived, "名单上标着归档了");
    assert_eq!(
        listed.archive_record, record,
        "名单上带着谁、什么时候、为什么（2e-3）"
    );
    let expected_audit = (
        "principal".to_owned(),
        "sub_room.archived".to_owned(),
        json!({ "parentCatalogId": parent.to_string(), "reason": "manual" }),
    );
    assert_eq!(
        audits(&database.runtime, sub_room).await,
        std::slice::from_ref(&expected_audit)
    );

    // 归档过的再来一次（读到的还是在用的那一版）：不再写。
    let again = SubRoomArchiveStore::archive_sub_room(
        &repositories,
        &SubRoomArchival {
            room: archived.room(),
            expected_version: before.room().version(),
            parent_catalog_id: parent,
            by: Some(owner),
            reason: SubRoomArchiveReason::Manual,
            at: time(30),
        },
    )
    .await
    .expect("写入不报错");
    assert_eq!(again, SubRoomArchiveOutcome::Unavailable);
    assert_eq!(audits(&database.runtime, sub_room).await, [expected_audit]);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 恢复_清掉记录_撞了名字不写_父房间归档了收不了也拿不回来() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-restore-owner").await;
    let parent = create_room(&repositories, owner).await;
    let release = draft(&repositories, parent, owner, "Release", time(10)).await;
    assert_eq!(create(&repositories, &release).await, None);
    let sub_room = release.snapshot.catalog().id();
    assert_eq!(
        archive_now(&repositories, parent, sub_room, owner, time(20)).await,
        SubRoomArchiveOutcome::Done
    );

    // 名字让出来了：另一间叫同一个名字的建得了，这一间就恢复不了。
    let same_name = draft(&repositories, parent, owner, "RELEASE", time(25)).await;
    assert_eq!(create(&repositories, &same_name).await, None);
    assert_eq!(
        restore_now(&repositories, parent, sub_room, owner, time(30)).await,
        SubRoomArchiveOutcome::NameTaken
    );
    assert_eq!(
        status(&load(&repositories, sub_room).await),
        PrivateRoomLifecycleStatus::Archived
    );

    // 把同名的那一间收起来以后就恢复得了：状态、记录一起清，解开的事排上。
    let other = same_name.snapshot.catalog().id();
    assert_eq!(
        archive_now(&repositories, parent, other, owner, time(35)).await,
        SubRoomArchiveOutcome::Done
    );
    assert_eq!(
        restore_now(&repositories, parent, sub_room, owner, time(40)).await,
        SubRoomArchiveOutcome::Done
    );
    assert_eq!(
        status(&load(&repositories, sub_room).await),
        PrivateRoomLifecycleStatus::Active
    );
    assert_eq!(
        link(&repositories, sub_room)
            .await
            .expect("还挂着")
            .archived,
        None
    );
    assert_eq!(
        list(&repositories, parent, owner).await[0].archive_record,
        None
    );
    assert!(
        queued(&database.runtime, sub_room)
            .await
            .contains_key(&None)
    );
    let actions: Vec<(String, Value)> = audits(&database.runtime, sub_room)
        .await
        .into_iter()
        .map(|(_, action, metadata)| (action, metadata))
        .collect();
    assert_eq!(
        actions[1],
        (
            "sub_room.restored".to_owned(),
            json!({ "parentCatalogId": parent.to_string() })
        )
    );

    // 父房间归档了：没归档的那一间跟着归档，哪一间都收不了、拿不回来。
    archive(&repositories, parent, owner).await;
    assert_eq!(
        status(&load(&repositories, sub_room).await),
        PrivateRoomLifecycleStatus::Archived
    );
    assert_eq!(
        restore_now(&repositories, parent, sub_room, owner, time(50)).await,
        SubRoomArchiveOutcome::Unavailable
    );
    assert_eq!(
        restore_now(&repositories, parent, other, owner, time(50)).await,
        SubRoomArchiveOutcome::Unavailable
    );

    database.close().await;
}
