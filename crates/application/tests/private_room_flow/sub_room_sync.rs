//! 子房间第 2c-2 步（`specs/redesign/sub-rooms.md` 的“对账”）：父房间变了以后，聊天服务器上跟着做。
//! 夹具是 `sub_rooms.rs` 里的那一份：存父房间时照真实存储的规矩重算子房间、把要做的事排上；Matrix 的
//! 替身记下每一次改动落在哪间房。时钟不走，“过了一阵”由 `let_sync_come_due` 摆出来。

use agent_room_application::private_rooms::SubRoomSyncRetry;

use super::*;

const SPEAKER: [PrivateRoomCapability; 3] = [
    PrivateRoomCapability::View,
    PrivateRoomCapability::Speak,
    PrivateRoomCapability::Automate,
];
const REMOVED: PrivateRoomMembershipStatus = PrivateRoomMembershipStatus::Removed;

/// 夹具里直接放进存储的房间在聊天服务器上的房间号。
pub(super) fn room(catalog: u128) -> String {
    format!("!room-{catalog}:matrix.test")
}

/// 两间子房间，成员进了第一间；另一间里只有房主。
pub(super) async fn two_sub_rooms() -> Rooms {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.store.seed(&Seed::open(21, "design"));
    rooms.join(MEMBER, 20).await.expect("成员进了一间");
    rooms.matrix.clear_changes();
    rooms
}

/// 给这个人登记一个 Agent，它在聊天服务器上已经在这几间房里。
pub(super) fn agent_in(rooms: &Rooms, owner: u128, name: &str, catalogs: &[u128]) {
    let agent = MatrixUserId::new(format!("@{name}:matrix.test")).expect("用户有效");
    rooms.agents.register(principal_id(owner), &agent);
    for catalog in catalogs {
        rooms
            .matrix
            .set_membership_in(&room(*catalog), &agent, PrivateMatrixMembership::Joined);
    }
}

fn governing(target: u128) -> GovernPrivateRoomMember {
    GovernPrivateRoomMember {
        actor: person(OWNER),
        catalog_id: catalog_id(PARENT),
        target_principal_id: principal_id(target),
    }
}

async fn remove_from_parent(rooms: &Rooms, target: u128) {
    rooms
        .service
        .remove(governing(target))
        .await
        .expect("房主能移出");
}

pub(super) async fn set_parent_permissions(
    rooms: &Rooms,
    target: u128,
    permissions: PrivateRoomPermissions,
) {
    rooms
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: person(OWNER),
            catalog_id: catalog_id(PARENT),
            target_principal_id: principal_id(target),
            permissions,
        })
        .await
        .expect("房主能改权限");
}

fn standing(rooms: &Rooms, catalog: u128, person: u128) -> Option<PrivateRoomMembershipStatus> {
    rooms
        .store
        .room(catalog)
        .member(principal_id(person))
        .map(PrivateRoomMember::status)
}

fn at(millis: i64) -> UtcMillis {
    UtcMillis::new(millis).expect("时间有效")
}

#[tokio::test]
async fn 被移出房间的人当场从他在的子房间里出去_他的_agent_一起_没进的那一间不动() {
    let rooms = two_sub_rooms().await;
    agent_in(&rooms, MEMBER, "member-agent", &[PARENT, 20]);

    remove_from_parent(&rooms, MEMBER).await;

    let out = ["kick @member:matrix.test", "kick @member-agent:matrix.test"];
    assert_eq!(rooms.matrix.changes_in(&room(PARENT)), out);
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        out,
        "这个请求结束前，子房间里也做完了"
    );
    assert!(rooms.matrix.changes_in(&room(21)).is_empty());
    assert_eq!(standing(&rooms, 20, MEMBER), Some(REMOVED));
    assert!(rooms.store.queued_sync().is_empty(), "不用等后台");
}

#[tokio::test]
async fn 被封禁的人在子房间里只是出去_自己退出房间的人也跟着出去() {
    let rooms = two_sub_rooms().await;
    rooms.join(VIEWER, 20).await.expect("只能看的也进了");
    rooms.matrix.clear_changes();

    rooms
        .service
        .ban(governing(MEMBER))
        .await
        .expect("房主能封禁");
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["kick @member:matrix.test"],
        "封禁落在父房间上：子房间里踢出去就够了，进子房间要先在父房间里"
    );
    assert_eq!(standing(&rooms, 20, MEMBER), Some(REMOVED));

    // 自己退出：网页先替他在聊天服务器上离开父房间，再告诉控制面。
    rooms
        .matrix
        .set_membership(&matrix_user(VIEWER), PrivateMatrixMembership::Left);
    rooms.matrix.clear_changes();
    rooms
        .service
        .leave(PrivateRoomMembershipAction {
            actor: person(VIEWER),
            catalog_id: catalog_id(PARENT),
        })
        .await
        .expect("能退出");
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["kick @viewer:matrix.test"]
    );
    assert_eq!(standing(&rooms, 20, VIEWER), Some(REMOVED));
    assert!(rooms.store.queued_sync().is_empty());
}

