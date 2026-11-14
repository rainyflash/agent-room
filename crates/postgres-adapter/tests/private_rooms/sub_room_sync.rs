//! 聊天服务器上还没跟着做的事（`private_room_sub_room_sync`，2c-2）：领、做完、没做成往后排。往里排
//! 是存父房间时的事，在 `sub_room_follow.rs` 里查过。测试库是一路用下来的，别的用例排着的事还留着，
//! 所以这里都只领自己那间房下面的；不分房间地领时只数自己的。

use agent_room_application::ports::{SubRoomSyncScope, SubRoomSyncTask};

use super::{
    follow::{id, queued, waiting},
    membership::{change, enter},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 排着的事按先后领_租约里别处领不到_过了能再领_只领这间房下面的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-sync-claim-owner").await;
    let first = seed_principal(pool, "sub-room-sync-claim-first").await;
    let second = seed_principal(pool, "sub-room-sync-claim-second").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    for room in [parent, elsewhere] {
        for person in [first, second] {
            admit(&repositories, room, owner, person).await;
        }
    }
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    let gamma = open(&repositories, elsewhere, "Gamma", 12).await;
    for (above, sub_room) in [(parent, alpha), (parent, beta), (elsewhere, gamma)] {
        for (person, at) in [(first, 20), (second, 21)] {
            assert!(enter(&repositories, above, sub_room, person, time(at)).await);
        }
    }
    // 先移出一个、再移出另一个：两间子房间里各排上两件，先排的在前。别的房间里另有一件。
    for (above, person, at) in [
        (parent, first, 50),
        (elsewhere, first, 55),
        (parent, second, 60),
    ] {
        change(&repositories, above, time(at), |room| {
            room.remove_member(owner, person).expect("房主能移出");
        })
        .await;
    }
    let under = SubRoomSyncScope::Under(parent);

    let earliest = claim(&repositories, under, 3, time(100), time(200)).await;
    assert_eq!(
        earliest
            .iter()
            .map(|task| task.principal)
            .collect::<Vec<_>>(),
        [Some(first), Some(first), Some(second)],
        "先排的先领"
    );
    assert!(earliest.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(
        earliest
            .iter()
            .all(|task| (task.generation, task.attempt) == (1, 1))
    );
    assert!(
        earliest
            .iter()
            .all(|task| [alpha, beta].contains(&task.catalog_id))
    );

    // 领走的在租约里别处领不到：剩下的只有一件。
    let rest = claim(&repositories, under, 10, time(100), time(250)).await;
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].principal, Some(second));
    assert!(
        claim(&repositories, under, 10, time(199), time(300))
            .await
            .is_empty()
    );

    // 别的房间下面的不受影响，也只领它那一件。
    let other = SubRoomSyncScope::Under(elsewhere);
    let apart = claim(&repositories, other, 10, time(100), time(200)).await;
    assert_eq!(apart.len(), 1);
    assert_eq!(
        (apart[0].catalog_id, apart[0].principal),
        (gamma, Some(first))
    );

    // 租约到了（做到一半进程断了）：别处接着领，算第二次。
    let again = claim(&repositories, under, 10, time(200), time(300)).await;
    assert_eq!(ids(&again), ids(&earliest));
    assert!(
        again
            .iter()
            .all(|task| (task.generation, task.attempt) == (1, 2))
    );

    // 不分房间地领：到点的都领得到。
    let everywhere = SubRoomSyncScope::Everywhere;
    let everything = claim(
        &repositories,
        everywhere,
        u16::MAX,
        time(1_000),
        time(2_000),
    )
    .await;
    let mine = everything
        .iter()
        .filter(|task| [alpha, beta, gamma].contains(&task.catalog_id))
        .count();
    assert_eq!(mine, 5);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 做完删掉领到的那一版_做的时候又排进来过的再排一次_子房间不在了的不排() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-sync-done-owner").await;
    let member = seed_principal(pool, "sub-room-sync-done-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    assert!(enter(&repositories, parent, alpha, member, time(20)).await);
    let under = SubRoomSyncScope::Under(parent);
    let next = || async { one(&claim(&repositories, under, 10, time(100), time(200)).await) };

    // 收回发言：排上一件。领走、做完，队里空了。
    set_rights(&repositories, parent, member, viewing(), 30).await;
    let task = next().await;
    assert_eq!((task.catalog_id, task.principal), (alpha, Some(member)));
    assert_eq!((task.generation, task.attempt), (1, 1));
    complete(&repositories, task).await;
    assert!(queued(pool, alpha).await.is_empty());

    // 做的时候又改了一次：同一条换成新的一版。先领的那一处做完时删不到它，再排一次，马上能领。
    set_rights(&repositories, parent, member, ordinary_permissions(), 40).await;
    let stale = next().await;
    set_rights(&repositories, parent, member, viewing(), 50).await;
    assert_eq!(queued(pool, alpha).await, waiting([(member, 2)]));
    complete(&repositories, stale).await;
    assert_eq!(queued(pool, alpha).await, waiting([(member, 3)]));
    let fresh = next().await;
    assert_eq!(
        (fresh.id, fresh.generation, fresh.attempt),
        (stale.id, 3, 1)
    );

    // 后领的那一处先做完、把这一条删了：先领的那一处做完时照样再排，是新的一条。
    complete(&repositories, fresh).await;
    assert!(queued(pool, alpha).await.is_empty());
    complete(&repositories, stale).await;
    assert_eq!(queued(pool, alpha).await, waiting([(member, 1)]));
    let renewed = next().await;
    assert_ne!(renewed.id, stale.id);
    complete(&repositories, renewed).await;

    // 这间子房间自己的事（跟着父房间归档）也一样，认的是另一个唯一索引。
    change(&repositories, parent, time(60), |room| {
        room.archive(owner).expect("房主能归档");
    })
    .await;
    let lock = next().await;
    assert_eq!((lock.catalog_id, lock.principal), (alpha, None));
    complete(&repositories, lock).await;
    assert!(queued(pool, alpha).await.is_empty());
    complete(&repositories, lock).await;
    assert_eq!(
        queued(pool, alpha).await.into_iter().collect::<Vec<_>>(),
        [(None, 1)]
    );

    // 子房间连父子关系一起没了的不排，也不报错。
    let nowhere = RoomCatalogId::from_uuid(Uuid::now_v7());
    let gone = SubRoomSyncTask {
        catalog_id: nowhere,
        ..stale
    };
    complete(&repositories, gone).await;
    retry(&repositories, gone, time(900), "matrix.unavailable").await;
    assert!(queued(pool, nowhere).await.is_empty());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 没做成记下原因到点再试_做的时候又排进来过的不记_再排一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-sync-retry-owner").await;
    let member = seed_principal(pool, "sub-room-sync-retry-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    assert!(enter(&repositories, parent, alpha, member, time(20)).await);
    let under = SubRoomSyncScope::Under(parent);

    set_rights(&repositories, parent, member, viewing(), 30).await;
    let task = one(&claim(&repositories, under, 10, time(100), time(200)).await);
    retry(&repositories, task, time(500), "matrix.unavailable").await;
    assert_eq!(
        failure(pool, alpha, member).await.as_deref(),
        Some("matrix.unavailable")
    );
    assert!(
        claim(&repositories, under, 10, time(499), time(600))
            .await
            .is_empty(),
        "没到点领不到"
    );
    let second = one(&claim(&repositories, under, 10, time(500), time(600)).await);
    assert_eq!(
        (second.id, second.generation, second.attempt),
        (task.id, 1, 2)
    );

    // 做的时候又改了一次：没做成的原因不记到新的那一版上，再排一次，马上能领、从头数。
    set_rights(&repositories, parent, member, ordinary_permissions(), 40).await;
    retry(&repositories, second, time(900), "matrix.forbidden").await;
    assert_eq!(queued(pool, alpha).await, waiting([(member, 3)]));
    assert_eq!(failure(pool, alpha, member).await, None);
    let fresh = one(&claim(&repositories, under, 10, time(501), time(600)).await);
    assert_eq!((fresh.generation, fresh.attempt), (3, 1));

    database.close().await;
}

/// 房主在这间房下面建一间没有门的子房间。
async fn open(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    name: &str,
    at: i64,
) -> RoomCatalogId {
    let owner = load(repositories, parent).await.room().owner_principal_id();
    let sub_room = draft(repositories, parent, owner, name, time(at)).await;
    assert_eq!(create(repositories, &sub_room).await, None);
    id(&sub_room)
}

fn viewing() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View]).expect("权限有效")
}

