//! 子房间第 2f-1a 步（`specs/redesign/sub-rooms.md` 的“进门”）：本机 Agent 进子房间之前替它的主人
//! 占好位置。没有门的，主人还不在里面就照他自己点“加入”的那一段把他加进去；有门的不替他加，父房间的
//! 房主和管理员也一样；说不了话的、不在父房间里的、归档了的都进不来。
//!
//! Agent 进门那一头（照这个结果放不放行、回什么码）在 `agent_lobby_entry_flow.rs`。

use agent_room_application::private_rooms::{AgentOwnerSeat, SubRoomAgentOwnerSeating};

use super::*;

impl Rooms {
    async fn seat(&self, owner: u128, catalog: u128) -> AgentOwnerSeat {
        self.service
            .seat_agent_owner(catalog_id(catalog), principal_id(owner))
            .await
            .expect("问得出来")
    }
}

#[tokio::test]
async fn 没有门的子房间_主人还不在就替他加进去_和他自己点加入一样_再来一次不动() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.clear_events();

    assert_eq!(rooms.seat(MEMBER, 20).await, AgentOwnerSeat::Inside);
    let room = rooms.store.room(20);
    let member = room.member(principal_id(MEMBER)).expect("记成了成员");
    assert!(member.has_joined());
    assert_eq!(
        member.permissions(),
        permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Automate,
        ]),
        "权限照父房间里的"
    );
    assert_eq!(
        rooms.events(),
        ["matrix.speak.on", "sub_rooms.admit", "matrix.invite"],
        "和他自己点加入同一段：先设发言级别，再记成员，最后邀请"
    );

    rooms.clear_events();
    assert_eq!(rooms.seat(MEMBER, 20).await, AgentOwnerSeat::Inside);
    assert!(rooms.events().is_empty(), "已经在里面的什么都不动");
}

#[tokio::test]
async fn 有门的不替主人加_父房间的管理员也一样() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(21, "security")
    });
    rooms.clear_events();

    for owner in [MEMBER, ADMIN] {
        assert_eq!(rooms.seat(owner, 21).await, AgentOwnerSeat::DoorClosed);
        assert!(rooms.store.room(21).member(principal_id(owner)).is_none());
    }
    assert!(rooms.events().is_empty());
}

#[tokio::test]
async fn 说不了话的_不在父房间里的_归档了的_不是子房间的_都进不来_什么都不动() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "old")
    });
    rooms.clear_events();

    for (owner, catalog) in [(VIEWER, 20), (OUTSIDER, 20), (MEMBER, 22), (MEMBER, PARENT)] {
        assert_eq!(
            rooms.seat(owner, catalog).await,
            AgentOwnerSeat::Refused,
            "{owner} 进 {catalog}"
        );
    }
    assert!(rooms.store.room(20).member(principal_id(VIEWER)).is_none());
    assert!(rooms.events().is_empty());
}
