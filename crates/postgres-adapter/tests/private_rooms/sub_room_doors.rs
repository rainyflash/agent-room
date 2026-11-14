//! 子房间的存储（`specs/redesign/sub-rooms.md` 的 2d-1）：门和“不在名单里显示”跟名字、用途在同一个
//! 事务里存；第一次拆门记下那一刻，之后不往后挪；门已经不是读到的那样时不写；建的时候带的门存得
//! 下来、读得出来；没有门的藏不住，数据库自己也拦着。

use agent_room_application::ports::{
    PrivateRoomListingStore, SubRoomDoorChange, SubRoomRevision, SubRoomRevisionBlock,
};
use agent_room_domain::sub_rooms::SubRoomDoor;

use super::{membership::revise, *};

type Door = (RoomJoinMode, bool);

const OPEN: Door = (RoomJoinMode::Open, false);
const REQUEST: Door = (RoomJoinMode::Request, false);
const INVITE_ONLY: Door = (RoomJoinMode::InviteOnly, false);
const HIDDEN_REQUEST: Door = (RoomJoinMode::Request, true);
const HIDDEN_INVITE_ONLY: Door = (RoomJoinMode::InviteOnly, true);

/// 门的四样：哪种门、藏没藏、第一次拆门的时间、加没加密。
async fn door(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
) -> (RoomJoinMode, bool, Option<UtcMillis>, bool) {
    let link = link(repositories, catalog).await.expect("有父子关系");
    (
        link.access,
        link.hidden,
        link.door_opened_at,
        link.end_to_end_encrypted,
    )
}

/// 一间子房间的一次改动：名字照给的，用途清空，门从 `from` 换成 `to`。
fn revision(
    parent: RoomCatalogId,
    catalog_id: RoomCatalogId,
    name: &str,
    doors: Option<(Door, Door)>,
    at: UtcMillis,
) -> SubRoomRevision {
    let door = |(access, hidden)| SubRoomDoor::new(access, hidden).expect("门有效");
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
async fn 建的时候带的门存得下来_名单里读得出来_没有门的藏不住() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-door-create-owner").await;
    let parent = create_room(&repositories, owner).await;

    // 有门的一律加密、没有门的跟父房间是用例定的；这里只看存下来的是交给它的。
    let guarded = NewSubRoom {
        access: RoomJoinMode::Request,
        hidden: true,
        ..draft(&repositories, parent, owner, "Guarded", time(3)).await
    };
    let plain = NewSubRoom {
        end_to_end_encrypted: false,
        ..draft(&repositories, parent, owner, "Plain", time(5)).await
    };
    for sub_room in [&guarded, &plain] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (guarded, plain) = (
        guarded.snapshot.catalog().id(),
        plain.snapshot.catalog().id(),
    );
    assert_eq!(
        door(&repositories, guarded).await,
        (RoomJoinMode::Request, true, None, true)
    );
    assert_eq!(
        door(&repositories, plain).await,
        (RoomJoinMode::Open, false, None, false)
    );
    let listed = list(&repositories, parent, owner).await;
    assert_eq!(
        listed
            .iter()
            .map(|entry| (entry.name.as_str(), entry.access, entry.hidden))
            .collect::<Vec<_>>(),
        [
            ("Guarded", RoomJoinMode::Request, true),
            ("Plain", RoomJoinMode::Open, false)
        ]
    );

    // 没有门的藏不住：门的类型造不出这个样子，数据库自己也拦着。
    sqlx::query(
        "UPDATE agent_room.private_room_parent SET hidden = true WHERE catalog_entry_id = $1",
    )
    .bind(plain.as_uuid())
    .execute(&database.runtime)
    .await
    .expect_err("约束拦着");
    sqlx::query(
        "UPDATE agent_room.private_room_parent SET access = 'open' WHERE catalog_entry_id = $1",
    )
    .bind(guarded.as_uuid())
    .execute(&database.runtime)
    .await
    .expect_err("拆门不去掉隐藏，约束也拦着");

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 门和隐藏跟名字一起存_第一次拆门记下那一刻_再拆不往后挪_门变了的不写() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-door-owner").await;
    let parent = create_room(&repositories, owner).await;
    let plain = NewSubRoom {
        end_to_end_encrypted: false,
        ..draft(&repositories, parent, owner, "Plain", time(5)).await
    };
    let other = draft(&repositories, parent, owner, "Other", time(6)).await;
    for sub_room in [&plain, &other] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (plain, other) = (plain.snapshot.catalog().id(), other.snapshot.catalog().id());
    let change = |name: &str, doors, at| {
        let revision = revision(parent, plain, name, doors, at);
        let repositories = &repositories;
        async move { revise(repositories, &revision).await }
    };

    // 装门：门和隐藏一起存。不是拆门，不记时间；目录上的加密标记另有一步改。
    let installed = change("Plain", Some((OPEN, HIDDEN_INVITE_ONLY)), time(50)).await;
    assert_eq!(installed, None);
    assert_eq!(
        door(&repositories, plain).await,
        (RoomJoinMode::InviteOnly, true, None, false)
    );
    PrivateRoomListingStore::mark_room_encrypted(&repositories, plain)
        .await
        .expect("子房间的加密标记改得了");
    // 换一种门也不是拆门。
    let swapped = Some((HIDDEN_INVITE_ONLY, HIDDEN_REQUEST));
    assert_eq!(change("Plain", swapped, time(55)).await, None);
    assert_eq!(
        door(&repositories, plain).await,
        (RoomJoinMode::Request, true, None, true)
    );

    // 拆门：记下这一刻。加密还在。
    let opened = Some((HIDDEN_REQUEST, OPEN));
    assert_eq!(change("Plain", opened, time(60)).await, None);
    let first_opened = (RoomJoinMode::Open, false, Some(time(60)), true);
    assert_eq!(door(&repositories, plain).await, first_opened);

    // 再装上、再拆，没有门的时候只改名：那一刻都不往后挪。
    let reinstalled = Some((OPEN, INVITE_ONLY));
    assert_eq!(change("Plain", reinstalled, time(70)).await, None);
    assert_eq!(
        door(&repositories, plain).await,
        (RoomJoinMode::InviteOnly, false, Some(time(60)), true)
    );
    let reopened = Some((INVITE_ONLY, OPEN));
    assert_eq!(change("Plain", reopened, time(80)).await, None);
    assert_eq!(door(&repositories, plain).await, first_opened);
    assert_eq!(change("Renamed", None, time(90)).await, None);
    assert_eq!(door(&repositories, plain).await, first_opened);
    assert_eq!(load(&repositories, plain).await.catalog().name(), "Renamed");

    // 新门是照读到的旧门算的：存的时候门已经不是那样了（别人刚换过），什么都不写，名字也不写。
    let stale = Some((INVITE_ONLY, HIDDEN_REQUEST));
    assert_eq!(
        change("Stale", stale, time(95)).await,
        Some(SubRoomRevisionBlock::DoorChanged)
    );
    assert_eq!(door(&repositories, plain).await, first_opened);
    assert_eq!(load(&repositories, plain).await.catalog().name(), "Renamed");

    // 撞了名字的时候门也不写。
    assert_eq!(
        change("other", Some((OPEN, REQUEST)), time(96)).await,
        Some(SubRoomRevisionBlock::NameTaken)
    );
    assert_eq!(door(&repositories, plain).await, first_opened);

    // 别的子房间的门没被带着改。
    assert_eq!(
        door(&repositories, other).await,
        (RoomJoinMode::Open, false, None, true)
    );

    database.close().await;
}