/// 房主在父房间里改一个人的权限：他在的子房间里跟着重算，聊天服务器上要做的事排上。
async fn set_rights(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    target: PrincipalId,
    permissions: PrivateRoomPermissions,
    at: i64,
) {
    change(repositories, parent, time(at), |room| {
        let owner = room.owner_principal_id();
        room.update_permissions(owner, target, permissions)
            .expect("房主能改权限");
    })
    .await;
}

async fn claim(
    repositories: &PostgresRepositories,
    scope: SubRoomSyncScope,
    limit: u16,
    now: UtcMillis,
    lease_until: UtcMillis,
) -> Vec<SubRoomSyncTask> {
    SubRoomStore::claim_sub_room_sync(repositories, scope, limit, now, lease_until)
        .await
        .expect("领得到")
}

async fn complete(repositories: &PostgresRepositories, task: SubRoomSyncTask) {
    SubRoomStore::complete_sub_room_sync(repositories, task)
        .await
        .expect("记得上");
}

async fn retry(
    repositories: &PostgresRepositories,
    task: SubRoomSyncTask,
    retry_at: UtcMillis,
    failure_code: &'static str,
) {
    SubRoomStore::retry_sub_room_sync(repositories, task, retry_at, failure_code)
        .await
        .expect("记得上");
}

fn one(tasks: &[SubRoomSyncTask]) -> SubRoomSyncTask {
    assert_eq!(tasks.len(), 1, "该领到一件：{tasks:?}");
    tasks[0]
}

fn ids(tasks: &[SubRoomSyncTask]) -> Vec<i64> {
    tasks.iter().map(|task| task.id).collect()
}

/// 这个人在这间子房间里排着的那件事上记的没做成的原因。
async fn failure(pool: &PgPool, sub_room: RoomCatalogId, principal: PrincipalId) -> Option<String> {
    sqlx::query_scalar(
        r"SELECT failure_code
            FROM agent_room.private_room_sub_room_sync
           WHERE catalog_entry_id = $1
             AND principal_id = $2",
    )
    .bind(sub_room.as_uuid())
    .bind(principal.as_uuid())
    .fetch_one(pool)
    .await
    .expect("排着的事可读")
}
