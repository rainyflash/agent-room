//! 子房间最近有人说话的时间（`specs/redesign/sub-rooms.md` 第 2j-1 步）：没归档、父房间也在用的子房间
//! 才读；记下的时间只往后挪，也不动房间的版本。30 天没人说话自动归档（第 2j-2 步）：领走最近说话、
//! 最近恢复、建的时间都早于界限的，租期里别人领不到；父房间关了自动归档的不领。

use agent_room_application::ports::{
    MatrixRoomId, QuietSubRoom, SubRoomActivityStore, SubRoomArchiveOutcome,
};
use agent_room_domain::sub_rooms::SubRoomPolicy;

use super::{
    archive::{archive_now, restore_now},
    *,
};

const DAY: i64 = 24 * 60 * 60 * 1000;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 只列没归档的子房间_父房间归档了一间都不列_最近说话只往后挪() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-activity-owner").await;
    let parent = create_room(&repositories, owner).await;
    let alpha = draft(&repositories, parent, owner, "Alpha", time(3)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(5)).await;
    for sub_room in [&alpha, &beta] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let id = |sub_room: &NewSubRoom| sub_room.snapshot.catalog().id();
    assert_eq!(
        archive_now(&repositories, parent, id(&beta), owner, time(7)).await,
        SubRoomArchiveOutcome::Done
    );

    assert_eq!(listed(&repositories).await, [id(&alpha)], "归档了的不读");

    let before = load(&repositories, id(&alpha)).await.room().version();
    for at in [9_000, 8_000] {
        SubRoomActivityStore::record_sub_room_last_message(&repositories, id(&alpha), time(at))
            .await
            .expect("记得下");
    }
    assert_eq!(
        last_message_at(&database.runtime, id(&alpha)).await,
        Some(9_000),
        "早一点的不往回挪"
    );
    assert_eq!(
        load(&repositories, id(&alpha)).await.room().version(),
        before,
        "不算改子房间"
    );

    archive(&repositories, parent, owner).await;
    assert!(
        listed(&repositories).await.is_empty(),
        "父房间归档了，下面的都不读"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 领走安静的子房间_恢复的从恢复那一刻算_租期里别人领不到_关了自动归档的不领() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-auto-archive-owner").await;
    let parent = create_room(&repositories, owner).await;
    // 都是开头建的：Alpha 没人说过话；Beta 第 40 天还有人说话；Gamma 第 1 天归档、第 20 天恢复。
    let alpha = draft(&repositories, parent, owner, "Alpha", time(3)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(5)).await;
    let gamma = draft(&repositories, parent, owner, "Gamma", time(7)).await;
    for sub_room in [&alpha, &beta, &gamma] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let id = |sub_room: &NewSubRoom| sub_room.snapshot.catalog().id();
    SubRoomActivityStore::record_sub_room_last_message(&repositories, id(&beta), time(40 * DAY))
        .await
        .expect("记得下");
    assert_eq!(
        archive_now(&repositories, parent, id(&gamma), owner, time(DAY)).await,
        SubRoomArchiveOutcome::Done
    );
    assert_eq!(
        restore_now(&repositories, parent, id(&gamma), owner, time(20 * DAY)).await,
        SubRoomArchiveOutcome::Done
    );
    let alpha_room = listed_rooms(&repositories)
        .await
        .into_iter()
        .find(|(catalog_id, _)| *catalog_id == id(&alpha))
        .map(|(_, matrix_room)| matrix_room)
        .expect("Alpha 在读的名单上");

    // 第 45 天：30 天以前就没人说话的只有 Alpha。
    let now = 45 * DAY;
    assert_eq!(
        claim(&repositories, now).await,
        [QuietSubRoom {
            catalog_id: id(&alpha),
            parent_catalog_id: parent,
            matrix_room: alpha_room,
        }]
    );
    assert!(
        claim(&repositories, now).await.is_empty(),
        "租期里别人领不到"
    );
    assert_eq!(
        claim(&repositories, now + 11 * 60 * 1000).await.len(),
        1,
        "租期过了再领"
    );

    // 第 51 天：Gamma 恢复也满 30 天了。
    let later = 51 * DAY;
    assert_eq!(
        ids(claim(&repositories, later).await),
        [id(&alpha), id(&gamma)]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );

    // 房主关了自动归档：一间都不领。
    let off = SubRoomPolicyRecord {
        catalog_id: parent,
        policy: SubRoomPolicy {
            auto_archive: false,
            ..SubRoomPolicy::DEFAULT
        },
        updated_by: owner,
        updated_at: time(52 * DAY),
        version: version(1),
    };
    assert!(save_policy(&repositories, &off, None).await);
    assert!(claim(&repositories, 60 * DAY).await.is_empty());

    database.close().await;
}

async fn claim(repositories: &PostgresRepositories, now: i64) -> Vec<QuietSubRoom> {
    SubRoomActivityStore::claim_quiet_sub_rooms(
        repositories,
        time(now - 30 * DAY),
        time(now),
        time(now + 10 * 60 * 1000),
        50,
    )
    .await
    .expect("领得到")
}

fn ids(rooms: Vec<QuietSubRoom>) -> std::collections::BTreeSet<RoomCatalogId> {
    rooms.into_iter().map(|room| room.catalog_id).collect()
}

async fn listed_rooms(repositories: &PostgresRepositories) -> Vec<(RoomCatalogId, MatrixRoomId)> {
    SubRoomActivityStore::sub_rooms_for_activity(repositories)
        .await
        .expect("列得出")
}

async fn listed(repositories: &PostgresRepositories) -> Vec<RoomCatalogId> {
    SubRoomActivityStore::sub_rooms_for_activity(repositories)
        .await
        .expect("列得出")
        .into_iter()
        .map(|(catalog_id, _)| catalog_id)
        .collect()
}

async fn last_message_at(pool: &PgPool, sub_room: RoomCatalogId) -> Option<i64> {
    sqlx::query_scalar(
        r"SELECT floor(extract(epoch FROM last_message_at) * 1000)::bigint
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .fetch_one(pool)
    .await
    .expect("读得出")
}
