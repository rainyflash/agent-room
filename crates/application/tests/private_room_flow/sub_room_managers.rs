//! 子房间第 2d-4 步（`specs/redesign/room-roles.md` 的 R5）：设、撤子房间管理员。管得了这一间的人都
//! 能设，他得已经在这一间里；撤别人要父房间的房主或管理员，子房间管理员只能撤自己。身上的“管理”和
//! 管理员名单一起改，父房间里一变、子房间照着重算时还管这一间。夹具是 `sub_rooms.rs` 里的那一份。

use agent_room_application::private_rooms::ChangeSubRoomManager;

use super::{
    membership::{refusal, sub_room},
    sync::set_parent_permissions,
    *,
};

const MANAGER: [PrivateRoomCapability; 4] = [
    PrivateRoomCapability::View,
    PrivateRoomCapability::Speak,
    PrivateRoomCapability::Automate,
    PrivateRoomCapability::Manage,
];

fn change(actor: AuthenticatedPrincipal, target: u128, grant: bool) -> ChangeSubRoomManager {
    ChangeSubRoomManager {
        actor,
        parent_catalog_id: catalog_id(PARENT),
        catalog_id: catalog_id(20),
        target: principal_id(target),
        grant,
    }
}

impl Rooms {
    pub(super) async fn set_manager(
        &self,
        actor: u128,
        target: u128,
        grant: bool,
    ) -> Result<(), PrivateRoomFailure> {
        self.service
            .change_sub_room_manager(change(person(actor), target, grant))
            .await
    }

    fn manages(&self, target: u128) -> bool {
        let room = self.store.room(20);
        let in_room = room
            .member(principal_id(target))
            .is_some_and(|member| member.permissions().manage());
        let listed = self.store.link(20).managers.contains(&principal_id(target));
        assert_eq!(in_room, listed, "身上的“管理”和名单一起改");
        in_room
    }

    /// 成员建了 20 号子房间（他管这一间），只能看的人和父房间的管理员都进来了。
    async fn release_with_viewer(&self) {
        self.create(MEMBER, 20, "release").await.expect("成员建的");
        self.join(VIEWER, 20).await.expect("能进");
        self.join(ADMIN, 20).await.expect("父房间的管理员能进");
        self.clear_events();
    }
}

#[tokio::test]
async fn 管这一间的人都能设_身上有管理也在名单上_父房间里一变还管这一间() {
    let rooms = Rooms::new();
    rooms.release_with_viewer().await;

    rooms
        .set_manager(MEMBER, VIEWER, true)
        .await
        .expect("子房间管理员能设");
    assert_eq!(rooms.events(), ["sub_rooms.manager"], "不碰聊天服务器");
    assert!(rooms.manages(VIEWER));
    assert_eq!(
        rooms
            .store
            .room(20)
            .member(principal_id(VIEWER))
            .expect("在里面")
            .permissions(),
        permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Manage]),
        "查看、发言照他在父房间里的，只多一项管理"
    );
    let recorded = rooms
        .store
        .manager_role_changes
        .lock()
        .expect("身份变动锁正常")
        .clone();
    assert_eq!(recorded.len(), 1, "记一笔身份变动");
    assert_eq!(recorded[0].actor, Some(principal_id(MEMBER)));
    assert_eq!(recorded[0].change.target, principal_id(VIEWER));

    // 父房间里给了他发言：子房间照着重算，“管理”看名单，还在。
    set_parent_permissions(&rooms, VIEWER, permissions(&MANAGER[..3])).await;
    assert!(rooms.manages(VIEWER), "重算以后还管这一间");
    assert_eq!(
        rooms
            .store
            .room(20)
            .member(principal_id(VIEWER))
            .expect("在里面")
            .permissions(),
        permissions(&MANAGER),
        "发言跟着父房间来了"
    );

    // 已经是了，再设一次什么都不做。
    rooms.clear_events();
    rooms
        .set_manager(ADMIN, VIEWER, true)
        .await
        .expect("重发不算错");
    assert!(rooms.events().is_empty());
}

#[tokio::test]
async fn 撤别人要父房间的房主或管理员_子房间管理员只能撤自己_不用重新登录() {
    let rooms = Rooms::new();
    rooms.release_with_viewer().await;
    rooms.set_manager(MEMBER, VIEWER, true).await.expect("能设");

    assert_eq!(
        refusal(rooms.set_manager(VIEWER, MEMBER, false).await),
        sub_room(SubRoomRefusal::PeerManager),
        "子房间管理员撤不了另一位"
    );
    assert!(rooms.manages(MEMBER));

    // 自己不当了：登录过了很久也行。
    let mut stale = person(VIEWER);
    stale.recently_authenticated = false;
    rooms
        .service
        .change_sub_room_manager(change(stale, VIEWER, false))
        .await
        .expect("自己不当了不用重新登录");
    assert!(!rooms.manages(VIEWER));

    // 撤别人、设别人都要最近登录过。
    let mut stale = person(ADMIN);
    stale.recently_authenticated = false;
    assert_eq!(
        refusal(
            rooms
                .service
                .change_sub_room_manager(change(stale, MEMBER, false))
                .await
        ),
        PrivateRoomFailureKind::ReauthenticationRequired
    );
    rooms
        .set_manager(ADMIN, MEMBER, false)
        .await
        .expect("父房间的管理员能撤");
    assert!(!rooms.manages(MEMBER));
    assert_eq!(
        rooms
            .store
            .room(20)
            .member(principal_id(MEMBER))
            .expect("还在里面")
            .permissions(),
        permissions(&MANAGER[..3]),
        "撤了管理还在这一间里"
    );

    // 名单空了以后只有父房间的房主和管理员管这一间。
    assert_eq!(
        refusal(rooms.set_manager(MEMBER, VIEWER, true).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    rooms
        .set_manager(OWNER, MEMBER, true)
        .await
        .expect("房主能设回来");
    assert!(rooms.manages(MEMBER));
}

#[tokio::test]
async fn 设不了的_没进这一间的_父房间的房主和管理员_管不了的_归档了的() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.set_manager(MEMBER, VIEWER, true).await),
        sub_room(SubRoomRefusal::TargetNotInside),
        "他得先进这一间"
    );
    assert_eq!(
        refusal(rooms.set_manager(MEMBER, OWNER, true).await),
        sub_room(SubRoomRefusal::ManagedByParent),
        "房主本来就管"
    );
    rooms.join(VIEWER, 20).await.expect("能进");
    rooms.clear_events();
    assert_eq!(
        refusal(rooms.set_manager(VIEWER, VIEWER, true).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "old")
    });
    assert_eq!(
        refusal(
            rooms
                .service
                .change_sub_room_manager(ChangeSubRoomManager {
                    catalog_id: catalog_id(22),
                    ..change(person(OWNER), MEMBER, true)
                })
                .await
        ),
        sub_room(SubRoomRefusal::Archived)
    );
}

#[tokio::test]
async fn 存的当口他被移出了父房间_这一次不写() {
    let rooms = Rooms::new();
    rooms.release_with_viewer().await;
    rooms.store.leave_parent_before_admission(VIEWER);

    assert_eq!(
        refusal(rooms.set_manager(MEMBER, VIEWER, true).await),
        sub_room(SubRoomRefusal::NotInParent)
    );
    assert!(
        !rooms
            .store
            .link(20)
            .managers
            .contains(&principal_id(VIEWER)),
        "没记进名单"
    );
}
