//! 子房间的存储（`specs/redesign/sub-rooms.md` 的 2d-3）：在子房间里被封禁的人，在这一间在等的申请
//! 在存房间的那个事务里作废，每小时的清理也兜着；整间房（不是子房间）的封禁不动申请。

use agent_room_application::ports::{JoinRequestCreateOutcome, PrivateRoomJoinRequestStore};

use super::{
    join_requests::{guarded, record, request, statuses},
    *,
};

/// 在一间房里封禁一个人，照用例那样经过存储存进去。
async fn ban(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    owner: PrincipalId,
    target: PrincipalId,
    at: UtcMillis,
) {
    let snapshot = load(repositories, catalog).await;
    let mut room = snapshot.room().clone();
    room.ban_member(owner, target).expect("房主能封禁");
    PrivateRoomStore::save(repositories, &room, snapshot.room().version(), at)
        .await
        .expect("封禁存得进去");
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 子房间里被封禁的人_在等的申请存房间时作废_清理也兜着_整间房的封禁不动申请() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-ban-owner").await;
    let member = seed_principal(pool, "sub-room-ban-member").await;
    let other = seed_principal(pool, "sub-room-ban-other").await;
    let stranger = seed_principal(pool, "sub-room-ban-stranger").await;
    let parent = create_room(&repositories, owner).await;
    for person in [member, other] {
        admit(&repositories, parent, owner, person).await;
    }
    let request_door = (RoomJoinMode::Request, false);
    let security = guarded(
        &repositories,
        parent,
        owner,
        "Security",
        request_door,
        time(5),
    )
    .await;
    for person in [member, other] {
        let outcome = request(&repositories, parent, security, person, time(30)).await;
        assert_eq!(
            outcome,
            agent_room_application::ports::SubRoomJoinRequestOutcome::Created
        );
    }

    // 在子房间里封禁他（从没进来过的也封得了）：他在等的申请同一个事务里作废，别人的不动。
    ban(&repositories, security, owner, member, time(40)).await;
    assert_eq!(statuses(pool, security, member).await, ["voided"]);
    assert_eq!(statuses(pool, security, other).await, ["pending"]);

    // 回滚到旧版那段时间里被封禁的：存房间时没作废，每小时的清理兜着。
    sqlx::query(
        "INSERT INTO agent_room.private_room_membership (
             catalog_entry_id, principal_id, membership_status, permission_bits,
             created_at, status_changed_at
         ) VALUES ($1, $2, 'banned', 0, now(), now())",
    )
    .bind(security.as_uuid())
    .bind(other.as_uuid())
    .execute(pool)
    .await
    .expect("可写");
    let swept = PrivateRoomJoinRequestStore::sweep_join_requests(&repositories, time(60), time(0))
        .await
        .expect("清理得了");
    assert!(swept.voided >= 1);
    assert_eq!(statuses(pool, security, other).await, ["voided"]);

    // 整间房的封禁不动申请（公开房间那一套自己管）。
    let created = PrivateRoomJoinRequestStore::create_join_request(
        &repositories,
        &record(parent, stranger, time(70)),
    )
    .await
    .expect("写入不报错");
    assert_eq!(created, JoinRequestCreateOutcome::Created);
    ban(&repositories, parent, owner, stranger, time(80)).await;
    assert_eq!(statuses(pool, parent, stranger).await, ["pending"]);

    database.close().await;
}
