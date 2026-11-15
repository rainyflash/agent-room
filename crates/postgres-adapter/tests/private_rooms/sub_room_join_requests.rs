//! 子房间的存储（`specs/redesign/sub-rooms.md` 的 2d-2）：申请锁着门记，他还在父房间里、这一间此刻
//! 收申请（要批准、在名单里显示、没归档）才记；换完门不收申请的、他离开了父房间的、父房间归档了的，
//! 在等的申请在那一次的事务里作废；每小时的清理不再把子房间的申请当成“房间不公开了”作废。

use agent_room_application::ports::{
    JoinRequestRecord, PrivateRoomJoinRequestStore, SubRoomDoorChange, SubRoomJoinRequestOutcome,
    SubRoomRevision,
};
use agent_room_domain::{
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
    sub_rooms::SubRoomDoor,
};

use super::{membership::revise, *};

/// 一个在等的申请。
fn record(catalog_id: RoomCatalogId, requester: PrincipalId, at: UtcMillis) -> JoinRequestRecord {
    JoinRequestRecord {
        catalog_id,
        requester,
        message: JoinRequestMessage::parse("想来帮忙").expect("留言有效"),
        status: JoinRequestStatus::Pending,
        requested_at: at,
        decided_at: None,
        decided_by: None,
    }
}

async fn request(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    catalog_id: RoomCatalogId,
    requester: PrincipalId,
    at: UtcMillis,
) -> SubRoomJoinRequestOutcome {
    SubRoomStore::request_sub_room_join(repositories, parent, &record(catalog_id, requester, at))
        .await
        .expect("写入不报错")
}

/// 他在这间房的申请，按记下的先后：每一个的状态。
async fn statuses(pool: &PgPool, catalog: RoomCatalogId, requester: PrincipalId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT status FROM agent_room.private_room_join_request
          WHERE catalog_entry_id = $1 AND requester_principal_id = $2
          ORDER BY id",
    )
    .bind(catalog.as_uuid())
    .bind(requester.as_uuid())
    .fetch_all(pool)
    .await
    .expect("可读")
}

/// 一间带门的子房间（建的人是房主），回它的房间号。
async fn guarded(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    owner: PrincipalId,
    name: &str,
    (access, hidden): (RoomJoinMode, bool),
    at: UtcMillis,
) -> RoomCatalogId {
    let sub_room = NewSubRoom {
        access,
        hidden,
        ..draft(repositories, parent, owner, name, at).await
    };
    assert_eq!(create(repositories, &sub_room).await, None);
    sub_room.snapshot.catalog().id()
}

