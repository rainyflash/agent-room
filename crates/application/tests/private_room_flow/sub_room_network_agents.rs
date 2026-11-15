//! 网络 Agent 凭口令或敲门进了父房间、自己建子房间（`specs/redesign/sub-rooms.md` 第 2h-1b 步）：它的
//! 合成主体不是房间里的人，子房间里只有房主，它记成这一间的 Agent 成员；同名的回到它建过的那间。
//! 存的当口它被移出父房间时什么都不写，由真实存储的测试看（`sub_room_agent_members.rs`）。

use agent_room_application::private_rooms::{
    AgentSubRoomCreated, CreateAgentSubRoom, PrivateRoomResult,
};

use super::*;

/// 不在房间里的网络 Agent 的合成主体。
const NETWORK: u128 = 91;

fn network_agent() -> AgentId {
    AgentId::from_uuid(uuid(71_000))
}

fn by_network(parent: u128, catalog: u128, name: &str) -> CreateAgentSubRoom {
    CreateAgentSubRoom {
        actor: PrivateRoomActor::NetworkAgent(device(
            principal_id(NETWORK),
            "@user-0123abcd:matrix.test",
        )),
        parent_catalog_id: catalog_id(parent),
        catalog_id: catalog_id(catalog),
        name: name.to_owned(),
        access: RoomJoinMode::Request,
        category: None,
        created_by_agent: network_agent(),
    }
}

/// 父房间里记上凭口令进来的网络 Agent。
fn admit(rooms: &Rooms, catalog: u128) {
    rooms.agent_members.join(
        catalog,
        network_agent(),
        &MatrixUserId::new("@network-agent:matrix.test".to_owned()).expect("用户有效"),
    );
}

fn with_network_agent() -> Rooms {
    let rooms = Rooms::new();
    rooms.agents.own(principal_id(NETWORK), network_agent());
    admit(&rooms, PARENT);
    rooms
}

async fn create(
    rooms: &Rooms,
    request: CreateAgentSubRoom,
) -> PrivateRoomResult<AgentSubRoomCreated> {
    rooms.service.create_agent_sub_room(request).await
}

fn has_link(rooms: &Rooms, catalog: u128) -> bool {
    rooms
        .store
        .links
        .lock()
        .expect("关系锁正常")
        .iter()
        .any(|stored| stored.link.catalog_id == catalog_id(catalog))
}

#[tokio::test]
async fn 凭口令进来的网络_agent_自己建子房间_子房间里只有房主_它记成这一间的_agent_成员() {
    let rooms = with_network_agent();

    let created = create(&rooms, by_network(PARENT, 20, "Its own"))
        .await
        .expect("Agent 成员能建");

    assert!(created.created);
    assert_eq!(created.parent_name, "Project");
    assert_eq!(created.room.parent_catalog_id(), Some(catalog_id(PARENT)));
    assert!(
        created.room.room().member(principal_id(NETWORK)).is_none(),
        "它的合成主体不是里面的人"
    );
    assert!(
        created
            .room
            .room()
            .member(principal_id(OWNER))
            .is_some_and(PrivateRoomMember::has_joined)
    );
    let stored = rooms.store.link(20);
    assert_eq!(stored.link.created_by, principal_id(NETWORK));
    assert_eq!(stored.link.created_by_agent, Some(network_agent()));
    assert_eq!(stored.link.access, RoomJoinMode::Request);
    assert!(stored.managers.is_empty(), "它不是人，不进管理员名单");
    assert_eq!(stored.creator_agent, Some(network_agent()));

    let again = create(&rooms, by_network(PARENT, 21, "its own"))
        .await
        .expect("回到那间");
    assert!(!again.created, "同名的回到它建过的那间，不分大小写");
    assert_eq!(again.room.catalog().id(), catalog_id(20));
    assert_eq!(again.category, None);
}

#[tokio::test]
async fn 别人建的同名子房间不算它的_照规矩说撞了名字() {
    let rooms = with_network_agent();
    rooms
        .create(MEMBER, 20, "Shared")
        .await
        .expect("成员建了一间");

    assert_eq!(
        create(&rooms, by_network(PARENT, 21, "shared"))
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::NameTaken)
    );
}

#[tokio::test]
async fn 不是父房间里已加入的_agent_成员说找不到_房主收紧了照实说_子房间下面不能再开() {
    let rooms = Rooms::new();
    rooms.agents.own(principal_id(NETWORK), network_agent());
    let refused = |result: PrivateRoomResult<AgentSubRoomCreated>| result.unwrap_err().kind();

    assert_eq!(
        refused(create(&rooms, by_network(PARENT, 20, "Nope")).await),
        PrivateRoomFailureKind::NotFound
    );

    admit(&rooms, PARENT);
    rooms
        .change_policy(
            OWNER,
            SubRoomPolicy {
                creators: SubRoomCreators::Admins,
                ..SubRoomPolicy::DEFAULT
            },
        )
        .await
        .expect("房主能改");
    assert_eq!(
        refused(create(&rooms, by_network(PARENT, 21, "Admins")).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AdminsOnly),
        "Agent 成员不是管理员"
    );
    rooms
        .change_policy(
            OWNER,
            SubRoomPolicy {
                agents_may_create: false,
                ..SubRoomPolicy::DEFAULT
            },
        )
        .await
        .expect("房主能改");
    assert_eq!(
        refused(create(&rooms, by_network(PARENT, 22, "Agents")).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AgentsMayNotCreate)
    );
    rooms
        .change_policy(OWNER, SubRoomPolicy::DEFAULT)
        .await
        .expect("房主能改");

    rooms
        .create(OWNER, 23, "Design")
        .await
        .expect("房主建了一间");
    admit(&rooms, 23);
    assert_eq!(
        refused(create(&rooms, by_network(23, 24, "Deeper")).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::Nested)
    );
    assert!(
        [20, 21, 22, 24]
            .into_iter()
            .all(|catalog| !has_link(&rooms, catalog)),
        "被拒的一间也没存"
    );
}
