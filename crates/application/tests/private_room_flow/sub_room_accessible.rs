//! 子房间第 2f-1b 步（`specs/redesign/sub-rooms.md` 的“接口”）：给 Bridge 的房间列表带子房间。
//! 只列账号的 Agent 进得去的父房间下面的：看得到的、没归档的、没被封禁的；有门、账号又不在里面的
//! 也列（Agent 照着进会被告诉要主人先申请）；分类写名字；子房间自己不当父房间。
//!
//! 接口那一头（`/lobbies/accessible?subRooms=1` 的回答长什么样）在 control-plane 的
//! `features/lobbies/tests/sub_rooms.rs`。

use agent_room_application::{
    ports::PrivateRoomSnapshot,
    private_rooms::{AccountSubRoomListing, CreateSubRoomCategory, ReviseSubRoom},
};
use agent_room_domain::ids::SubRoomCategoryId;

use super::*;

impl Rooms {
    /// 存着的那份快照，和 `list_for_account` 给 Bridge 的一样。
    fn stored(&self, catalog: u128) -> PrivateRoomSnapshot {
        self.store.rooms.lock().expect("房间锁正常")[&catalog_id(catalog)].clone()
    }

    /// 列出来的：（父房间、子房间、分类）。
    async fn accessible(
        &self,
        viewer: u128,
        parents: &[u128],
    ) -> Vec<(RoomCatalogId, RoomCatalogId, Option<String>)> {
        let parents: Vec<PrivateRoomSnapshot> = parents
            .iter()
            .map(|catalog| self.stored(*catalog))
            .collect();
        self.service
            .accessible_sub_rooms(principal_id(viewer), &parents)
            .await
            .expect("读得出来")
            .into_iter()
            .map(|sub_room| {
                (
                    sub_room.parent_catalog_id,
                    sub_room.entry.catalog_id,
                    sub_room.category,
                )
            })
            .collect()
    }
}

#[tokio::test]
async fn 看得到的都列_有门的也列_藏起来的只有进去过的和管父房间的看得到_归档和被封禁的不列() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(21, "design")
    });
    rooms.store.seed(&Seed {
        access: RoomJoinMode::InviteOnly,
        hidden: true,
        ..Seed::open(22, "secret")
    });
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(23, "old")
    });
    rooms.store.seed(&Seed::open(24, "banned"));
    rooms.store.change(24, |room| {
        room.admit(
            principal_id(MEMBER),
            permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Speak]),
        )
        .expect("进来过");
        room.ban_member(principal_id(OWNER), principal_id(MEMBER))
            .expect("房主能封禁");
    });

    let parent = catalog_id(PARENT);
    assert_eq!(
        rooms.accessible(MEMBER, &[PARENT]).await,
        [
            (parent, catalog_id(20), None),
            (parent, catalog_id(21), None)
        ],
        "要批准的也列；藏起来的他没进过；归档的、他被封禁的都不列"
    );
    assert_eq!(
        rooms.accessible(OWNER, &[PARENT]).await,
        [
            (parent, catalog_id(20), None),
            (parent, catalog_id(21), None),
            (parent, catalog_id(22), None),
            (parent, catalog_id(24), None),
        ],
        "房主管着父房间，藏起来的也看得到"
    );
}

#[tokio::test]
async fn 分类写名字_未分类的不写() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.store.seed(&Seed::open(21, "design"));
    let ship = SubRoomCategoryId::from_uuid(uuid(90));
    rooms
        .service
        .create_sub_room_category(CreateSubRoomCategory {
            actor: person(OWNER),
            parent_catalog_id: catalog_id(PARENT),
            category_id: ship,
            name: "Ship".to_owned(),
        })
        .await
        .expect("房主能建分类");
    rooms
        .service
        .revise_sub_room(ReviseSubRoom {
            actor: person(OWNER),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(21),
            name: None,
            description: None,
            access: None,
            hidden: None,
            category: Some(Some(ship)),
        })
        .await
        .expect("房主能挪分类");

    let parent = catalog_id(PARENT);
    assert_eq!(
        rooms.accessible(MEMBER, &[PARENT]).await,
        [
            (parent, catalog_id(20), None),
            (parent, catalog_id(21), Some("Ship".to_owned())),
        ]
    );
}

#[tokio::test]
async fn 只看_agent_进得去的父房间_子房间自己不当父房间() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));

    // 只能看的人的 Agent 进不去父房间，下面的子房间也不列。
    assert!(rooms.accessible(VIEWER, &[PARENT]).await.is_empty());
    // 交进来的是子房间本身时，不去找它下面的。
    rooms.join(MEMBER, 20).await.expect("成员能进");
    assert!(rooms.accessible(MEMBER, &[20]).await.is_empty());
}
