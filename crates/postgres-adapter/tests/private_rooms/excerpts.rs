//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7a，表 `room_excerpt`）：放着实况的房间
//! 才找得到；记的时候还是读房间时那一刻开着的实况才记；关实况、改成要申请、改回私密、归档时一起删；平台
//! 拿掉时和实况那一行一样留着；每天按保留期和实况清；导出里有。删账户时删他分享的、他房间里的在
//! `real_database.rs` 的删除那一条里查。

use agent_room_application::ports::{
    AccountDeletionRepository, ExcerptRoomRecord, InsertRoomExcerptOutcome, MatrixEventId,
    PrivateRoomListingRecord, RoomExcerptRecord, RoomExcerptStore, RoomLiveViewStore,
};
use agent_room_domain::{
    room_excerpts::RoomExcerptCode,
    room_listing::{RoomJoinMode, RoomShareCode},
};

use super::{
    listing::{Saved, archive, create_room, save, settings, version},
    live_views::{enable, hold, listed_room},
    *,
};

const DAY: i64 = 86_400_000;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 放着实况的房间才找得到_记的时候核对那一刻_段码不重() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "excerpt-store").await;
    let member = seed_principal(&database.runtime, "excerpt-store-b").await;
    let (room, _) = listed_room(&repositories, owner, "Exc001").await;

    assert_eq!(excerpt_room(&repositories, room).await, None, "还没开实况");
    enable(&repositories, room, owner, 2_000).await;
    let found = excerpt_room(&repositories, room)
        .await
        .expect("开着就找得到");
    assert_eq!(found.enabled_at, time(2_000));
    assert_eq!(found.share_code, RoomShareCode::parse("Exc001").unwrap());
    assert!(found.matrix_room_id.as_str().starts_with("!private"));

    let first = excerpt("Abcd0001", room, member, 3_000, 4_000);
    assert_eq!(
        insert(&repositories, &first, 2_000).await,
        InsertRoomExcerptOutcome::Inserted
    );
    assert_eq!(
        insert(
            &repositories,
            &excerpt("Abcd0001", room, owner, 3_000, 5_000),
            2_000
        )
        .await,
        InsertRoomExcerptOutcome::CodeTaken
    );
    assert_eq!(
        insert(
            &repositories,
            &excerpt("Abcd0002", room, owner, 3_000, 5_000),
            1_999
        )
        .await,
        InsertRoomExcerptOutcome::NotLive,
        "读房间以后实况关了又开，那一刻对不上就不记"
    );
    let second = excerpt("Abcd0003", room, owner, 3_500, 6_000);
    assert_eq!(
        insert(&repositories, &second, 2_000).await,
        InsertRoomExcerptOutcome::Inserted
    );

    assert_eq!(
        listed(&repositories, room, None).await,
        [second.clone(), first.clone()]
    );
    assert_eq!(
        listed(&repositories, room, Some(member)).await,
        std::slice::from_ref(&first)
    );
    let listing = RoomExcerptStore::room_excerpts(&repositories, room, None)
        .await
        .expect("可读");
    assert_eq!(
        listing.share_code,
        Some(RoomShareCode::parse("Exc001").unwrap())
    );
    assert_eq!(
        RoomExcerptStore::find_excerpt(&repositories, room, &first.code)
            .await
            .expect("可读"),
        Some(first.clone())
    );
    let other_room = create_room(&repositories, owner).await;
    assert_eq!(
        RoomExcerptStore::find_excerpt(&repositories, other_room, &first.code)
            .await
            .expect("可读"),
        None,
        "别的房间的码不认"
    );

    for expected in [true, false] {
        assert_eq!(
            RoomExcerptStore::delete_excerpt(&repositories, room, &first.code)
                .await
                .expect("删掉，重复也行"),
            expected
        );
    }
    assert_eq!(listed(&repositories, room, None).await, [second]);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 关实况_改成要申请_改回私密_归档时一起删_平台拿掉时留着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "excerpt-off").await;
    let (room, listing) = listed_room(&repositories, owner, "Exc002").await;

    enable(&repositories, room, owner, 1_000).await;
    shared(&repositories, room, owner, "Off00001", 1_000).await;
    RoomLiveViewStore::disable_live_view(&repositories, room)
        .await
        .expect("关掉");
    assert!(listed(&repositories, room, None).await.is_empty(), "关实况");

    enable(&repositories, room, owner, 2_000).await;
    shared(&repositories, room, owner, "Off00002", 2_000).await;
    let request = PrivateRoomListingRecord {
        settings: settings(true, RoomJoinMode::Request),
        version: version(2),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &request, Some(version(1)), owner).await,
        Saved
    );
    assert!(
        listed(&repositories, room, None).await.is_empty(),
        "改成要申请"
    );

    let open = PrivateRoomListingRecord {
        version: version(3),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &open, Some(version(2)), owner).await,
        Saved
    );
    enable(&repositories, room, owner, 3_000).await;
    shared(&repositories, room, owner, "Off00003", 3_000).await;
    let private = PrivateRoomListingRecord {
        settings: settings(false, RoomJoinMode::Open),
        version: version(4),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &private, Some(version(3)), owner).await,
        Saved
    );
    assert!(
        listed(&repositories, room, None).await.is_empty(),
        "改回私密"
    );

    let listed_again = PrivateRoomListingRecord {
        version: version(5),
        ..listing
    };
    assert_eq!(
        save(&repositories, &listed_again, Some(version(4)), owner).await,
        Saved
    );
    enable(&repositories, room, owner, 4_000).await;
    shared(&repositories, room, owner, "Off00004", 4_000).await;
    hold(&database.migration, room, owner).await;
    assert_eq!(
        excerpt_room(&repositories, room).await,
        None,
        "平台拿掉的房间不能再分享"
    );
    assert_eq!(
        listed(&repositories, room, None).await.len(),
        1,
        "平台拿掉时和实况那一行一样留着，放回来照旧"
    );
    sqlx::query("DELETE FROM agent_room.public_room_platform_hold WHERE catalog_entry_id = $1")
        .bind(room.as_uuid())
        .execute(&database.migration)
        .await
        .expect("放回来");

    archive(&repositories, room, owner).await;
    assert!(listed(&repositories, room, None).await.is_empty(), "归档");

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 每天清掉过了保留期的_实况关了或开在第一句之后的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    // 每个账户最多一间公开房间：三间房三个房主。
    let owner = seed_principal(&database.runtime, "excerpt-prune").await;
    let short_owner = seed_principal(&database.runtime, "excerpt-prune-b").await;
    let late_owner = seed_principal(&database.runtime, "excerpt-prune-c").await;
    let (room, _) = listed_room(&repositories, owner, "Exc003").await;
    let (short, _) = listed_room(&repositories, short_owner, "Exc004").await;
    let (late, _) = listed_room(&repositories, late_owner, "Exc006").await;
    sqlx::query("UPDATE agent_room.room_catalog_entry SET retention_days = 7 WHERE id = $1")
        .bind(short.as_uuid())
        .execute(&database.migration)
        .await
        .expect("设保留期");
    let now = 100 * DAY;
    enable(&repositories, room, owner, DAY).await;
    enable(&repositories, short, short_owner, DAY).await;
    enable(&repositories, late, late_owner, now - DAY).await;

    // 默认 30 天：31 天前的删，29 天前的留着。
    shared(&repositories, room, owner, "Old00001", now - 31 * DAY).await;
    shared(&repositories, room, owner, "New00001", now - 29 * DAY).await;
    // 设了 7 天：8 天前的删。
    shared(&repositories, short, short_owner, "Old00002", now - 8 * DAY).await;
    shared(&repositories, short, short_owner, "New00002", now - 6 * DAY).await;
    // 没过保留期，可第一句在这次开实况以前（不知怎么留下的）。
    shared(&repositories, late, late_owner, "Stray001", now - 2 * DAY).await;

    assert_eq!(
        RoomExcerptStore::prune_excerpts(&repositories, time(now), 30)
            .await
            .expect("清得掉"),
        3
    );
    assert_eq!(codes(&repositories, room).await, ["New00001"]);
    assert_eq!(codes(&repositories, short).await, ["New00002"]);
    assert!(codes(&repositories, late).await.is_empty());

    // 实况那一行没了（不经关实况的那几处）也清掉。
    sqlx::query("DELETE FROM agent_room.room_live_view WHERE catalog_entry_id = $1")
        .bind(short.as_uuid())
        .execute(&database.migration)
        .await
        .expect("删实况那一行");
    assert_eq!(
        RoomExcerptStore::prune_excerpts(&repositories, time(now), 30)
            .await
            .expect("清得掉"),
        1
    );
    assert!(codes(&repositories, short).await.is_empty());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 导出里是自己分享的_只有码_房间_几句和时间() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "excerpt-export").await;
    let member = seed_principal(&database.runtime, "excerpt-export-b").await;
    let (room, _) = listed_room(&repositories, owner, "Exc005").await;
    enable(&repositories, room, owner, 1_000).await;
    assert_eq!(
        insert(
            &repositories,
            &excerpt("Export01", room, member, 2_000, 3_000),
            1_000
        )
        .await,
        InsertRoomExcerptOutcome::Inserted
    );

    let owner_export = AccountDeletionRepository::export(&repositories, owner, time(4_000))
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    assert_eq!(owner_export.data["roomExcerpts"], serde_json::json!([]));
    let member_export = AccountDeletionRepository::export(&repositories, member, time(4_000))
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    let exported = &member_export.data["roomExcerpts"][0];
    assert_eq!(exported["code"], "Export01");
    assert_eq!(exported["roomId"], room.as_uuid().to_string());
    assert_eq!(exported["messageCount"], 2);
    assert!(exported["firstMessageAt"].is_string());
    assert!(exported["sharedAt"].is_string());
    assert_eq!(
        exported.as_object().unwrap().len(),
        5,
        "不导出事件 ID，也没有话"
    );

    database.close().await;
}

