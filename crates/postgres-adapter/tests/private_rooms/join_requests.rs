//! 申请进公开房间（`specs/public-rooms/design.md`）：同一个人同时只有一个在等的申请；处理时间不早于
//! 申请时间；改回私密、归档的房间里在等的作废；处理完过了保留期的删掉。

use agent_room_application::ports::{
    JoinRequestCreateOutcome, JoinRequestRecord, JoinRequestSweep, PrivateRoomJoinRequestStore,
};
use agent_room_domain::room_join_requests::{JoinRequestMessage, JoinRequestStatus};

use super::{
    listing::{Saved, archive, create_room, record, save, version},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 申请往返_同一个人只有一个在等的_处理以后能再申请() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "join-owner").await;
    let requester = seed_principal(&database.runtime, "join-requester").await;
    let room = listed_room(&repositories, owner, "Req001").await;

    let first = waiting(room, requester, "想来听听", 2_000);
    assert_eq!(
        create(&repositories, &first).await,
        JoinRequestCreateOutcome::Created
    );
    assert_eq!(
        create(&repositories, &waiting(room, requester, "换一句", 2_500)).await,
        JoinRequestCreateOutcome::AlreadyPending(first.clone()),
        "已经有一个在等的就回答那一个"
    );
    assert_eq!(
        PrivateRoomJoinRequestStore::pending_join_request(&repositories, room, requester)
            .await
            .expect("可读"),
        Some(first.clone())
    );
    let pending = PrivateRoomJoinRequestStore::pending_join_requests(&repositories, room)
        .await
        .expect("可读");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].record, first);
    assert_eq!(pending[0].requester_display_name, "测试主体 join-requester");

    // 处理时间早于申请时间（两台机器的钟不一样）也按申请时间记。
    assert!(
        decide(
            &repositories,
            room,
            requester,
            JoinRequestStatus::Rejected,
            Some(owner),
            1_000
        )
        .await
    );
    assert!(
        !decide(
            &repositories,
            room,
            requester,
            JoinRequestStatus::Approved,
            Some(owner),
            3_000
        )
        .await,
        "没有在等的了"
    );
    assert_eq!(
        PrivateRoomJoinRequestStore::last_rejected_at(&repositories, room, requester)
            .await
            .expect("可读"),
        Some(time(2_000))
    );
    assert_eq!(
        PrivateRoomJoinRequestStore::pending_join_request(&repositories, room, requester)
            .await
            .expect("可读"),
        None
    );

    let second = waiting(room, requester, "", 4_000);
    assert_eq!(
        create(&repositories, &second).await,
        JoinRequestCreateOutcome::Created
    );
    assert!(
        decide(
            &repositories,
            room,
            requester,
            JoinRequestStatus::Withdrawn,
            None,
            5_000
        )
        .await
    );
    let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(
        r"SELECT status, decided_by FROM agent_room.private_room_join_request
           WHERE catalog_entry_id = $1 ORDER BY requested_at",
    )
    .bind(room.as_uuid())
    .fetch_all(&database.runtime)
    .await
    .expect("可读");
    assert_eq!(
        rows,
        [
            ("rejected".to_owned(), Some(owner.as_uuid())),
            ("withdrawn".to_owned(), None),
        ]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 改回私密和归档的房间里在等的作废_处理完过了保留期的删掉() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let requester = seed_principal(&database.runtime, "sweep-requester").await;
    let other = seed_principal(&database.runtime, "sweep-other").await;
    let listed_owner = seed_principal(&database.runtime, "sweep-listed").await;
    let listed = listed_room(&repositories, listed_owner, "Swp001").await;
    let unlisted_owner = seed_principal(&database.runtime, "sweep-unlisted").await;
    let unlisted = listed_room(&repositories, unlisted_owner, "Swp002").await;
    let archived_owner = seed_principal(&database.runtime, "sweep-archived").await;
    let archived = listed_room(&repositories, archived_owner, "Swp003").await;
    for room in [listed, unlisted, archived] {
        let request = waiting(room, requester, "", 1_000);
        assert_eq!(
            create(&repositories, &request).await,
            JoinRequestCreateOutcome::Created
        );
    }
    let hidden = record(unlisted, unlisted_owner, false, "Swp002", 2);
    assert_eq!(
        save(&repositories, &hidden, Some(version(1)), unlisted_owner).await,
        Saved
    );
    archive(&repositories, archived, archived_owner).await;

    // 一个早就处理完的、一个刚处理完的。
    create(&repositories, &waiting(listed, other, "", 100)).await;
    assert!(
        decide(
            &repositories,
            listed,
            other,
            JoinRequestStatus::Rejected,
            Some(listed_owner),
            200
        )
        .await
    );
    let recent = seed_principal(&database.runtime, "sweep-recent").await;
    create(&repositories, &waiting(listed, recent, "", 6_000)).await;
    assert!(
        decide(
            &repositories,
            listed,
            recent,
            JoinRequestStatus::Approved,
            Some(listed_owner),
            7_000
        )
        .await
    );

    let sweep =
        PrivateRoomJoinRequestStore::sweep_join_requests(&repositories, time(10_000), time(5_000))
            .await
            .expect("清理");
    assert_eq!(
        sweep,
        JoinRequestSweep {
            voided: 2,
            deleted: 1
        }
    );
    for (room, still_waiting) in [(listed, true), (unlisted, false), (archived, false)] {
        let pending =
            PrivateRoomJoinRequestStore::pending_join_request(&repositories, room, requester)
                .await
                .expect("可读");
        assert_eq!(pending.is_some(), still_waiting);
    }
    assert_eq!(
        PrivateRoomJoinRequestStore::last_rejected_at(&repositories, listed, other)
            .await
            .expect("可读"),
        None,
        "过了保留期的删掉了"
    );

    assert_eq!(
        PrivateRoomJoinRequestStore::void_pending_join_requests(
            &repositories,
            listed,
            time(11_000)
        )
        .await
        .expect("作废"),
        1
    );
    assert!(
        PrivateRoomJoinRequestStore::pending_join_requests(&repositories, listed)
            .await
            .expect("可读")
            .is_empty()
    );
    let remaining: i64 = sqlx::query_scalar(
        r"SELECT count(*) FROM agent_room.private_room_join_request
           WHERE catalog_entry_id = ANY($1)",
    )
    .bind([listed.as_uuid(), unlisted.as_uuid(), archived.as_uuid()])
    .fetch_one(&database.runtime)
    .await
    .expect("可读");
    assert_eq!(remaining, 4, "作废的留到过了保留期");
    database.close().await;
}