/// 只换门，名字照旧。
fn door_change(
    parent: RoomCatalogId,
    catalog_id: RoomCatalogId,
    name: &str,
    doors: Option<(RoomJoinMode, RoomJoinMode)>,
    at: UtcMillis,
) -> SubRoomRevision {
    let door = |access| SubRoomDoor::new(access, false).expect("门有效");
    SubRoomRevision {
        catalog_id,
        parent_catalog_id: parent,
        name: name.to_owned(),
        name_key: sub_room_name_key(name),
        description: String::new(),
        door: doors.map(|(from, to)| SubRoomDoorChange {
            from: door(from),
            to: door(to),
        }),
        at,
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 申请锁着门记_要批准又在名单里的才收_不在父房间的不收_同一个人只记一个() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-request-owner").await;
    let member = seed_principal(&database.runtime, "sub-room-request-member").await;
    let outsider = seed_principal(&database.runtime, "sub-room-request-outsider").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
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
    let mut closed = Vec::new();
    for (name, door) in [
        ("Vault", (RoomJoinMode::InviteOnly, false)),
        ("Secret", (RoomJoinMode::Request, true)),
        ("Plain", (RoomJoinMode::Open, false)),
    ] {
        closed.push(guarded(&repositories, parent, owner, name, door, time(6)).await);
    }

    let created = request(&repositories, parent, security, member, time(30)).await;
    assert_eq!(created, SubRoomJoinRequestOutcome::Created);
    let again = request(&repositories, parent, security, member, time(31)).await;
    assert_eq!(
        again,
        SubRoomJoinRequestOutcome::AlreadyPending(time(30)),
        "同一个人只记一个，回先记的那个"
    );
    assert_eq!(
        statuses(&database.runtime, security, member).await,
        ["pending"]
    );
    for catalog in closed {
        assert_eq!(
            request(&repositories, parent, catalog, member, time(32)).await,
            SubRoomJoinRequestOutcome::Closed,
            "只能请进来的、藏起来的、没有门的都不收"
        );
        assert!(
            statuses(&database.runtime, catalog, member)
                .await
                .is_empty()
        );
    }
    assert_eq!(
        request(&repositories, parent, security, outsider, time(33)).await,
        SubRoomJoinRequestOutcome::NotInParent
    );

    // 名单上：申请的人看到自己在等，别人看到的只是有几个在等（给不给看由用例定）。
    let by_member = list(&repositories, parent, member).await;
    let row = by_member
        .iter()
        .find(|entry| entry.catalog_id == security)
        .expect("在名单里");
    assert_eq!(
        (row.viewer_requested_at, row.pending_requests),
        (Some(time(30)), 1)
    );
    let by_owner = list(&repositories, parent, owner).await;
    let row = by_owner
        .iter()
        .find(|entry| entry.catalog_id == security)
        .expect("在名单里");
    assert_eq!((row.viewer_requested_at, row.pending_requests), (None, 1));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 换完门不收申请的作废_改名不动_离开父房间的作废_父房间归档了的都作废() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-void-owner").await;
    let member = seed_principal(pool, "sub-room-void-member").await;
    let leaver = seed_principal(pool, "sub-room-void-leaver").await;
    let parent = create_room(&repositories, owner).await;
    for person in [member, leaver] {
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
    let design = guarded(
        &repositories,
        parent,
        owner,
        "Design",
        request_door,
        time(5),
    )
    .await;
    for (catalog, person) in [(security, member), (security, leaver), (design, member)] {
        let outcome = request(&repositories, parent, catalog, person, time(30)).await;
        assert_eq!(outcome, SubRoomJoinRequestOutcome::Created);
    }

    // 只改名：门那几列不碰，在等的照旧等。
    let renamed = door_change(parent, security, "Security 2", None, time(40));
    assert_eq!(revise(&repositories, &renamed).await, None);
    assert_eq!(statuses(pool, security, member).await, ["pending"]);

    // 他离开了父房间：他在各子房间在等的申请，改父房间的那个事务里就作废。
    let above = load(&repositories, parent).await;
    let mut without = above.room().clone();
    without.remove_member(owner, leaver).expect("房主能移出");
    PrivateRoomStore::save(&repositories, &without, above.room().version(), time(41))
        .await
        .expect("移出存得进去");
    assert_eq!(statuses(pool, security, leaver).await, ["voided"]);
    assert_eq!(statuses(pool, security, member).await, ["pending"]);

    // 改成只能请进来：在等的一起作废，别的子房间的不动。
    let doors = Some((RoomJoinMode::Request, RoomJoinMode::InviteOnly));
    let invite_only = door_change(parent, security, "Security 2", doors, time(50));
    assert_eq!(revise(&repositories, &invite_only).await, None);
    assert_eq!(statuses(pool, security, member).await, ["voided"]);
    assert_eq!(statuses(pool, design, member).await, ["pending"]);
    let decided: Option<i64> = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM decided_at) * 1000)::bigint
           FROM agent_room.private_room_join_request
          WHERE catalog_entry_id = $1 AND requester_principal_id = $2",
    )
    .bind(security.as_uuid())
    .bind(member.as_uuid())
    .fetch_one(pool)
    .await
    .expect("可读");
    assert_eq!(decided, Some(time(50).value()), "作废的时间是换门的那一刻");

    // 父房间归档了：子房间在等的都作废。
    archive(&repositories, parent, owner).await;
    assert_eq!(statuses(pool, design, member).await, ["voided"]);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 每小时的清理不动还收申请的子房间_门被别处改了的作废_不公开的房间照旧作废() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-sweep-owner").await;
    let member = seed_principal(pool, "sub-room-sweep-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
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
    let shifting = guarded(
        &repositories,
        parent,
        owner,
        "Shifting",
        request_door,
        time(5),
    )
    .await;
    for catalog in [security, shifting] {
        let outcome = request(&repositories, parent, catalog, member, time(30)).await;
        assert_eq!(outcome, SubRoomJoinRequestOutcome::Created);
    }
    // 回滚到旧版那段时间里门被改过：没经过作废那一步。
    sqlx::query(
        "UPDATE agent_room.private_room_parent SET access = 'invite_only'
          WHERE catalog_entry_id = $1",
    )
    .bind(shifting.as_uuid())
    .execute(pool)
    .await
    .expect("可改");
    // 一间不公开的房间（不是子房间）里留着的申请，照旧算“房间不公开了”。
    let unlisted = PrivateRoomJoinRequestStore::create_join_request(
        &repositories,
        &record(parent, member, time(30)),
    )
    .await
    .expect("写入不报错");
    assert_eq!(
        unlisted,
        agent_room_application::ports::JoinRequestCreateOutcome::Created
    );

    let swept = PrivateRoomJoinRequestStore::sweep_join_requests(&repositories, time(60), time(0))
        .await
        .expect("清理得了");
    assert_eq!(swept.voided, 2);
    assert_eq!(statuses(pool, security, member).await, ["pending"]);
    assert_eq!(statuses(pool, shifting, member).await, ["voided"]);
    assert_eq!(statuses(pool, parent, member).await, ["voided"]);

    database.close().await;
}