async fn excerpt_room(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
) -> Option<ExcerptRoomRecord> {
    RoomExcerptStore::excerpt_room(repositories, room)
        .await
        .expect("可读")
}

fn excerpt(
    code: &str,
    room: RoomCatalogId,
    shared_by: PrincipalId,
    first_event_at: i64,
    shared_at: i64,
) -> RoomExcerptRecord {
    RoomExcerptRecord {
        code: RoomExcerptCode::parse(code).unwrap(),
        catalog_id: room,
        event_ids: vec![
            MatrixEventId::new("$first:matrix.test").unwrap(),
            MatrixEventId::new("$second:matrix.test").unwrap(),
        ],
        first_event_at: time(first_event_at),
        shared_by,
        shared_at: time(shared_at),
    }
}

async fn insert(
    repositories: &PostgresRepositories,
    excerpt: &RoomExcerptRecord,
    enabled_at: i64,
) -> InsertRoomExcerptOutcome {
    RoomExcerptStore::insert_excerpt(repositories, excerpt, time(enabled_at))
        .await
        .expect("写入不报错")
}

/// 记下一段，第一句在 `first_event_at`；实况开在那一刻或更早。
async fn shared(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    by: PrincipalId,
    code: &str,
    first_event_at: i64,
) {
    let enabled_at = RoomExcerptStore::excerpt_room(repositories, room)
        .await
        .expect("可读")
        .expect("放着实况")
        .enabled_at;
    assert_eq!(
        insert(
            repositories,
            &excerpt(code, room, by, first_event_at, first_event_at + 1_000),
            enabled_at.value()
        )
        .await,
        InsertRoomExcerptOutcome::Inserted
    );
}

async fn listed(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    shared_by: Option<PrincipalId>,
) -> Vec<RoomExcerptRecord> {
    RoomExcerptStore::room_excerpts(repositories, room, shared_by)
        .await
        .expect("可读")
        .excerpts
}

async fn codes(repositories: &PostgresRepositories, room: RoomCatalogId) -> Vec<String> {
    listed(repositories, room, None)
        .await
        .into_iter()
        .map(|excerpt| excerpt.code.as_str().to_owned())
        .collect()
}