#[tokio::test]
async fn 父房间里收回发言_子房间里他和他的_agent_跟着不能说_给回来跟着能说() {
    let rooms = two_sub_rooms().await;
    agent_in(&rooms, MEMBER, "member-agent", &[PARENT, 20]);

    set_parent_permissions(&rooms, MEMBER, viewer_permissions()).await;
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.off @member:matrix.test",
            "speak.off @member-agent:matrix.test"
        ],
        "他已经在这一间里，不再邀请"
    );
    assert_eq!(
        rooms
            .store
            .room(20)
            .member(principal_id(MEMBER))
            .expect("还在里面")
            .permissions(),
        viewer_permissions()
    );

    rooms.matrix.clear_changes();
    set_parent_permissions(&rooms, MEMBER, speaker_permissions()).await;
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.on @member:matrix.test",
            "speak.on @member-agent:matrix.test"
        ]
    );
    assert!(rooms.matrix.changes_in(&room(21)).is_empty());
    assert!(rooms.store.queued_sync().is_empty());
}

#[tokio::test]
async fn 父房间归档_没归档的子房间在聊天服务器上跟着锁住_之后里面的人不再设发言级别() {
    let rooms = two_sub_rooms().await;
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "old")
    });

    rooms
        .service
        .archive(ArchivePrivateRoom {
            actor: person(OWNER).into(),
            catalog_id: catalog_id(PARENT),
        })
        .await
        .expect("房主能归档");

    for catalog in [PARENT, 20, 21] {
        assert_eq!(rooms.matrix.changes_in(&room(catalog)), ["archive"]);
    }
    assert!(
        rooms.matrix.changes_in(&room(22)).is_empty(),
        "自己归档过的不动"
    );
    assert_eq!(
        rooms.store.room(20).status(),
        PrivateRoomLifecycleStatus::Archived
    );
    assert!(rooms.store.link(20).archived);
    assert!(rooms.store.queued_sync().is_empty());

    // 归档了的子房间里还排着某个人的事：只补邀请。设发言级别会把整间房的门槛写回“在用”的那一套。
    rooms
        .store
        .queue_sync(catalog_id(20), Some(principal_id(MEMBER)));
    rooms.matrix.clear_changes();
    assert_eq!(rooms.sync().await.done, 1);
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
}

#[tokio::test]
async fn 聊天服务器不通_请求照样成功_事留着记下原因_到点再试_通了以后做完() {
    let rooms = two_sub_rooms().await;
    rooms.join(MEMBER, 21).await.expect("另一间也进了");
    rooms.matrix.clear_changes();
    rooms
        .matrix
        .fail_in(&room(20), MatrixFailureKind::DependencyUnavailable);

    remove_from_parent(&rooms, MEMBER).await;

    assert!(
        rooms.matrix.changes_in(&room(21)).is_empty(),
        "整个聊天服务器用不了：这一批剩下的不再试"
    );
    let waiting = rooms.store.queued_sync();
    assert_eq!(waiting.len(), 2);
    for queued in &waiting {
        assert_eq!(queued.attempts, 1);
        assert_eq!(queued.failure_code, Some("matrix.unavailable"));
        assert_eq!(queued.next_attempt_at, Some(at(31_000)), "三十秒以后再试");
    }

    // 还没到点：后台这一轮领不到。
    assert_eq!(rooms.sync().await, SubRoomSyncOutcome::default());

    // 到点了还是不通：再往后排，隔得更久。
    rooms.store.let_sync_come_due();
    let again = rooms.sync().await;
    assert_eq!(again.done, 0);
    let retry = |task_id| SubRoomSyncRetry {
        task_id,
        failure_code: "matrix.unavailable",
        attempt: 2,
    };
    assert_eq!(again.retrying, [retry(1), retry(2)]);
    assert_eq!(
        rooms.store.queued_sync()[0].next_attempt_at,
        Some(at(61_000))
    );

    rooms.matrix.recover(&room(20));
    rooms.store.let_sync_come_due();
    let recovered = rooms.sync().await;
    assert_eq!((recovered.done, recovered.retrying.len()), (2, 0));
    for catalog in [20, 21] {
        assert_eq!(
            rooms.matrix.changes_in(&room(catalog)),
            ["kick @member:matrix.test"]
        );
    }
    assert!(rooms.store.queued_sync().is_empty());
}

