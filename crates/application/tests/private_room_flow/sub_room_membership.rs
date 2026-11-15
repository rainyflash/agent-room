//! 子房间第 2b 步（`specs/redesign/sub-rooms.md`）：人自己进一间子房间、从里面退出，和改它的名字、
//! 用途。夹具是 `sub_rooms.rs` 里的那一份；Matrix 的替身不分房间，只记每个人最后的成员状态。

use super::*;

const SPEAKER: [PrivateRoomCapability; 3] = [
    PrivateRoomCapability::View,
    PrivateRoomCapability::Speak,
    PrivateRoomCapability::Automate,
];

pub(super) fn refusal(result: Result<impl Sized, PrivateRoomFailure>) -> PrivateRoomFailureKind {
    result.err().expect("应该被拒").kind()
}

pub(super) const fn sub_room(refusal: SubRoomRefusal) -> PrivateRoomFailureKind {
    PrivateRoomFailureKind::SubRoom(refusal)
}

fn membership_in_matrix(rooms: &Rooms, person: u128) -> Option<PrivateMatrixMembership> {
    rooms.matrix.membership_of(&matrix_user(person))
}

#[tokio::test]
async fn 没有门的父房间里的人点一下就进_直接是已加入_权限照父房间里的() {
    let rooms = Rooms::new();
    rooms.create(OWNER, 20, "release").await.expect("房主建的");
    rooms.clear_events();

    let joined = rooms.join(MEMBER, 20).await.expect("成员能进");
    let member = joined.room().member(principal_id(MEMBER)).expect("在里面");
    assert!(member.has_joined(), "不用再点接受");
    assert_eq!(member.permissions(), permissions(&SPEAKER));
    assert_eq!(joined.parent_catalog_id(), Some(catalog_id(PARENT)));
    assert_eq!(
        rooms.events(),
        ["matrix.speak.on", "sub_rooms.admit", "matrix.invite"],
        "发言级别先设好，记下成员，最后才邀请"
    );
    assert_eq!(
        membership_in_matrix(&rooms, MEMBER),
        Some(PrivateMatrixMembership::Invited),
        "网页接着替他加入"
    );
    assert_eq!(
        rooms.matrix.speaking_allowed(matrix_user(MEMBER).as_str()),
        Some(true)
    );
    assert_eq!(
        rooms.store.room(20),
        *joined.room(),
        "存下来的就是回的这一间"
    );

    // 只能看的进来也只能看。
    rooms.clear_events();
    let viewing = rooms.join(VIEWER, 20).await.expect("只能看的也能进");
    assert_eq!(
        viewing
            .room()
            .member(principal_id(VIEWER))
            .expect("在里面")
            .permissions(),
        permissions(&[PrivateRoomCapability::View])
    );
    assert_eq!(
        rooms.events(),
        ["matrix.speak.off", "sub_rooms.admit", "matrix.invite"]
    );

    // 父房间的管理员进来就管这一间，但不记成这一间的子房间管理员：他管的是整间房。
    let managing = rooms.join(ADMIN, 20).await.expect("管理员能进");
    assert_eq!(
        managing
            .room()
            .member(principal_id(ADMIN))
            .expect("在里面")
            .permissions(),
        permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Manage,
        ])
    );
    assert!(rooms.store.link(20).managers.is_empty());

    let list = rooms.list(MEMBER, PARENT).await.expect("成员能读名单");
    assert_eq!(list.sub_rooms[0].entry.member_count, 4);
}

#[tokio::test]
async fn 已经在里面的再点一次只补邀请_不再存_也不重设发言() {
    let rooms = Rooms::new();
    rooms.create(OWNER, 20, "release").await.expect("房主建的");
    let first = rooms.join(MEMBER, 20).await.expect("成员能进");
    // 他在 Matrix 上拒了邀请，或者上一次邀请没发成。
    rooms
        .matrix
        .set_membership(&matrix_user(MEMBER), PrivateMatrixMembership::Left);
    rooms.clear_events();

    let again = rooms.join(MEMBER, 20).await.expect("再点一次不算错");
    assert_eq!(again, first, "还是那一间，版本没动");
    assert_eq!(
        rooms.events(),
        ["matrix.invite"],
        "被禁言的人再点一次加入，不该把发言级别冲回去"
    );
    assert_eq!(
        membership_in_matrix(&rooms, MEMBER),
        Some(PrivateMatrixMembership::Invited)
    );
}

