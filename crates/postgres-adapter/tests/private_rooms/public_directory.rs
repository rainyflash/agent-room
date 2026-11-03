//! 发现页的目录（`specs/public-rooms/design.md`）：只列公开着、还在用的房间；搜名字、简介和房主昵称
//! @ 前面那段，通配符照字面；按加入方式、最近有人说话、加不加密筛；三种排序；按分享码找详情；
//! 登录的人在每间房的处境；“最近有人说话”只给公开着的房间读、记的时间只往后走、不算改设置；
//! 在线人数问的是已加入的人，最近进来的在前；平台拿掉的房间目录、分享码、“最近有人说话”都不算，
//! 在等的申请清理时作废，放回去又照房主的设置列出来。
//!
//! 同一个库里别的测试也建公开房间，所以这里的房间名都带一个只属于这次的记号，按它搜。

use std::collections::BTreeMap;

use agent_room_application::ports::{
    JoinRequestRecord, PrivateRoomJoinRequestStore, PrivateRoomListingRecord,
    PrivateRoomListingStore, PublicRoomActivityStore, PublicRoomDirectoryStore,
    PublicRoomPlatformHold, PublicRoomPlatformHoldStore, PublicRoomRecord, PublicRoomSearch,
    PublicRoomSort, PublicRoomViewerState,
};
use agent_room_domain::{
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
    room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode},
};

use super::{
    listing::{Saved, archive, create_listed, create_room, record, save, settings},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 目录只列公开着的_三种排序() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;

    let all = search(&repositories, &token, PublicRoomSort::Active).await;
    assert_eq!(
        ids(&all),
        [rooms.open, rooms.request],
        "不公开的、归档的不列"
    );
    let open = &all[0];
    assert_eq!(open.name, format!("目录 {token} 读书会"));
    assert_eq!(open.owner_display_name, "测试主体 dir-open");
    assert_eq!((open.member_count, open.agent_count), (1, 0));
    assert_eq!(open.join_mode, RoomJoinMode::Open);
    assert!(!open.end_to_end_encrypted, "新建时就公开的不加密");
    assert_eq!(open.last_message_at, Some(time(9_000)));
    assert!(all[1].end_to_end_encrypted);
    assert_eq!(all[1].member_count, 2);
    assert_eq!(all[1].owner_display_name, "dir-request-owner@mail.test");

    let by_members = search(&repositories, &token, PublicRoomSort::Members).await;
    assert_eq!(ids(&by_members), [rooms.request, rooms.open]);
    let newest = search(&repositories, &token, PublicRoomSort::Newest).await;
    assert_eq!(ids(&newest), [rooms.request, rooms.open]);
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 目录能筛能翻页_搜的字照字面_房主只认_at_前面() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;
    let base = query(&token, PublicRoomSort::Active);

    let cases = [
        (
            PublicRoomSearch {
                join_mode: Some(RoomJoinMode::Request),
                ..base.clone()
            },
            vec![rooms.request],
        ),
        (
            PublicRoomSearch {
                active_since: Some(time(8_000)),
                ..base.clone()
            },
            vec![rooms.open],
        ),
        (
            PublicRoomSearch {
                unencrypted_only: true,
                ..base.clone()
            },
            vec![rooms.open],
        ),
        (
            PublicRoomSearch {
                offset: 1,
                limit: 1,
                ..base.clone()
            },
            vec![rooms.request],
        ),
        (with_text(&base, &format!("{token} 读书")), vec![rooms.open]),
        (with_text(&base, &format!("{token}%")), Vec::new()),
        (with_text(&base, "dir-request-owner"), vec![rooms.request]),
        (with_text(&base, "mail.test"), Vec::new()),
    ];
    for (search, expected) in cases {
        let found: Vec<_> = PublicRoomDirectoryStore::search_public_rooms(&repositories, &search)
            .await
            .expect("可读")
            .iter()
            .map(|room| room.catalog_id)
            .filter(|id| [rooms.open, rooms.request].contains(id))
            .collect();
        assert_eq!(found, expected, "{search:?}");
    }
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 网络_agent_只看到进得去的_关了网络_agent_的和只能邀请的不列() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let owner = seed_principal(&database.runtime, "dir-network").await;
    let open = listed_room(
        &repositories,
        owner,
        &token,
        RoomJoinMode::Open,
        true,
        1_000,
    )
    .await;
    let request = listed_room(
        &repositories,
        owner,
        &token,
        RoomJoinMode::Request,
        true,
        2_000,
    )
    .await;
    let closed = listed_room(
        &repositories,
        owner,
        &token,
        RoomJoinMode::Open,
        false,
        3_000,
    )
    .await;
    let invite_only = listed_room(
        &repositories,
        owner,
        &token,
        RoomJoinMode::InviteOnly,
        true,
        4_000,
    )
    .await;

    let everyone = search(&repositories, &token, PublicRoomSort::Newest).await;
    assert_eq!(ids(&everyone), [invite_only, closed, request, open]);
    let network = PublicRoomDirectoryStore::search_public_rooms(
        &repositories,
        &PublicRoomSearch {
            network_agents_only: true,
            ..query(&token, PublicRoomSort::Newest)
        },
    )
    .await
    .expect("可读");
    assert_eq!(ids(&network), [request, open]);
    database.close().await;
}

