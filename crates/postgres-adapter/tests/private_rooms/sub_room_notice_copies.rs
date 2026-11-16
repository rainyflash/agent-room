//! 子房间里父房间须知的副本还没写的（`private_room_parent_notice_sync`，须知与置顶的 4N-3b）：新建子房间时
//! 在建房的事务里排一条；父房间改须知时下面每间子房间（归档了的也算）重排；领、写好、没写成往后排，规矩
//! 和 `private_room_sub_room_sync` 一样。测试库是一路用下来的，所以都只领自己那间房下面的。

use agent_room_application::ports::{
    ParentNoticeSyncStore, ParentNoticeSyncTask, SubRoomArchiveOutcome, SubRoomSyncScope,
};

use super::{archive::archive_now, sync::open, *};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 新建子房间时排一条_改须知时下面每间都重排_归档了的也排_只领这间房下面的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "notice-copy-queue-owner").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    let gamma = open(&repositories, elsewhere, "Gamma", 12).await;
    assert_eq!(
        archive_now(&repositories, parent, beta, owner, time(20)).await,
        SubRoomArchiveOutcome::Done
    );
    for sub_room in [alpha, beta, gamma] {
        assert_eq!(
            row(pool, sub_room).await,
            Some((1, 0, None)),
            "建的时候排上"
        );
    }

    let under = SubRoomSyncScope::Under(parent);
    let first = claim(&repositories, under, time(100), time(200)).await;
    assert_eq!(catalogs(&first), [alpha, beta], "先建的在前");
    assert!(
        first
            .iter()
            .all(|task| (task.generation, task.attempt) == (1, 1))
    );
    assert!(
        claim(&repositories, under, time(150), time(300))
            .await
            .is_empty(),
        "租约里领不到"
    );

    // 父房间改了须知：下面两间都重排，归档了的也算；别的房间下面的不动。
    let queued = ParentNoticeSyncStore::queue_parent_notice_sync(&repositories, parent, time(160))
        .await
        .expect("排得上");
    assert_eq!(queued, 2);
    for sub_room in [alpha, beta] {
        assert_eq!(row(pool, sub_room).await, Some((2, 0, None)));
    }
    assert_eq!(row(pool, gamma).await, Some((1, 0, None)));
    let again = claim(&repositories, under, time(170), time(300)).await;
    assert_eq!(catalogs(&again), [alpha, beta], "重排的马上能领");
    assert!(
        again
            .iter()
            .all(|task| (task.generation, task.attempt) == (2, 1))
    );

    let apart = claim(
        &repositories,
        SubRoomSyncScope::Under(elsewhere),
        time(100),
        time(200),
    )
    .await;
    assert_eq!(catalogs(&apart), [gamma]);

    // 没有子房间的房间：什么都不排。
    let lonely = create_room(&repositories, owner).await;
    let none = ParentNoticeSyncStore::queue_parent_notice_sync(&repositories, lonely, time(160))
        .await
        .expect("排得上");
    assert_eq!(none, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 写好删掉领到的那一版_做的时候又排进来的再排一次_没写成记原因到点再试() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "notice-copy-done-owner").await;
    let parent = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let under = SubRoomSyncScope::Under(parent);
    let next = || async { one(&claim(&repositories, under, time(100), time(200)).await) };

    // 建的时候排的那一条：领走、写好，删掉。
    let task = next().await;
    complete(&repositories, task).await;
    assert_eq!(row(pool, alpha).await, None);

    // 写的时候父房间又改了一次：先领的那一处写好时删不到它，再排一次，马上能领、从头数。
    queue(&repositories, parent).await;
    let stale = next().await;
    queue(&repositories, parent).await;
    assert_eq!(row(pool, alpha).await, Some((2, 0, None)));
    complete(&repositories, stale).await;
    assert_eq!(row(pool, alpha).await, Some((3, 0, None)));
    let fresh = next().await;
    assert_eq!((fresh.generation, fresh.attempt), (3, 1));

    // 没写成：记下原因，没到点领不到，到点再领算第二次。
    retry(&repositories, fresh, time(500), "matrix.unavailable").await;
    assert_eq!(
        row(pool, alpha).await,
        Some((3, 1, Some("matrix.unavailable".to_owned())))
    );
    assert!(
        claim(&repositories, under, time(499), time(600))
            .await
            .is_empty()
    );
    let second = one(&claim(&repositories, under, time(500), time(600)).await);
    assert_eq!((second.generation, second.attempt), (3, 2));

    // 写的时候又排进来过：没写成的原因不记到新的那一版上，再排一次。
    queue(&repositories, parent).await;
    retry(&repositories, second, time(900), "matrix.forbidden").await;
    assert_eq!(row(pool, alpha).await, Some((5, 0, None)));

    // 后领的那一处先写好、把这一条删了：先领的那一处写好时照样再排。
    let latest = one(&claim(&repositories, under, time(501), time(600)).await);
    complete(&repositories, latest).await;
    assert_eq!(row(pool, alpha).await, None);
    complete(&repositories, second).await;
    assert_eq!(row(pool, alpha).await, Some((1, 0, None)));

    // 子房间连父子关系一起没了的不排，也不报错。
    let nowhere = ParentNoticeSyncTask {
        catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
        ..second
    };
    complete(&repositories, nowhere).await;
    retry(&repositories, nowhere, time(900), "matrix.unavailable").await;
    assert_eq!(row(pool, nowhere.catalog_id).await, None);

    database.close().await;
}

async fn queue(repositories: &PostgresRepositories, parent: RoomCatalogId) {
    ParentNoticeSyncStore::queue_parent_notice_sync(repositories, parent, time(300))
        .await
        .expect("排得上");
}

async fn claim(
    repositories: &PostgresRepositories,
    scope: SubRoomSyncScope,
    now: UtcMillis,
    lease_until: UtcMillis,
) -> Vec<ParentNoticeSyncTask> {
    ParentNoticeSyncStore::claim_parent_notice_sync(repositories, scope, 10, now, lease_until)
        .await
        .expect("领得到")
}

async fn complete(repositories: &PostgresRepositories, task: ParentNoticeSyncTask) {
    ParentNoticeSyncStore::complete_parent_notice_sync(repositories, task)
        .await
        .expect("记得上");
}

async fn retry(
    repositories: &PostgresRepositories,
    task: ParentNoticeSyncTask,
    retry_at: UtcMillis,
    failure_code: &'static str,
) {
    ParentNoticeSyncStore::retry_parent_notice_sync(repositories, task, retry_at, failure_code)
        .await
        .expect("记得上");
}

fn one(tasks: &[ParentNoticeSyncTask]) -> ParentNoticeSyncTask {
    assert_eq!(tasks.len(), 1, "该领到一件：{tasks:?}");
    tasks[0]
}

fn catalogs(tasks: &[ParentNoticeSyncTask]) -> Vec<RoomCatalogId> {
    tasks.iter().map(|task| task.catalog_id).collect()
}

/// 这间子房间排着的那一条：第几版、领过几次、上次没写成的原因。
async fn row(pool: &PgPool, sub_room: RoomCatalogId) -> Option<(i64, i32, Option<String>)> {
    sqlx::query_as(
        r"SELECT generation, attempts, failure_code
            FROM agent_room.private_room_parent_notice_sync
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .fetch_optional(pool)
    .await
    .expect("排着的副本可读")
}
