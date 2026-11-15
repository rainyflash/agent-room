//! 子房间第 2d-3 步（`specs/redesign/sub-rooms.md` 的“请进来、移出和封禁”）：管得了这一间的人把父
//! 房间里的人直接请进来、移出、封禁、解封，父房间的房主和管理员不用先进来。夹具是 `sub_rooms.rs`
//! 里的那一份，封禁时在等的申请作废照真实存储做在存房间的那一步（`void_banned_requests`）。

use agent_room_application::private_rooms::{GovernSubRoomMember, SubRoomJoinState};
use agent_room_domain::room_join_requests::JoinRequestStatus;

use super::{
    membership::{refusal, sub_room},
    *,
};

const SPEAKER: [PrivateRoomCapability; 3] = [
    PrivateRoomCapability::View,
    PrivateRoomCapability::Speak,
    PrivateRoomCapability::Automate,
];

fn govern(actor: u128, catalog: u128, target: u128) -> GovernSubRoomMember {
    GovernSubRoomMember {
        actor: person(actor),
        parent_catalog_id: catalog_id(PARENT),
        catalog_id: catalog_id(catalog),
        target: principal_id(target),
    }
}

fn status_of(rooms: &Rooms, catalog: u128, person: u128) -> Option<PrivateRoomMembershipStatus> {
    rooms
        .store
        .room(catalog)
        .member(principal_id(person))
        .map(PrivateRoomMember::status)
}

fn request_status(rooms: &Rooms, catalog: u128, person: u128) -> Vec<JoinRequestStatus> {
    rooms
        .store
        .requests_of(catalog)
        .iter()
        .filter(|request| request.requester == principal_id(person))
        .map(|request| request.status)
        .collect()
}

impl Rooms {
    async fn admit(
        &self,
        actor: u128,
        catalog: u128,
        target: u128,
    ) -> Result<(), PrivateRoomFailure> {
        self.service
            .admit_sub_room_member(govern(actor, catalog, target))
            .await
    }

    async fn remove(
        &self,
        actor: u128,
        catalog: u128,
        target: u128,
    ) -> Result<(), PrivateRoomFailure> {
        self.service
            .remove_sub_room_member(govern(actor, catalog, target))
            .await
    }

    async fn ban(
        &self,
        actor: u128,
        catalog: u128,
        target: u128,
    ) -> Result<(), PrivateRoomFailure> {
        self.service
            .ban_sub_room_member(govern(actor, catalog, target))
            .await
    }

    async fn unban(
        &self,
        actor: u128,
        catalog: u128,
        target: u128,
    ) -> Result<(), PrivateRoomFailure> {
        self.service
            .unban_sub_room_member(govern(actor, catalog, target))
            .await
    }

    async fn ask_to_join(&self, person: u128, catalog: u128) -> SubRoomJoinState {
        self.service
            .request_sub_room_join(agent_room_application::private_rooms::RequestSubRoomJoin {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                message: String::new(),
            })
            .await
            .expect("能申请")
    }

    fn seed_door(&self, catalog: u128, name: &str, access: RoomJoinMode) {
        self.store.seed(&Seed {
            access,
            ..Seed::open(catalog, name)
        });
    }
}