#[tokio::test]
async fn 不在父房间里的人_对不上的父房间_没有的房间号_都当找不到() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.store.seed(&Seed::open(21, "website"));
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.join(OUTSIDER, 20).await),
        PrivateRoomFailureKind::NotFound
    );
    assert_eq!(
        refusal(rooms.join(MEMBER, 999).await),
        PrivateRoomFailureKind::NotFound
    );
    // 拿父房间自己的房间号当子房间。
    assert_eq!(
        refusal(rooms.join(MEMBER, PARENT).await),
        PrivateRoomFailureKind::NotFound
    );
    // 地址里的“父房间”其实是他在的另一间子房间。
    let under_wrong_parent = rooms
        .service
        .join_sub_room(SubRoomMembershipAction {
            actor: person(MEMBER),
            parent_catalog_id: catalog_id(20),
            catalog_id: catalog_id(21),
        })
        .await;
    assert_eq!(
        refusal(under_wrong_parent),
        PrivateRoomFailureKind::NotFound
    );
    assert_eq!(
        refusal(rooms.leave(OUTSIDER, 20).await),
        PrivateRoomFailureKind::NotFound
    );
    assert_eq!(
        refusal(rooms.revise(OUTSIDER, 20, Some("mine"), None).await),
        PrivateRoomFailureKind::NotFound
    );
    assert!(rooms.events().is_empty(), "什么都没动");
}

#[tokio::test]
async fn 有门的要批准或者只能请进来_房主和管理员不看门_隐藏的别人找不到() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(21, "hiring")
    });
    rooms.store.seed(&Seed {
        access: RoomJoinMode::InviteOnly,
        ..Seed::open(22, "board")
    });
    rooms.store.seed(&Seed {
        access: RoomJoinMode::InviteOnly,
        hidden: true,
        ..Seed::open(23, "security")
    });
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.join(MEMBER, 21).await),
        sub_room(SubRoomRefusal::ApprovalRequired)
    );
    assert_eq!(
        refusal(rooms.join(MEMBER, 22).await),
        sub_room(SubRoomRefusal::InviteOnly)
    );
    assert_eq!(
        refusal(rooms.join(MEMBER, 23).await),
        PrivateRoomFailureKind::NotFound,
        "不在名单里显示的，没进的人拿房间号也问不出来"
    );
    assert!(rooms.events().is_empty(), "被门挡住的什么都没动");

    for catalog in [21, 22, 23] {
        let joined = rooms.join(ADMIN, catalog).await.expect("管理员不看门");
        assert!(
            joined
                .room()
                .member(principal_id(ADMIN))
                .expect("在里面")
                .permissions()
                .manage()
        );
    }
}

#[tokio::test]
async fn 封禁着的进不来_归档了的谁都进不来() {
    let rooms = Rooms::new();
    rooms.create(OWNER, 20, "release").await.expect("房主建的");
    rooms.join(MEMBER, 20).await.expect("成员能进");
    rooms.join(ADMIN, 20).await.expect("管理员能进");
    rooms.store.change(20, |room| {
        for banned in [MEMBER, ADMIN] {
            room.ban_member(principal_id(OWNER), principal_id(banned))
                .expect("房主能封禁");
        }
    });
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "done")
    });
    rooms.clear_events();

    for banned in [MEMBER, ADMIN] {
        assert_eq!(
            refusal(rooms.join(banned, 20).await),
            sub_room(SubRoomRefusal::Banned),
            "父房间的管理员在这一间里被封禁了也进不来"
        );
    }
    for anyone in [VIEWER, ADMIN, OWNER] {
        assert_eq!(
            refusal(rooms.join(anyone, 22).await),
            sub_room(SubRoomRefusal::Archived)
        );
    }
    assert!(rooms.events().is_empty(), "什么都没动");
}

