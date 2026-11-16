//! 子房间最近有人说话的时间（`specs/redesign/sub-rooms.md` 第 2j-1 步）：没归档、父房间也在用的子房间
//! 才读；记下的时间只往后挪，也不动房间的版本。

use agent_room_application::ports::{SubRoomActivityStore, SubRoomArchiveOutcome};

use super::{archive::archive_now, *};

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