#[tokio::test]
async fn 只是一间房里动不了_别的照做_那一件留着() {
    let rooms = two_sub_rooms().await;
    rooms.join(MEMBER, 21).await.expect("另一间也进了");
    rooms.matrix.clear_changes();
    rooms
        .matrix
        .fail_in(&room(20), MatrixFailureKind::Forbidden);

    remove_from_parent(&rooms, MEMBER).await;

    assert_eq!(
        rooms.matrix.changes_in(&room(21)),
        ["kick @member:matrix.test"]
    );
    let waiting = rooms.store.queued_sync();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].catalog_id, catalog_id(20));
    assert_eq!(waiting[0].failure_code, Some("matrix.forbidden"));
}

#[tokio::test]
async fn 做的当口他自己退出了这一间_或者这一间归档了_动完再看一眼照新的摆() {
    let rooms = two_sub_rooms().await;
    let member = Some(principal_id(MEMBER));
    rooms.store.queue_sync(catalog_id(20), member);
    rooms.store.interrupt_read(20, 1, |store| {
        store.change(20, |room| {
            room.leave(principal_id(MEMBER)).expect("里面的人能退出");
        });
    });
    assert_eq!(rooms.sync().await.done, 1);
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["speak.on @member:matrix.test", "kick @member:matrix.test"],
        "照旧样子设了发言级别，再看时他已经不在里面：踢出去"
    );

    let rooms = two_sub_rooms().await;
    rooms.store.queue_sync(catalog_id(20), member);
    rooms.store.interrupt_read(20, 1, |store| store.archive(20));
    assert_eq!(rooms.sync().await.done, 1);
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        ["speak.on @member:matrix.test", "archive"],
        "设发言级别把归档的房间解开了：再锁一次"
    );
}

#[tokio::test]
async fn 没进过的新房主收到邀请_在这一间封着的不邀请_没有聊天账户的只动它的_agent() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    // 库里已经照父房间改好：换了房主以后他在每一间子房间里；另两个是摆出来的少见情形。
    rooms.store.change(20, |room| {
        room.admit(principal_id(MEMBER), permissions(&SPEAKER))
            .expect("能记进来");
        room.admit(principal_id(VIEWER), viewer_permissions())
            .expect("能记进来");
        room.admit(principal_id(EARLIER), permissions(&SPEAKER))
            .expect("能记进来");
    });
    rooms.matrix.set_membership_in(
        &room(20),
        &matrix_user(VIEWER),
        PrivateMatrixMembership::Banned,
    );
    agent_in(&rooms, EARLIER, "network-agent", &[20]);
    for person in [MEMBER, VIEWER, EARLIER] {
        rooms
            .store
            .queue_sync(catalog_id(20), Some(principal_id(person)));
    }

    assert_eq!(rooms.sync().await.done, 3);

    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [
            "speak.on @member:matrix.test",
            "invite @member:matrix.test",
            "speak.off @viewer:matrix.test",
            "speak.on @network-agent:matrix.test",
        ],
        "网页接着照“服务器记着已加入、聊天服务器上是受邀”替新房主加入"
    );
}

#[tokio::test]
async fn 做的时候同一件事又排进来过_做完不删它_再排一次_下一轮照新的再做一遍() {
    let rooms = two_sub_rooms().await;
    let member = Some(principal_id(MEMBER));
    let requeue = move |store: &MemoryRooms| store.queue_sync(catalog_id(20), member);
    rooms.store.queue_sync(catalog_id(20), member);
    rooms.store.interrupt_read(20, 1, requeue);

    rooms.sync().await;
    let waiting = rooms.store.queued_sync();
    assert_eq!(waiting.len(), 1, "这一遍照的可能是旧样子");
    assert_eq!(
        (waiting[0].attempts, waiting[0].next_attempt_at),
        (0, None),
        "马上能领"
    );
    assert!(waiting[0].generation > 1);

    assert_eq!(rooms.sync().await.done, 1);
    assert!(rooms.store.queued_sync().is_empty());

    // 没做成的时候也一样：不记到旧的那一版上，再排一次。
    rooms.store.queue_sync(catalog_id(20), member);
    rooms.store.interrupt_read(20, 0, requeue);
    rooms
        .matrix
        .fail_in(&room(20), MatrixFailureKind::Forbidden);
    assert_eq!(rooms.sync().await.retrying.len(), 1);
    let waiting = rooms.store.queued_sync();
    assert_eq!(
        (waiting[0].failure_code, waiting[0].next_attempt_at),
        (None, None)
    );
}

#[tokio::test]
async fn 后台每一轮先补对账再领事_子房间不在了的事直接算做完() {
    let rooms = two_sub_rooms().await;
    rooms.clear_events();
    assert_eq!(rooms.sync().await, SubRoomSyncOutcome::default());
    assert_eq!(rooms.events(), ["sub_rooms.follow_stale"]);

    rooms
        .store
        .queue_sync(catalog_id(99), Some(principal_id(MEMBER)));
    rooms.store.queue_sync(catalog_id(98), None);
    assert_eq!(rooms.sync().await.done, 2);
    assert!(rooms.store.queued_sync().is_empty());
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
}
