//! 子房间第 2c-3 步（`specs/redesign/sub-rooms.md` 的“禁言”）：父房间的禁言在子房间里一起算。这里
//! 是私人房间服务的这一半：在子房间里给发言级别之前，先问他是不是被禁言着，免得进来、新建、改权限
//! 把禁言冲掉。治理台账的那一半（落、撤回、到期）在 `moderation_flow/sub_rooms.rs`。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份：谁在哪间房里被禁言着由存储的替身记，父房间里的在它的子房间里
//! 一起算。

use super::{
    sync::{agent_in, room, set_parent_permissions, two_sub_rooms},
    *,
};

const MEMBER_USER: &str = "@member:matrix.test";

/// 一间没有门的子房间，里面只有房主。
fn one_sub_room() -> Rooms {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms
}

#[tokio::test]
async fn 在父房间里被禁言着的人进子房间_不给发言级别_退出再进来也一样() {
    let rooms = one_sub_room();
    rooms.store.mute(PARENT, MEMBER);

    rooms.join(MEMBER, 20).await.expect("被禁言的人照样能进");
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.off @member:matrix.test",
            "invite @member:matrix.test"
        ]
    );

    rooms.leave(MEMBER, 20).await.expect("能退出");
    rooms.matrix.clear_changes();
    rooms.join(MEMBER, 20).await.expect("退出过的能再进");
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.off @member:matrix.test",
            "invite @member:matrix.test"
        ],
        "退出再进来不该把禁言绕过去"
    );

    // 禁言撤了以后再进来的照常给。
    rooms.store.unmute(PARENT, MEMBER);
    rooms.leave(MEMBER, 20).await.expect("能退出");
    rooms.matrix.clear_changes();
    rooms.join(MEMBER, 20).await.expect("能再进");
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["speak.on @member:matrix.test", "invite @member:matrix.test"]
    );
}

#[tokio::test]
async fn 只在一间子房间里被禁言的_那一间不给_别的子房间照给() {
    let rooms = one_sub_room();
    rooms.store.seed(&Seed::open(21, "design"));
    rooms.store.mute(20, MEMBER);

    rooms.join(MEMBER, 20).await.expect("能进");
    rooms.join(MEMBER, 21).await.expect("能进");

    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.off @member:matrix.test",
            "invite @member:matrix.test"
        ]
    );
    assert_eq!(
        rooms.matrix.changes_in(&room(21)),
        ["speak.on @member:matrix.test", "invite @member:matrix.test"]
    );
}

#[tokio::test]
async fn 进来的当口禁言落下来_记成成员以后再看一眼_把发言级别收回() {
    let rooms = one_sub_room();
    // 给发言级别之前问的那一次还没有，记成成员以后再问就有了。
    rooms.store.mute_after(1, PARENT, MEMBER);

    rooms.join(MEMBER, 20).await.expect("能进");

    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.on @member:matrix.test",
            "speak.off @member:matrix.test",
            "invite @member:matrix.test"
        ],
        "那条禁言落的时候还看不到他在这一间里，得由进门的这一边收回"
    );
}

#[tokio::test]
async fn 被禁言着的人新建的子房间里也不给发言级别() {
    let rooms = Rooms::new();
    rooms.store.mute(PARENT, MEMBER);

    rooms
        .create(MEMBER, 20, "release")
        .await
        .expect("被禁言的人照样能建");

    assert_eq!(rooms.matrix.speaking_allowed(MEMBER_USER), Some(false));
    assert_eq!(
        rooms.matrix.speaking_allowed("@owner:matrix.test"),
        Some(true),
        "房主没被禁言，照常给"
    );
}

#[tokio::test]
async fn 建的当口禁言落下来_存好以后再看一眼_把发言级别收回() {
    let rooms = Rooms::new();
    // 建之前问房主和他各一次都还没有，存好以后再问就有了。
    rooms.store.mute_after(2, PARENT, MEMBER);

    rooms.create(MEMBER, 20, "release").await.expect("能建");

    assert_eq!(rooms.matrix.speaking_allowed(MEMBER_USER), Some(false));
    assert_eq!(
        rooms.matrix.speaking_allowed("@owner:matrix.test"),
        Some(true)
    );
}

#[tokio::test]
async fn 父房间里改被禁言的人的权限_子房间里不把禁言冲掉_他的_agent_也不给() {
    let rooms = two_sub_rooms().await;
    agent_in(&rooms, MEMBER, "member-agent", &[PARENT, 20]);
    rooms.store.mute(PARENT, MEMBER);

    set_parent_permissions(&rooms, MEMBER, viewer_permissions()).await;
    rooms.matrix.clear_changes();
    set_parent_permissions(&rooms, MEMBER, speaker_permissions()).await;
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.off @member:matrix.test",
            "speak.off @member-agent:matrix.test"
        ],
        "权限给回来了，禁言还在：不能说"
    );

    // 禁言撤了以后再照着摆一遍的，才给。
    rooms.store.unmute(PARENT, MEMBER);
    set_parent_permissions(&rooms, MEMBER, viewer_permissions()).await;
    rooms.matrix.clear_changes();
    set_parent_permissions(&rooms, MEMBER, speaker_permissions()).await;
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.on @member:matrix.test",
            "speak.on @member-agent:matrix.test"
        ]
    );
}

#[tokio::test]
async fn 照着摆的当口禁言落下来_动完再看一眼_照新的再摆一遍() {
    let rooms = two_sub_rooms().await;
    agent_in(&rooms, MEMBER, "member-agent", &[PARENT, 20]);
    set_parent_permissions(&rooms, MEMBER, viewer_permissions()).await;
    rooms.matrix.clear_changes();
    rooms.store.mute_after(1, PARENT, MEMBER);

    set_parent_permissions(&rooms, MEMBER, speaker_permissions()).await;

    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.on @member:matrix.test",
            "speak.on @member-agent:matrix.test",
            "speak.off @member:matrix.test",
            "speak.off @member-agent:matrix.test"
        ]
    );
    assert!(rooms.store.queued_sync().is_empty(), "这件事做完了");
}