#[tokio::test]
async fn 存之前他被移出了父房间_子房间里不留他_也不邀请() {
    let rooms = Rooms::new();
    rooms.create(OWNER, 20, "release").await.expect("房主建的");
    rooms.store.leave_parent_before_admission(MEMBER);
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.join(MEMBER, 20).await),
        PrivateRoomFailureKind::NotFound,
        "和不在父房间里的人一样回"
    );
    assert!(rooms.store.room(20).member(principal_id(MEMBER)).is_none());
    assert_eq!(rooms.events(), ["matrix.speak.on", "sub_rooms.admit"]);
    assert_eq!(membership_in_matrix(&rooms, MEMBER), None, "没有邀请");
}

#[tokio::test]
async fn 退出只要一个请求_他的_agent_跟着出去_这一间的管理员记录一起没了() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    assert_eq!(rooms.store.link(20).managers, [principal_id(MEMBER)]);
    let agent = MatrixUserId::new("@agent-of-member:matrix.test").expect("用户有效");
    rooms.agents.register(principal_id(MEMBER), &agent);
    rooms
        .matrix
        .set_membership(&agent, PrivateMatrixMembership::Joined);
    rooms.clear_events();

    rooms.leave(MEMBER, 20).await.expect("能退出");
    assert_eq!(
        rooms.events(),
        [
            "matrix.kick",
            "matrix.membership",
            "matrix.kick",
            "store.save"
        ],
        "控制面替他在 Matrix 上离开，再把他的 Agent 移出去"
    );
    assert_eq!(
        membership_in_matrix(&rooms, MEMBER),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        rooms.matrix.membership_of(&agent),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        rooms
            .store
            .room(20)
            .member(principal_id(MEMBER))
            .expect("留着记录")
            .status(),
        PrivateRoomMembershipStatus::Removed
    );
    assert!(
        rooms.store.link(20).managers.is_empty(),
        "退出了就不再管这一间"
    );

    // 同一个请求重发：没什么可退的。
    rooms.clear_events();
    rooms.leave(MEMBER, 20).await.expect("再退一次不算错");
    assert!(rooms.events().is_empty());

    // 没有门的还能再进，回来是普通成员。
    let back = rooms.join(MEMBER, 20).await.expect("退出过的能再进");
    assert_eq!(
        back.room()
            .member(principal_id(MEMBER))
            .expect("在里面")
            .permissions(),
        permissions(&SPEAKER),
        "不带着以前的管理"
    );
    assert_eq!(
        refusal(rooms.revise(MEMBER, 20, Some("mine"), None).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
}

#[tokio::test]
async fn 房主退不出去_从没进来的不算错_归档了的退不了() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.leave(OWNER, 20).await),
        sub_room(SubRoomRefusal::OwnerStays),
        "房主在每一间子房间里"
    );
    rooms.leave(VIEWER, 20).await.expect("没进来过的不算错");
    assert!(rooms.events().is_empty(), "什么都没动");

    rooms.store.archive(20);
    assert_eq!(
        refusal(rooms.leave(MEMBER, 20).await),
        sub_room(SubRoomRefusal::Archived)
    );
    assert!(rooms.events().is_empty(), "什么都没动");
}

