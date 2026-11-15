//! 子房间第 2d-5 步（`specs/redesign/sub-rooms.md` 的“加密和谁读得到”）：建的当口父房间公开着，或者
//! 刚改回私密、刚公开，存好以后照它补上。父房间改公开设置时带着子房间走，在
//! `private_room_listing_flow`。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份，新建的子房间在 Matrix 替身上都是 `!private:matrix.test`。

use agent_room_application::ports::MatrixRoomEncryption;

use super::{membership::refusal, *};

const CREATED: &str = "!private:matrix.test";

fn listed_lines(rooms: &Rooms) -> Vec<String> {
    rooms
        .matrix
        .changes_in(CREATED)
        .into_iter()
        .filter(|change| change.starts_with("listed"))
        .collect()
}

#[tokio::test]
async fn 父房间公开着建的没有门的写下这一刻_有门的不写() {
    let rooms = Rooms::new();
    rooms.store.set_encrypted(PARENT, false);
    rooms.store.set_listed(PARENT, true);

    rooms.create(MEMBER, 20, "release").await.expect("没有门的");
    let link = rooms.store.link(20).link;
    assert!(!link.end_to_end_encrypted, "没有门的跟父房间，不加密");
    assert_eq!(link.door_opened_at, Some(UtcMillis::new(1_000).unwrap()));
    assert_eq!(listed_lines(&rooms), ["listed 1000"]);

    rooms.matrix.clear_changes();
    rooms
        .service
        .create_sub_room(CreateSubRoom {
            access: RoomJoinMode::Request,
            ..Rooms::request(MEMBER, 21, "security")
        })
        .await
        .expect("有门的");
    assert_eq!(
        rooms.store.link(21).link.door_opened_at,
        None,
        "有门的要批准才进得来，拆门时再写"
    );
    assert!(listed_lines(&rooms).is_empty());

    // 父房间私密着建的没有门的不写：私密房间里后进来的人照旧拿得到以前的钥匙。
    rooms.store.set_listed(PARENT, false);
    rooms.store.set_encrypted(PARENT, true);
    rooms.matrix.clear_changes();
    rooms.create(MEMBER, 22, "design").await.expect("没有门的");
    assert_eq!(rooms.store.link(22).link.door_opened_at, None);
    assert!(listed_lines(&rooms).is_empty());
}

#[tokio::test]
async fn 建的当口父房间改回私密了_存好以后补开加密() {
    let rooms = Rooms::new();
    rooms.store.set_encrypted(PARENT, false);
    rooms.store.set_listed(PARENT, true);
    // 读完父房间加不加密，房主就把它改回了私密：父房间加了密，公开设置也关了。
    rooms.store.change_after_parent_facts(|store| {
        store.set_encrypted(PARENT, true);
        store.set_listed(PARENT, false);
    });

    rooms.create(MEMBER, 20, "release").await.expect("建得出");
    assert_eq!(
        rooms
            .matrix
            .creation()
            .expect("建了")
            .request()
            .encryption(),
        MatrixRoomEncryption::Unencrypted,
        "建的时候父房间还不加密"
    );
    let link = rooms.store.link(20).link;
    assert!(link.end_to_end_encrypted, "存好以后跟上了");
    assert_eq!(link.door_opened_at, None, "父房间已经不公开了");
    assert!(
        rooms
            .matrix
            .changes_in(CREATED)
            .contains(&"encrypt".to_owned())
    );
    let events = rooms.events();
    let created = events.iter().position(|event| *event == "sub_rooms.create");
    let encrypted = events.iter().position(|event| *event == "matrix.encrypt");
    assert!(created < encrypted, "先存好再补：{events:?}");
}

#[tokio::test]
async fn 建的当口父房间公开了_这一刻没写进_重发同一个请求补上() {
    let rooms = Rooms::new();
    rooms.store.set_encrypted(PARENT, false);
    let matrix = rooms.matrix.clone();
    // 第一次读父房间（建之前）时还私密着；第二次（存好以后）它已经公开了，可 Matrix 正写不进。
    rooms.store.change_after_parent_facts(move |store| {
        store.change_after_parent_facts(move |store| {
            store.set_listed(PARENT, true);
            matrix.fail_in(CREATED, MatrixFailureKind::DependencyUnavailable);
        });
    });

    assert_eq!(
        refusal(rooms.create(MEMBER, 20, "release").await),
        PrivateRoomFailureKind::DependencyUnavailable
    );
    assert_eq!(
        rooms.store.link(20).link.door_opened_at,
        None,
        "存好了，还没写"
    );

    rooms.matrix.recover(CREATED);
    rooms
        .create(MEMBER, 20, "release")
        .await
        .expect("重发回同一间，顺带补上");
    assert_eq!(
        rooms.store.link(20).link.door_opened_at,
        Some(UtcMillis::new(1_000).unwrap())
    );
    assert_eq!(listed_lines(&rooms), ["listed 1000"]);
}