/// 一间公开的房间，名字带着这次的记号。
async fn listed_room(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    token: &str,
    join_mode: RoomJoinMode,
    allow_network_agents: bool,
    first_listed_at: i64,
) -> RoomCatalogId {
    let catalog_id = create_room(repositories, owner).await;
    let listing = listing_record(catalog_id, owner, join_mode, first_listed_at);
    let listing = PrivateRoomListingRecord {
        settings: RoomListingSettings::new(
            true,
            join_mode,
            RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
            allow_network_agents,
        )
        .unwrap(),
        ..listing
    };
    assert_eq!(save(repositories, &listing, None, owner).await, Saved);
    rename(
        repositories,
        catalog_id,
        &format!("目录 {token} {first_listed_at}"),
    )
    .await;
    catalog_id
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 按分享码找详情_带管理员_登录的人看得到自己的处境() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;

    let detail =
        PublicRoomDirectoryStore::public_room_by_share_code(&repositories, &rooms.request_code)
            .await
            .expect("可读")
            .expect("有这间房");
    assert_eq!(detail.room.catalog_id, rooms.request);
    assert_eq!(
        detail.admin_display_names,
        ["测试主体 dir-admin"],
        "房主不算"
    );
    for code in [&rooms.archived_code, &rooms.unlisted_code] {
        assert_eq!(
            PublicRoomDirectoryStore::public_room_by_share_code(&repositories, code)
                .await
                .expect("可读"),
            None
        );
    }

    let states: BTreeMap<_, _> = PublicRoomDirectoryStore::public_room_viewer_states(
        &repositories,
        rooms.admin,
        &[rooms.open, rooms.request],
    )
    .await
    .expect("可读")
    .into_iter()
    .collect();
    assert_eq!(
        states,
        BTreeMap::from([
            (rooms.open, PublicRoomViewerState::Pending),
            (rooms.request, PublicRoomViewerState::Joined),
        ])
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 最近说话只读公开着的房间_时间只往后走_不动设置的版本() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;

    let listed: BTreeMap<_, _> = PublicRoomActivityStore::listed_rooms_for_activity(&repositories)
        .await
        .expect("可读")
        .into_iter()
        .collect();
    for (room, expected) in [
        (rooms.open, true),
        (rooms.request, true),
        (rooms.archived, false),
        (rooms.unlisted, false),
    ] {
        assert_eq!(listed.contains_key(&room), expected, "{room:?}");
    }
    let open_room = PrivateRoomStore::find_by_catalog(&repositories, rooms.open)
        .await
        .expect("可读")
        .expect("有房间");
    assert_eq!(
        listed[&rooms.open].as_str(),
        open_room.instance().matrix_room_id().as_str()
    );

    let before = listing(&repositories, rooms.open).await;
    for (at, expected) in [(5_000, 9_000), (12_000, 12_000)] {
        PublicRoomActivityStore::record_last_message(&repositories, rooms.open, time(at))
            .await
            .expect("写入");
        let found = search(&repositories, &token, PublicRoomSort::Active).await;
        assert_eq!(found[0].last_message_at, Some(time(expected)), "{at}");
    }
    PublicRoomActivityStore::record_last_message(&repositories, rooms.request, time(1))
        .await
        .expect("还没说过话的也能写");
    let after = listing(&repositories, rooms.open).await;
    assert_eq!(after, before, "版本、改的人、改的时间都不动");
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 在线人数问已加入的人_最近进来的在前_在等批准的不问() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;
    let pool = &database.runtime;
    let admin = matrix_user(pool, rooms.admin).await;
    let request_owner = matrix_user(pool, rooms.request_owner).await;
    let open_owner = matrix_user(pool, rooms.open_owner).await;

    for (room, limit, expected) in [
        (rooms.request, 10, vec![admin.clone(), request_owner]),
        (rooms.request, 1, vec![admin]),
        (rooms.open, 10, vec![open_owner]),
    ] {
        let found: Vec<String> =
            PublicRoomDirectoryStore::public_room_presence_candidates(&repositories, room, limit)
                .await
                .expect("可读")
                .iter()
                .map(|user| user.as_str().to_owned())
                .collect();
        assert_eq!(found, expected, "{room:?} {limit}");
    }
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 平台拿掉的房间不列_分享码找不到_不读最近说话_申请清理时作废_放回去又列() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let token = Uuid::now_v7().simple().to_string();
    let rooms = seed_rooms(&database, &repositories, &token).await;
    let moderator = seed_principal(&database.runtime, "dir-moderator").await;
    let before = listing(&repositories, rooms.open).await;

    let hold = PublicRoomPlatformHold {
        catalog_id: rooms.open,
        held_by: moderator,
        reason: "冒充官方客服".to_owned(),
        held_at: time(20_000),
    };
    PublicRoomPlatformHoldStore::place_platform_hold(&repositories, &hold)
        .await
        .expect("拿掉");
    // 再拿一次：原因换成这一次的，时间不往回走。
    let again = PublicRoomPlatformHold {
        reason: "换个原因".to_owned(),
        held_at: time(10_000),
        ..hold.clone()
    };
    PublicRoomPlatformHoldStore::place_platform_hold(&repositories, &again)
        .await
        .expect("再拿一次");
    assert_eq!(
        PublicRoomPlatformHoldStore::platform_hold(&repositories, rooms.open)
            .await
            .expect("可读"),
        Some(PublicRoomPlatformHold {
            reason: "换个原因".to_owned(),
            ..hold
        })
    );

    let found = search(&repositories, &token, PublicRoomSort::Active).await;
    assert_eq!(ids(&found), [rooms.request]);
    assert_eq!(
        PublicRoomDirectoryStore::public_room_by_share_code(&repositories, &rooms.open_code)
            .await
            .expect("可读"),
        None
    );
    let active = PublicRoomActivityStore::listed_rooms_for_activity(&repositories)
        .await
        .expect("可读");
    assert!(!active.iter().any(|(room, _)| *room == rooms.open));
    assert_eq!(
        listing(&repositories, rooms.open).await,
        before,
        "房主的设置不动"
    );

    PrivateRoomJoinRequestStore::sweep_join_requests(&repositories, time(30_000), time(1))
        .await
        .expect("清理");
    assert_eq!(
        PrivateRoomJoinRequestStore::pending_join_request(&repositories, rooms.open, rooms.admin)
            .await
            .expect("可读"),
        None,
        "拿掉的房间里在等的作废"
    );

    assert!(
        PublicRoomPlatformHoldStore::release_platform_hold(&repositories, rooms.open)
            .await
            .expect("放回去")
    );
    assert!(
        !PublicRoomPlatformHoldStore::release_platform_hold(&repositories, rooms.open)
            .await
            .expect("再放一次"),
        "本来就没拿掉"
    );
    let found = search(&repositories, &token, PublicRoomSort::Active).await;
    assert_eq!(ids(&found), [rooms.open, rooms.request]);
    assert!(
        PublicRoomDirectoryStore::public_room_by_share_code(&repositories, &rooms.open_code)
            .await
            .expect("可读")
            .is_some()
    );
    database.close().await;
}

async fn matrix_user(pool: &PgPool, principal: PrincipalId) -> String {
    sqlx::query_scalar("SELECT matrix_user_id FROM agent_room.principal WHERE id = $1")
        .bind(principal.as_uuid())
        .fetch_one(pool)
        .await
        .expect("有这个主体")
}

async fn listing(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
) -> PrivateRoomListingRecord {
    PrivateRoomListingStore::listing(repositories, catalog_id)
        .await
        .expect("可读")
        .expect("有公开设置")
}

struct SeededRooms {
    open: RoomCatalogId,
    request: RoomCatalogId,
    archived: RoomCatalogId,
    unlisted: RoomCatalogId,
    open_owner: PrincipalId,
    request_owner: PrincipalId,
    open_code: RoomShareCode,
    request_code: RoomShareCode,
    archived_code: RoomShareCode,
    unlisted_code: RoomShareCode,
    admin: PrincipalId,
}

/// 一间新建时就公开、直接进、刚有人说过话的；一间加密的、要申请、多一位管理员、后公开的；
/// 一间公开以后归档的；一间改回私密的。管理员在第一间申请着。
async fn seed_rooms(
    database: &TestDatabase,
    repositories: &PostgresRepositories,
    token: &str,
) -> SeededRooms {
    let pool = &database.runtime;
    let open_owner = seed_principal(pool, "dir-open").await;
    let open = snapshot(RoomCatalogId::from_uuid(Uuid::now_v7()), open_owner);
    let open_listing = listing_record(open.catalog().id(), open_owner, RoomJoinMode::Open, 1_000);
    assert_eq!(
        create_listed(repositories, &open, &open_listing).await,
        Saved
    );
    rename(
        repositories,
        open.catalog().id(),
        &format!("目录 {token} 读书会"),
    )
    .await;
    sqlx::query(
        r"UPDATE agent_room.private_room_listing
             SET last_message_at = to_timestamp($2::double precision / 1000.0)
           WHERE catalog_entry_id = $1",
    )
    .bind(open.catalog().id().as_uuid())
    .bind(time(9_000).value())
    .execute(pool)
    .await
    .expect("写入最近说话");

    let request_owner = seed_principal(pool, "dir-request").await;
    // 早期没填昵称的账户，昵称就是邮箱。
    sqlx::query("UPDATE agent_room.principal SET display_name = $2 WHERE id = $1")
        .bind(request_owner.as_uuid())
        .bind("dir-request-owner@mail.test")
        .execute(pool)
        .await
        .expect("改昵称");
    let admin = seed_principal(pool, "dir-admin").await;
    let request = create_room(repositories, request_owner).await;
    let request_listing = listing_record(request, request_owner, RoomJoinMode::Request, 2_000);
    assert_eq!(
        save(repositories, &request_listing, None, request_owner).await,
        Saved
    );
    rename(repositories, request, &format!("目录 {token} 夜谈")).await;
    add_admin(repositories, request, request_owner, admin).await;

    let archived_owner = seed_principal(pool, "dir-archived").await;
    let archived = create_room(repositories, archived_owner).await;
    let archived_listing = listing_record(archived, archived_owner, RoomJoinMode::Open, 1_000);
    assert_eq!(
        save(repositories, &archived_listing, None, archived_owner).await,
        Saved
    );
    rename(repositories, archived, &format!("目录 {token} 归档")).await;
    archive(repositories, archived, archived_owner).await;

    let unlisted_owner = seed_principal(pool, "dir-unlisted").await;
    let unlisted = create_room(repositories, unlisted_owner).await;
    let unlisted_listing = PrivateRoomListingRecord {
        settings: settings(false, RoomJoinMode::Open),
        ..listing_record(unlisted, unlisted_owner, RoomJoinMode::Open, 1_000)
    };
    assert_eq!(
        save(repositories, &unlisted_listing, None, unlisted_owner).await,
        Saved
    );
    rename(repositories, unlisted, &format!("目录 {token} 私密")).await;

    PrivateRoomJoinRequestStore::create_join_request(
        repositories,
        &JoinRequestRecord {
            catalog_id: open.catalog().id(),
            requester: admin,
            message: JoinRequestMessage::default(),
            status: JoinRequestStatus::Pending,
            requested_at: time(3_000),
            decided_at: None,
            decided_by: None,
        },
    )
    .await
    .expect("申请");

    SeededRooms {
        open: open.catalog().id(),
        request,
        archived,
        unlisted,
        open_owner,
        request_owner,
        open_code: open_listing.share_code,
        request_code: request_listing.share_code,
        archived_code: archived_listing.share_code,
        unlisted_code: unlisted_listing.share_code,
        admin,
    }
}

fn listing_record(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    join_mode: RoomJoinMode,
    first_listed_at: i64,
) -> PrivateRoomListingRecord {
    PrivateRoomListingRecord {
        settings: settings(true, join_mode),
        share_code: RoomShareCode::from_entropy(*Uuid::now_v7().as_bytes()),
        first_listed_at: time(first_listed_at),
        updated_at: time(first_listed_at),
        ..record(catalog_id, owner, true, "Dir000", 1)
    }
}

async fn rename(repositories: &PostgresRepositories, catalog_id: RoomCatalogId, name: &str) {
    PrivateRoomStore::rename(repositories, catalog_id, name, time(5))
        .await
        .expect("改名");
}

async fn add_admin(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    admin: PrincipalId,
) {
    let snapshot = PrivateRoomStore::find_by_catalog(repositories, catalog_id)
        .await
        .expect("可读")
        .expect("有房间");
    // 存一次只能往前走一个版本：邀请、接受各存一次。
    let mut room = snapshot.room().clone();
    room.invite(owner, admin, PrivateRoomPermissions::ALL)
        .expect("房主能邀请管理员");
    PrivateRoomStore::save(repositories, &room, snapshot.room().version(), time(20))
        .await
        .expect("存上邀请");
    let invited = room.version();
    room.accept_invitation(admin).expect("管理员接受");
    PrivateRoomStore::save(repositories, &room, invited, time(21))
        .await
        .expect("存上接受");
}

async fn search(
    repositories: &PostgresRepositories,
    token: &str,
    sort: PublicRoomSort,
) -> Vec<PublicRoomRecord> {
    PublicRoomDirectoryStore::search_public_rooms(repositories, &query(token, sort))
        .await
        .expect("可读")
}

fn query(token: &str, sort: PublicRoomSort) -> PublicRoomSearch {
    PublicRoomSearch {
        text: Some(token.to_owned()),
        join_mode: None,
        active_since: None,
        unencrypted_only: false,
        network_agents_only: false,
        sort,
        offset: 0,
        limit: 10,
    }
}

fn with_text(base: &PublicRoomSearch, text: &str) -> PublicRoomSearch {
    PublicRoomSearch {
        text: Some(text.to_owned()),
        ..base.clone()
    }
}

fn ids(rooms: &[PublicRoomRecord]) -> Vec<RoomCatalogId> {
    rooms.iter().map(|room| room.catalog_id).collect()
}