async fn listed_room(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    share_code: &str,
) -> RoomCatalogId {
    let room = create_room(repositories, owner).await;
    assert_eq!(
        save(
            repositories,
            &record(room, owner, true, share_code, 1),
            None,
            owner
        )
        .await,
        Saved
    );
    room
}

fn waiting(
    catalog_id: RoomCatalogId,
    requester: PrincipalId,
    message: &str,
    requested_at: i64,
) -> JoinRequestRecord {
    JoinRequestRecord {
        catalog_id,
        requester,
        message: JoinRequestMessage::parse(message).unwrap(),
        status: JoinRequestStatus::Pending,
        requested_at: time(requested_at),
        decided_at: None,
        decided_by: None,
    }
}

async fn create(
    repositories: &PostgresRepositories,
    record: &JoinRequestRecord,
) -> JoinRequestCreateOutcome {
    PrivateRoomJoinRequestStore::create_join_request(repositories, record)
        .await
        .expect("写入不报错")
}

async fn decide(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    requester: PrincipalId,
    status: JoinRequestStatus,
    decided_by: Option<PrincipalId>,
    decided_at: i64,
) -> bool {
    PrivateRoomJoinRequestStore::decide_join_request(
        repositories,
        catalog_id,
        requester,
        status,
        decided_by,
        time(decided_at),
    )
    .await
    .expect("写入不报错")
}