#[tokio::test]
async fn 父房间的管理员没进来也能请人_哪种门都行_被禁言着的不给发言级别_在等的申请记成批准() {
    let rooms = Rooms::new();
    rooms.seed_door(20, "security", RoomJoinMode::Request);
    rooms.seed_door(21, "vault", RoomJoinMode::InviteOnly);
    rooms.ask_to_join(MEMBER, 20).await;
    rooms.store.mute(PARENT, MEMBER);
    rooms.clear_events();

    rooms
        .admit(ADMIN, 20, MEMBER)
        .await
        .expect("父房间的管理员能请");
    assert_eq!(
        rooms.events(),
        ["matrix.speak.off", "sub_rooms.admit", "matrix.invite"],
        "和他自己点“加入”同一段：发言级别、记成员、邀请"
    );
    let member = rooms.store.room(20);
    let member = member.member(principal_id(MEMBER)).expect("在里面");
    assert!(member.has_joined(), "不用再点接受");
    assert_eq!(member.permissions(), permissions(&SPEAKER));
    assert_eq!(
        rooms.matrix.speaking_allowed(matrix_user(MEMBER).as_str()),
        Some(false),
        "父房间里禁言着，请进来也不给"
    );
    assert!(
        rooms.store.room(20).member(principal_id(ADMIN)).is_none(),
        "请人的父房间管理员自己不用进来"
    );
    let stored = rooms.store.requests_of(20);
    assert_eq!(stored[0].status, JoinRequestStatus::Approved);
    assert_eq!(stored[0].decided_by, Some(principal_id(ADMIN)));

    // 只能请进来的也请得进来；权限照他在父房间里的。
    rooms.admit(OWNER, 21, VIEWER).await.expect("房主能请");
    assert_eq!(
        rooms
            .store
            .room(21)
            .member(principal_id(VIEWER))
            .expect("在里面")
            .permissions(),
        permissions(&[PrivateRoomCapability::View])
    );

    // 已经在里面的再请一次只补一遍邀请。
    rooms.clear_events();
    rooms
        .admit(OWNER, 21, VIEWER)
        .await
        .expect("再请一次不算错");
    assert_eq!(rooms.events(), ["matrix.invite"]);
}

#[tokio::test]
async fn 请不进来的_不在父房间里的_封禁着的要先解封_管不了的_归档了的() {
    let rooms = Rooms::new();
    rooms.seed_door(20, "security", RoomJoinMode::InviteOnly);
    rooms.store.change(20, |room| {
        room.ban_member(principal_id(OWNER), principal_id(MEMBER))
            .expect("房主能封禁");
    });
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.admit(OWNER, 20, OUTSIDER).await),
        sub_room(SubRoomRefusal::NotInParent)
    );
    assert_eq!(
        refusal(rooms.admit(OWNER, 20, MEMBER).await),
        sub_room(SubRoomRefusal::TargetBanned)
    );
    assert_eq!(
        refusal(rooms.admit(VIEWER, 20, ADMIN).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "old")
    });
    assert_eq!(
        refusal(rooms.admit(OWNER, 22, MEMBER).await),
        sub_room(SubRoomRefusal::Archived)
    );
}

#[tokio::test]
async fn 移出_踢出去连他的_agent_不在里面的不算错_没有门的还能再进() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.join(VIEWER, 20).await.expect("能进");
    let agent = MatrixUserId::new("@agent-of-viewer:matrix.test").expect("用户有效");
    rooms.agents.register(principal_id(VIEWER), &agent);
    rooms
        .matrix
        .set_membership(&agent, PrivateMatrixMembership::Joined);
    rooms.clear_events();

    rooms
        .remove(ADMIN, 20, VIEWER)
        .await
        .expect("父房间的管理员没进来也能移出");
    assert_eq!(
        rooms.events(),
        [
            "matrix.kick",
            "matrix.membership",
            "matrix.kick",
            "store.save"
        ],
        "先在 Matrix 上踢出去、移出他的 Agent，再存"
    );
    assert_eq!(
        status_of(&rooms, 20, VIEWER),
        Some(PrivateRoomMembershipStatus::Removed)
    );
    assert_eq!(
        rooms.matrix.membership_of(&agent),
        Some(PrivateMatrixMembership::Left)
    );

    rooms.clear_events();
    rooms
        .remove(MEMBER, 20, VIEWER)
        .await
        .expect("再移一次不算错");
    rooms
        .remove(MEMBER, 20, OUTSIDER)
        .await
        .expect("不在里面的不算错");
    assert!(rooms.events().is_empty());

    rooms
        .join(VIEWER, 20)
        .await
        .expect("没有门的被移出了还能再进");
}

