//! 子房间第 2e-3 步（`specs/redesign/sub-rooms.md` 的“归档和恢复”“看得到什么”）：“已归档”的名单。
//! 只有归档了的，最近归档的在前；归档以前看得到它的人，归档以后也看得到这一行；每一行写着看的人能不能
//! 恢复，和恢复本身同一套规矩。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份；真实存储读这几列的测试在 `postgres-adapter` 的
//! `tests/private_rooms/sub_room_archive.rs`。

use agent_room_application::private_rooms::{ListSubRooms, SubRoomView};

use super::*;

impl Rooms {
    async fn archived_list(&self, viewer: u128) -> Vec<SubRoomView> {
        self.service
            .list_sub_rooms(ListSubRooms {
                actor: person(viewer),
                catalog_id: catalog_id(PARENT),
                archived: true,
            })
            .await
            .expect("父房间里的人读得到")
            .sub_rooms
    }
}

/// 名单里的房间号和这一行给不给“恢复”。
fn rows(views: &[SubRoomView]) -> Vec<(RoomCatalogId, bool)> {
    views
        .iter()
        .map(|view| (view.entry.catalog_id, view.restorable))
        .collect()
}

fn archived_at(rooms: &Rooms, catalog: u128, at: i64, reason: SubRoomArchiveReason) {
    rooms.store.archive(catalog);
    rooms.store.set_archive_record(
        catalog_id(catalog),
        Some(SubRoomArchiveRecord {
            at: UtcMillis::new(at).expect("时间有效"),
            by: (reason == SubRoomArchiveReason::Manual).then(|| principal_id(OWNER)),
            reason,
        }),
    );
}

#[tokio::test]
async fn 已归档的名单只有归档了的_最近归档的在前_没归档的名单里没有它们() {
    let rooms = Rooms::new();
    for (catalog, name) in [(20, "release"), (21, "design"), (22, "notes")] {
        rooms.store.seed(&Seed::open(catalog, name));
    }
    rooms.join(MEMBER, 20).await.expect("成员能进");
    archived_at(&rooms, 20, 2_000, SubRoomArchiveReason::Manual);
    archived_at(&rooms, 21, 3_000, SubRoomArchiveReason::Manual);

    let listed = rooms.list(OWNER, PARENT).await.expect("房主读得到");
    let active: Vec<RoomCatalogId> = listed
        .sub_rooms
        .iter()
        .map(|view| view.entry.catalog_id)
        .collect();
    assert_eq!(active, [catalog_id(22)], "归档了的不在平常的名单里");
    assert!(listed.sub_rooms.iter().all(|view| !view.restorable));

    let archived = rooms.archived_list(OWNER).await;
    assert_eq!(
        rows(&archived),
        [(catalog_id(21), true), (catalog_id(20), true)],
        "最近归档的在前；房主管得了每一间，都能恢复"
    );
    assert_eq!(
        archived[1].entry.archive_record,
        Some(SubRoomArchiveRecord {
            at: UtcMillis::new(2_000).expect("时间有效"),
            by: Some(principal_id(OWNER)),
            reason: SubRoomArchiveReason::Manual,
        }),
        "带着谁、什么时候、为什么归档的"
    );

    // 有人收起来的，管不了这一间的人拿不回来；自动归档的，原来在里面的人能。
    assert_eq!(
        rows(&rooms.archived_list(MEMBER).await),
        [(catalog_id(21), false), (catalog_id(20), false)]
    );
    archived_at(&rooms, 20, 2_000, SubRoomArchiveReason::Inactive);
    archived_at(&rooms, 21, 3_000, SubRoomArchiveReason::Inactive);
    assert_eq!(
        rows(&rooms.archived_list(MEMBER).await),
        [(catalog_id(21), false), (catalog_id(20), true)],
        "他进过 20 号、没进过 21 号"
    );
}

#[tokio::test]
async fn 藏起来的归档以后也只有原来看得到的人看得到_父房间归档以后谁都恢复不了() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        hidden: true,
        ..Seed::open(23, "secret")
    });
    rooms.store.seed(&Seed::open(20, "release"));
    archived_at(&rooms, 23, 2_000, SubRoomArchiveReason::Manual);
    archived_at(&rooms, 20, 3_000, SubRoomArchiveReason::Manual);

    assert_eq!(
        rows(&rooms.archived_list(MEMBER).await),
        [(catalog_id(20), false)],
        "没进过藏起来的那一间，归档以后也看不到它"
    );
    assert_eq!(
        rows(&rooms.archived_list(ADMIN).await),
        [(catalog_id(20), true), (catalog_id(23), true)],
        "父房间的管理员管着每一间，藏起来的也看得到、拿得回来"
    );

    // 跟着父房间归档的回不来。
    archived_at(&rooms, 20, 3_000, SubRoomArchiveReason::Parent);
    assert_eq!(
        rows(&rooms.archived_list(OWNER).await),
        [(catalog_id(20), false), (catalog_id(23), true)]
    );
    rooms.store.archive(PARENT);
    assert_eq!(
        rows(&rooms.archived_list(OWNER).await),
        [(catalog_id(20), false), (catalog_id(23), false)],
        "父房间归档了，哪一间都拿不回来"
    );
}
