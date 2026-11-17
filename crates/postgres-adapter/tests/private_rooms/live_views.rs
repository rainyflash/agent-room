//! 分享页放实况（`specs/redesign/project-rooms.md`“分享页放实况”，3P-6a）：公开、直接进、不加密、在用、
//! 没被平台拿掉的房间才记得下；已经开着的再开不换那一刻；改成要申请、改回私密、归档时在同一个事务里
//! 关掉，再开是新的一刻；平台拿掉时不删；导出里有。删账户时关掉在 `real_database.rs` 的删除那一条里查，
//! 项目房间建好就开着在 `repository_claims.rs` 里查。分享页按分享码找放着实况的房间（3P-6a-2）：
//! 只认开着、没被拿掉的主房间。

use agent_room_application::ports::{
    AccountDeletionRepository, EnableRoomLiveViewOutcome, LiveRoomLookup, LiveRoomRecord,
    PrivateRoomListingRecord, PrivateRoomListingStore, RoomLiveViewFacts, RoomLiveViewRecord,
    RoomLiveViewStore,
};
use agent_room_domain::room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode};

use super::{
    listing::{Saved, archive, create_listed, create_room, record, save, settings, version},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 公开直接进不加密的房间能打开_再开不换那一刻_关掉就没了() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-open").await;
    let manager = seed_principal(&database.runtime, "live-open-b").await;
    let (room, _) = listed_room(&repositories, owner, "Liv001").await;

    assert_eq!(
        facts(&repositories, room).await,
        RoomLiveViewFacts {
            listing: Some((true, RoomJoinMode::Open)),
            end_to_end_encrypted: false,
            platform_hold: false,
            live_view: None,
        }
    );
    let opened = live(owner, 2_000);
    assert_eq!(
        enable(&repositories, room, owner, 2_000).await,
        EnableRoomLiveViewOutcome::Enabled(opened)
    );
    assert_eq!(
        enable(&repositories, room, manager, 3_000).await,
        EnableRoomLiveViewOutcome::AlreadyEnabled(opened),
        "开着的再开不换人、不换那一刻"
    );
    assert_eq!(facts(&repositories, room).await.live_view, Some(opened));

    for _ in 0..2 {
        RoomLiveViewStore::disable_live_view(&repositories, room)
            .await
            .expect("关掉，重复也行");
        assert_eq!(facts(&repositories, room).await.live_view, None);
    }
    assert_eq!(
        enable(&repositories, room, manager, 4_000).await,
        EnableRoomLiveViewOutcome::Enabled(live(manager, 4_000)),
        "再开是新的一刻"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 不公开_加密_要申请_平台拿掉_关了的房间都记不下() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-blocked").await;
    let other = seed_principal(&database.runtime, "live-blocked-b").await;

    // 私人房间：没公开过、加密。
    let private = create_room(&repositories, owner).await;
    assert_eq!(
        facts(&repositories, private).await,
        RoomLiveViewFacts {
            listing: None,
            end_to_end_encrypted: true,
            platform_hold: false,
            live_view: None,
        }
    );
    assert_not_recorded(&repositories, private, owner).await;

    // 公开、不加密，但要申请才进得来。
    let (requested, listing) = listed_room(&repositories, other, "Liv002").await;
    let request = PrivateRoomListingRecord {
        settings: settings(true, RoomJoinMode::Request),
        version: version(2),
        ..listing
    };
    assert_eq!(
        save(&repositories, &request, Some(version(1)), other).await,
        Saved
    );
    assert_eq!(
        facts(&repositories, requested).await.listing,
        Some((true, RoomJoinMode::Request))
    );
    assert_not_recorded(&repositories, requested, other).await;

    // 被平台拿掉了；拿掉之前开着的那一行留着，只是不放。
    let held_owner = seed_principal(&database.runtime, "live-blocked-c").await;
    let (held, _) = listed_room(&repositories, held_owner, "Liv003").await;
    enable(&repositories, held, held_owner, 1_000).await;
    hold(&database.migration, held, held_owner).await;
    let held_facts = facts(&repositories, held).await;
    assert!(held_facts.platform_hold);
    assert_eq!(held_facts.live_view, Some(live(held_owner, 1_000)));
    RoomLiveViewStore::disable_live_view(&repositories, held)
        .await
        .expect("关掉");
    assert_not_recorded(&repositories, held, held_owner).await;

    // 公开着的房间归档了。
    let archiving_owner = seed_principal(&database.runtime, "live-blocked-d").await;
    let (closed, _) = listed_room(&repositories, archiving_owner, "Liv004").await;
    archive(&repositories, closed, archiving_owner).await;
    assert_not_recorded(&repositories, closed, archiving_owner).await;

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 改成只能邀请_改回私密_归档时一起关掉_别的设置改了照旧开着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-off").await;
    let (room, listing) = listed_room(&repositories, owner, "Liv005").await;
    enable(&repositories, room, owner, 1_000).await;

    // 只改了要不要 Agent，加入方式还是直接进：照旧开着。
    let agents_off = PrivateRoomListingRecord {
        settings: RoomListingSettings::new(
            true,
            RoomJoinMode::Open,
            RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
            false,
        )
        .unwrap(),
        version: version(2),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &agents_off, Some(version(1)), owner).await,
        Saved
    );
    assert_eq!(
        facts(&repositories, room).await.live_view,
        Some(live(owner, 1_000))
    );

    let invite_only = PrivateRoomListingRecord {
        settings: settings(true, RoomJoinMode::InviteOnly),
        version: version(3),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &invite_only, Some(version(2)), owner).await,
        Saved
    );
    assert_eq!(facts(&repositories, room).await.live_view, None);

    let open_again = PrivateRoomListingRecord {
        version: version(4),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &open_again, Some(version(3)), owner).await,
        Saved
    );
    assert_eq!(
        facts(&repositories, room).await.live_view,
        None,
        "改回直接进不会自己开回来"
    );
    enable(&repositories, room, owner, 2_000).await;
    let private_again = PrivateRoomListingRecord {
        settings: settings(false, RoomJoinMode::Open),
        version: version(5),
        ..listing.clone()
    };
    assert_eq!(
        save(&repositories, &private_again, Some(version(4)), owner).await,
        Saved
    );
    assert_eq!(facts(&repositories, room).await.live_view, None);

    let listed_again = PrivateRoomListingRecord {
        version: version(6),
        ..listing
    };
    assert_eq!(
        save(&repositories, &listed_again, Some(version(5)), owner).await,
        Saved
    );
    enable(&repositories, room, owner, 3_000).await;
    archive(&repositories, room, owner).await;
    assert_eq!(facts(&repositories, room).await.live_view, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 目录上记成加密时一起关掉() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-sealed").await;
    let (room, _) = listed_room(&repositories, owner, "Liv007").await;
    enable(&repositories, room, owner, 1_000).await;

    PrivateRoomListingStore::mark_room_encrypted(&repositories, room)
        .await
        .expect("改标记成功");
    let sealed = facts(&repositories, room).await;
    assert!(sealed.end_to_end_encrypted);
    assert_eq!(sealed.live_view, None);
    assert_not_recorded(&repositories, room, owner).await;

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 导出里是开的人那一份() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-export").await;
    let manager = seed_principal(&database.runtime, "live-export-b").await;
    let (room, _) = listed_room(&repositories, owner, "Liv006").await;
    enable(&repositories, room, manager, 1_000).await;

    let owner_export = AccountDeletionRepository::export(&repositories, owner, time(2_000))
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    assert_eq!(owner_export.data["liveViews"], serde_json::json!([]));
    let manager_export = AccountDeletionRepository::export(&repositories, manager, time(2_000))
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    assert_eq!(
        manager_export.data["liveViews"][0]["roomId"],
        room.as_uuid().to_string(),
        "导出里是开的人那一份"
    );
    assert!(manager_export.data["liveViews"][0]["enabledAt"].is_string());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 分享码只认此刻放着实况的主房间() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "live-lookup").await;
    let (room, _) = listed_room(&repositories, owner, "Liv008").await;

    assert_eq!(lookup(&repositories, "Liv008").await, None, "还没开");
    enable(&repositories, room, owner, 2_000).await;
    let found = lookup(&repositories, "Liv008").await.expect("开着就认得出");
    assert_eq!(found.catalog_id, room);
    assert_eq!(found.enabled_at, time(2_000));
    assert!(found.matrix_room_id.as_str().starts_with("!private"));
    assert_eq!(lookup(&repositories, "Liv00X").await, None, "没有这个码");

    // 挂到别的房间下面成了子房间：子房间一概不放。
    let parent = create_room(&repositories, owner).await;
    sqlx::query(
        r"INSERT INTO agent_room.private_room_parent (
               catalog_entry_id, parent_catalog_entry_id, name_key, access, hidden,
               created_by_principal_id, parent_version_applied, created_at
           ) VALUES ($1, $2, 'live', 'open', false, $3, 0, statement_timestamp())",
    )
    .bind(room.as_uuid())
    .bind(parent.as_uuid())
    .bind(owner.as_uuid())
    .execute(&database.migration)
    .await
    .expect("挂到别的房间下面");
    assert_eq!(lookup(&repositories, "Liv008").await, None, "子房间不放");
    sqlx::query("DELETE FROM agent_room.private_room_parent WHERE catalog_entry_id = $1")
        .bind(room.as_uuid())
        .execute(&database.migration)
        .await
        .expect("摘下来");
    assert!(lookup(&repositories, "Liv008").await.is_some());

    hold(&database.migration, room, owner).await;
    assert_eq!(
        lookup(&repositories, "Liv008").await,
        None,
        "平台拿掉的不放，开着的那一行还在"
    );
    assert!(facts(&repositories, room).await.live_view.is_some());

    database.close().await;
}