#[tokio::test]
async fn 谁动得了谁_房主动不了_父房间的管理员只有房主动得了_子房间管理员之间互相动不了() {
    let rooms = Rooms::new();
    rooms
        .create(MEMBER, 20, "release")
        .await
        .expect("成员建的，他管这一间");
    rooms.join(ADMIN, 20).await.expect("父房间的管理员能进");
    rooms.join(VIEWER, 20).await.expect("能进");
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.remove(MEMBER, 20, ADMIN).await),
        PrivateRoomFailureKind::OwnerOnly,
        "父房间的管理员只有房主动得了"
    );
    assert_eq!(
        refusal(rooms.ban(MEMBER, 20, OWNER).await),
        sub_room(SubRoomRefusal::OwnerStays)
    );
    assert_eq!(
        refusal(rooms.remove(MEMBER, 20, MEMBER).await),
        PrivateRoomFailureKind::InvalidRequest,
        "自己出去用“退出”"
    );
    assert_eq!(
        refusal(rooms.remove(VIEWER, 20, MEMBER).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    // 再有一位子房间管理员。
    rooms
        .set_manager(MEMBER, VIEWER, true)
        .await
        .expect("子房间管理员能设");
    assert_eq!(
        refusal(rooms.remove(VIEWER, 20, MEMBER).await),
        sub_room(SubRoomRefusal::PeerManager)
    );
    rooms
        .remove(ADMIN, 20, MEMBER)
        .await
        .expect("父房间的管理员动得了子房间管理员");
    rooms
        .remove(OWNER, 20, ADMIN)
        .await
        .expect("房主动得了管理员");
}

#[tokio::test]
async fn 封禁_在等的申请作废_之后申请不了也请不进来_解封以后回到已移出() {
    let rooms = Rooms::new();
    rooms.seed_door(20, "security", RoomJoinMode::Request);
    rooms.ask_to_join(MEMBER, 20).await;
    rooms.clear_events();

    rooms.ban(OWNER, 20, MEMBER).await.expect("房主能封禁");
    assert_eq!(rooms.events(), ["matrix.ban", "store.save"]);
    assert_eq!(
        status_of(&rooms, 20, MEMBER),
        Some(PrivateRoomMembershipStatus::Banned),
        "从没进来过的也封得了"
    );
    assert_eq!(
        request_status(&rooms, 20, MEMBER),
        [JoinRequestStatus::Voided]
    );
    assert_eq!(
        rooms.matrix.membership_of(&matrix_user(MEMBER)),
        Some(PrivateMatrixMembership::Banned)
    );
    assert_eq!(
        refusal(
            rooms
                .service
                .request_sub_room_join(agent_room_application::private_rooms::RequestSubRoomJoin {
                    actor: person(MEMBER),
                    parent_catalog_id: catalog_id(PARENT),
                    catalog_id: catalog_id(20),
                    message: String::new(),
                })
                .await
        ),
        sub_room(SubRoomRefusal::Banned)
    );
    assert_eq!(
        refusal(rooms.admit(ADMIN, 20, MEMBER).await),
        sub_room(SubRoomRefusal::TargetBanned)
    );
    assert_eq!(
        refusal(rooms.ban(OWNER, 20, OUTSIDER).await),
        sub_room(SubRoomRefusal::NotInParent),
        "不在这间房里、这一间里也没有过记录的不封"
    );

    rooms.clear_events();
    rooms
        .unban(ADMIN, 20, MEMBER)
        .await
        .expect("父房间的管理员能解封");
    assert_eq!(rooms.events(), ["matrix.unban", "store.save"]);
    assert_eq!(
        status_of(&rooms, 20, MEMBER),
        Some(PrivateRoomMembershipStatus::Removed)
    );
    rooms.clear_events();
    rooms
        .unban(ADMIN, 20, MEMBER)
        .await
        .expect("没封着的不算错");
    assert!(rooms.events().is_empty());
    assert_eq!(
        rooms.ask_to_join(MEMBER, 20).await,
        SubRoomJoinState::Pending {
            requested_at: UtcMillis::new(1_000).expect("时间有效")
        },
        "解封以后能再申请"
    );
}