#[tokio::test]
async fn 改名字和用途_目录先改_matrix_跟着写_没带的那一项不动() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.clear_events();

    let revised = rooms
        .revise(MEMBER, 20, Some("Launch"), Some("  Ship Alpha 95  "))
        .await
        .expect("建的人管这一间");
    assert_eq!(revised.catalog().name(), "Launch");
    assert_eq!(revised.catalog().description(), "Ship Alpha 95");
    assert_eq!(revised.parent_catalog_id(), Some(catalog_id(PARENT)));
    assert_eq!(
        rooms.events(),
        ["sub_rooms.revise", "matrix.set_name", "matrix.set_topic"]
    );
    assert_eq!(rooms.store.link(20).link.name_key, "launch");
    assert_eq!(
        rooms
            .matrix
            .room_name
            .lock()
            .expect("房间名锁正常")
            .as_deref(),
        Some("Launch")
    );
    assert_eq!(
        rooms.matrix.topic.lock().expect("话题锁正常").as_deref(),
        Some("Ship Alpha 95")
    );

    // 只改用途：名字不动。只改名字：用途不动。
    let described = rooms
        .revise(MEMBER, 20, None, Some(""))
        .await
        .expect("能清掉用途");
    assert_eq!(described.catalog().name(), "Launch");
    assert_eq!(described.catalog().description(), "");
    rooms
        .revise(MEMBER, 20, None, Some("Ship it"))
        .await
        .expect("能改用途");
    let renamed = rooms
        .revise(MEMBER, 20, Some("LAUNCH"), None)
        .await
        .expect("只换大小写不算和自己重名");
    assert_eq!(renamed.catalog().name(), "LAUNCH");
    assert_eq!(renamed.catalog().description(), "Ship it");

    // 和现在的一样也照写一遍：上一次目录改好了、Matrix 没写成的，再点一次保存就补上。
    rooms.clear_events();
    rooms
        .revise(MEMBER, 20, Some("LAUNCH"), Some("Ship it"))
        .await
        .expect("没变也不算错");
    assert_eq!(
        rooms.events(),
        ["sub_rooms.revise", "matrix.set_name", "matrix.set_topic"]
    );

    let listed = rooms.list(VIEWER, PARENT).await.expect("能读名单");
    assert_eq!(listed.sub_rooms[0].entry.name, "LAUNCH");
    assert_eq!(listed.sub_rooms[0].entry.description, "Ship it");
}

#[tokio::test]
async fn 管得了这一间的人才能改_父房间的房主和管理员没进来也能改() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.join(VIEWER, 20).await.expect("只能看的也能进");
    rooms.clear_events();

    // 在里面但不管这一间的，和根本没进来的普通成员，都改不了。
    assert_eq!(
        refusal(rooms.revise(VIEWER, 20, Some("mine"), None).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    rooms.store.seed(&Seed::open(21, "website"));
    assert_eq!(
        refusal(rooms.revise(MEMBER, 21, Some("mine"), None).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    for (who, name) in [(ADMIN, "by-admin"), (OWNER, "by-owner")] {
        let revised = rooms
            .revise(who, 20, Some(name), None)
            .await
            .expect("房主和管理员管每一间");
        assert_eq!(revised.catalog().name(), name);
    }
}

#[tokio::test]
async fn 改名撞了别的子房间_名字不像样_什么都没带_归档了的_都改不了() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.store.seed(&Seed::open(21, "website"));
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "done")
    });
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.revise(MEMBER, 20, Some("WebSite"), None).await),
        sub_room(SubRoomRefusal::NameTaken),
        "不分大小写"
    );
    assert_eq!(
        rooms.events(),
        ["sub_rooms.revise"],
        "目录说重名，Matrix 上的房间名就不动"
    );
    assert_eq!(rooms.store.link(20).link.name_key, "release");
    rooms
        .revise(MEMBER, 20, Some("done"), None)
        .await
        .expect("归档了的把名字让出来了");

    rooms.clear_events();
    for bad in ["a/b", " lead", ""] {
        assert_eq!(
            refusal(rooms.revise(MEMBER, 20, Some(bad), None).await),
            PrivateRoomFailureKind::InvalidRequest,
            "{bad:?}"
        );
    }
    assert_eq!(
        refusal(rooms.revise(MEMBER, 20, None, None).await),
        PrivateRoomFailureKind::InvalidRequest
    );
    assert_eq!(
        refusal(rooms.revise(OWNER, 22, Some("again"), None).await),
        sub_room(SubRoomRefusal::Archived),
        "归档了的要先恢复"
    );
    assert!(rooms.events().is_empty(), "什么都没动");
}