async fn lookup(repositories: &PostgresRepositories, share_code: &str) -> Option<LiveRoomRecord> {
    LiveRoomLookup::live_room(repositories, &RoomShareCode::parse(share_code).unwrap())
        .await
        .expect("可读")
}

/// 新建时就公开、直接进、不加密的房间。
pub(super) async fn listed_room(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    share_code: &str,
) -> (RoomCatalogId, PrivateRoomListingRecord) {
    let snapshot = snapshot(RoomCatalogId::from_uuid(Uuid::now_v7()), owner);
    let listing = record(snapshot.catalog().id(), owner, true, share_code, 1);
    assert_eq!(
        create_listed(repositories, &snapshot, &listing).await,
        Saved
    );
    (snapshot.catalog().id(), listing)
}

pub(super) async fn facts(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
) -> RoomLiveViewFacts {
    RoomLiveViewStore::live_view_facts(repositories, room)
        .await
        .expect("可读")
        .expect("有房间")
}

pub(super) async fn enable(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    by: PrincipalId,
    at: i64,
) -> EnableRoomLiveViewOutcome {
    RoomLiveViewStore::enable_live_view(repositories, room, by, time(at))
        .await
        .expect("写入不报错")
}

async fn assert_not_recorded(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    by: PrincipalId,
) {
    assert_eq!(
        enable(repositories, room, by, 1_000).await,
        EnableRoomLiveViewOutcome::NotEligible
    );
    assert_eq!(facts(repositories, room).await.live_view, None);
}

pub(super) async fn hold(pool: &PgPool, room: RoomCatalogId, by: PrincipalId) {
    sqlx::query(
        r"INSERT INTO agent_room.public_room_platform_hold (catalog_entry_id, held_by, held_at)
          VALUES ($1, $2, now())",
    )
    .bind(room.as_uuid())
    .bind(by.as_uuid())
    .execute(pool)
    .await
    .expect("记下平台拿掉");
}

fn live(by: PrincipalId, at: i64) -> RoomLiveViewRecord {
    RoomLiveViewRecord {
        enabled_by: by,
        enabled_at: time(at),
    }
}
