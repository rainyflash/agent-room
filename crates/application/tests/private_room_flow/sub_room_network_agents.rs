//! 网络 Agent 凭口令或敲门进了父房间、自己建子房间（`specs/redesign/sub-rooms.md` 第 2h-1b 步）：它的
//! 合成主体不是房间里的人，子房间里只有房主，它记成这一间的 Agent 成员；同名的回到它建过的那间。
//! 存的当口它被移出父房间时什么都不写，由真实存储的测试看（`sub_room_agent_members.rs`）。

use agent_room_application::{
    ports::PrivateRoomSnapshot,
    private_rooms::{
        AdmitPersonByAgent, AgentSubRoomArchival, AgentSubRoomCreated, AgentSubRoomListing,
        AgentSubRoomManagement, AgentSubRooms, ArchiveAgentSubRoom, CreateAgentSubRoom,
        FindAgentManagedSubRoom, ListAgentSubRooms, PrivateRoomResult, RequestSubRoomJoin,
    },
};
use agent_room_domain::{
    join_codes::PrivateRoomAgentMemberStatus, room_join_requests::JoinRequestStatus,
    sub_room_archive::SubRoomArchiveReason,
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

/// 父房间是网络 Agent 自己建的：它的合成主体就是房主。房间是它的，谁建的同名子房间都回到那间。
#[tokio::test]
async fn 网络_agent_自己的房间里_谁建的同名子房间都回到那间() {
    let rooms = Rooms::new();
    rooms.agents.own(principal_id(OWNER), network_agent());
    rooms
        .create(MEMBER, 20, "Shared")
        .await
        .expect("成员建了一间");
    let mut request = by_network(PARENT, 21, "shared");
    request.actor =
        PrivateRoomActor::NetworkAgent(device(principal_id(OWNER), "@user-0123abcd:matrix.test"));

    let again = create(&rooms, request).await.expect("回到那间");

    assert!(!again.created);
    assert_eq!(again.room.catalog().id(), catalog_id(20));
    assert!(!has_link(&rooms, 21), "没有另建");
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

// ---- 看子房间名单（2h-2）

async fn listed(rooms: &Rooms, catalog: u128, archived: bool) -> PrivateRoomResult<AgentSubRooms> {
    rooms
        .service
        .agent_sub_rooms(ListAgentSubRooms {
            viewer: principal_id(NETWORK),
            agent_id: network_agent(),
            catalog_id: catalog_id(catalog),
            archived,
        })
        .await
}

fn names(list: &AgentSubRooms) -> Vec<&str> {
    list.rooms
        .iter()
        .map(|item| item.entry.name.as_str())
        .collect()
}

/// 管子房间的人后来把它藏起来了。
fn hide(rooms: &Rooms, catalog: u128) {
    for stored in rooms.store.links.lock().expect("关系锁正常").iter_mut() {
        if stored.link.catalog_id == catalog_id(catalog) {
            stored.link.hidden = true;
        }
    }
}

fn behind_door(catalog: u128, name: &str) -> Seed {
    Seed {
        access: RoomJoinMode::Request,
        hidden: true,
        ..Seed::open(catalog, name)
    }
}

#[tokio::test]
async fn 凭口令进来的网络_agent_看子房间_藏起来的只看得到它建的和它在里面的_分类写名字() {
    let rooms = with_network_agent();
    rooms
        .create_category(ADMIN, 60, "Release")
        .await
        .expect("建了分类");
    rooms.store.seed(&Seed::open(20, "open"));
    rooms.store.seed(&behind_door(21, "hidden-inside"));
    rooms.store.seed(&behind_door(22, "hidden-elsewhere"));
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(23, "archived")
    });
    admit(&rooms, 21);
    create(
        &rooms,
        CreateAgentSubRoom {
            category: Some("release".to_owned()),
            ..by_network(PARENT, 24, "its-own")
        },
    )
    .await
    .expect("它建了一间");
    hide(&rooms, 24);

    let list = listed(&rooms, PARENT, false).await.expect("看得到");
    assert_eq!(list.parent.catalog().id(), catalog_id(PARENT));
    assert_eq!(names(&list), ["open", "hidden-inside", "its-own"]);
    assert_eq!(list.categories, ["Release"]);
    assert_eq!(list.rooms[2].category.as_deref(), Some("Release"));
    assert_eq!(list.rooms[0].category, None);

    let from_sub_room = listed(&rooms, 20, false).await.expect("拿子房间号也行");
    assert_eq!(names(&from_sub_room), names(&list));
    assert_eq!(
        names(&listed(&rooms, PARENT, true).await.expect("归档的")),
        ["archived"]
    );
}

#[tokio::test]
async fn 不是父房间里已加入的_agent_成员看不了名单() {
    let rooms = Rooms::new();
    rooms.agents.own(principal_id(NETWORK), network_agent());
    rooms.store.seed(&Seed::open(20, "open"));

    assert_eq!(
        listed(&rooms, PARENT, false).await.unwrap_err().kind(),
        PrivateRoomFailureKind::NotFound
    );
    admit(&rooms, 20);
    assert_eq!(
        listed(&rooms, 20, false).await.unwrap_err().kind(),
        PrivateRoomFailureKind::NotFound,
        "只在子房间里不算：要在父房间里"
    );
}

#[tokio::test]
async fn 合成主体是父房间里的人时照网页的规矩看_管父房间的看得到藏起来的() {
    let rooms = Rooms::new();
    rooms.store.seed(&behind_door(21, "hidden"));
    let as_viewer = |viewer: u128| ListAgentSubRooms {
        viewer: principal_id(viewer),
        agent_id: network_agent(),
        catalog_id: catalog_id(PARENT),
        archived: false,
    };

    let owner = rooms
        .service
        .agent_sub_rooms(as_viewer(OWNER))
        .await
        .expect("房主看得到");
    assert_eq!(names(&owner), ["hidden"]);
    let member = rooms
        .service
        .agent_sub_rooms(as_viewer(MEMBER))
        .await
        .expect("成员看得到名单");
    assert!(names(&member).is_empty(), "藏起来的成员看不到");
}

// ---- 归档（2h-3a）

async fn archived_by(
    rooms: &Rooms,
    principal: u128,
    catalog: u128,
) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
    rooms
        .service
        .archive_agent_sub_room(ArchiveAgentSubRoom {
            actor: PrivateRoomActor::NetworkAgent(device(
                principal_id(principal),
                "@user-0123abcd:matrix.test",
            )),
            agent_id: network_agent(),
            catalog_id: catalog_id(catalog),
        })
        .await
}

fn refused_with(result: PrivateRoomResult<Option<PrivateRoomSnapshot>>) -> PrivateRoomFailureKind {
    result.expect_err("应当被拒").kind()
}

#[tokio::test]
async fn 凭口令进来的网络_agent_归档它建的子房间_记它的合成主体_再关一次照样成功() {
    let rooms = with_network_agent();
    create(&rooms, by_network(PARENT, 20, "Its own"))
        .await
        .expect("它建了一间");
    // 真实存储建的时候就把它记成这一间的 Agent 成员（`seat_creator_agent`），替身要自己记。
    admit(&rooms, 20);

    let archived = archived_by(&rooms, NETWORK, 20)
        .await
        .expect("建它的 Agent 能归档")
        .expect("是子房间");

    assert_eq!(
        archived.room().status(),
        PrivateRoomLifecycleStatus::Archived
    );
    let record = rooms.store.link(20).link.archived.expect("记了归档");
    assert_eq!(record.by, Some(principal_id(NETWORK)));
    assert_eq!(record.reason, SubRoomArchiveReason::Manual);
    assert_eq!(
        rooms
            .matrix
            .changes_in(archived.instance().matrix_room_id().as_str()),
        ["archive"],
        "请求里就在聊天服务器上锁住了"
    );

    let again = archived_by(&rooms, NETWORK, 20)
        .await
        .expect("已经归档的再关一次照样成功")
        .expect("是子房间");
    assert_eq!(again.room().status(), PrivateRoomLifecycleStatus::Archived);
    assert_eq!(
        rooms.store.link(20).link.archived,
        Some(record),
        "不再记一遍"
    );
    assert_eq!(
        archived_by(&rooms, NETWORK, PARENT).await.expect("整间房"),
        None,
        "不是子房间：照整间房关"
    );
}

#[tokio::test]
async fn 别人建的_它被移出的_归档不了_不在父房间里的说找不到() {
    let rooms = with_network_agent();
    rooms.store.seed(&Seed::open(20, "someone else's"));
    admit(&rooms, 20);
    create(&rooms, by_network(PARENT, 21, "Its own"))
        .await
        .expect("它建了一间");
    admit(&rooms, 21);

    assert_eq!(
        refused_with(archived_by(&rooms, NETWORK, 20).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "在里面也不行：不是它建的"
    );
    rooms
        .agent_members
        .set_status(21, network_agent(), PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(
        refused_with(archived_by(&rooms, NETWORK, 21).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "被移出了这一间"
    );
    assert_eq!(
        rooms.store.room(21).status(),
        PrivateRoomLifecycleStatus::Active
    );

    let outside = Rooms::new();
    outside.agents.own(principal_id(NETWORK), network_agent());
    outside.store.seed(&Seed::open(20, "open"));
    assert_eq!(
        refused_with(archived_by(&outside, NETWORK, 20).await),
        PrivateRoomFailureKind::NotFound
    );
    assert_eq!(
        refused_with(archived_by(&outside, NETWORK, 99).await),
        PrivateRoomFailureKind::NotFound,
        "没有这间房"
    );
}

#[tokio::test]
async fn 合成主体是父房间的房主时照房主归档_谁建的都行_本机_agent_不走这一路() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));

    let archived = archived_by(&rooms, OWNER, 20)
        .await
        .expect("房主能归档")
        .expect("是子房间");
    assert_eq!(
        archived.room().status(),
        PrivateRoomLifecycleStatus::Archived
    );
    assert_eq!(
        rooms.store.link(20).link.archived.map(|record| record.by),
        Some(Some(principal_id(OWNER)))
    );

    rooms.store.seed(&Seed::open(21, "other"));
    let by_device = rooms
        .service
        .archive_agent_sub_room(ArchiveAgentSubRoom {
            actor: PrivateRoomActor::Device(device(
                principal_id(OWNER),
                "@user-0123abcd:matrix.test",
            )),
            agent_id: network_agent(),
            catalog_id: catalog_id(21),
        })
        .await;
    assert_eq!(refused_with(by_device), PrivateRoomFailureKind::Forbidden);
    assert_eq!(
        rooms.store.room(21).status(),
        PrivateRoomLifecycleStatus::Active
    );
}

// ---- 认出它管得了的子房间（2h-4）

async fn managed_by(
    rooms: &Rooms,
    principal: u128,
    catalog: u128,
) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
    rooms
        .service
        .agent_managed_sub_room(FindAgentManagedSubRoom {
            actor: PrivateRoomActor::NetworkAgent(device(
                principal_id(principal),
                "@user-0123abcd:matrix.test",
            )),
            agent_id: network_agent(),
            catalog_id: catalog_id(catalog),
        })
        .await
}

#[tokio::test]
async fn 合成主体是父房间的房主时_每间子房间都管得了_整间房回空() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));

    let managed = managed_by(&rooms, OWNER, 20)
        .await
        .expect("房主管得了")
        .expect("是子房间");
    assert_eq!(managed.catalog().id(), catalog_id(20));
    assert_eq!(
        managed_by(&rooms, OWNER, PARENT).await.expect("整间房"),
        None,
        "不是子房间：照整间房认"
    );
}

#[tokio::test]
async fn 凭口令进来的网络_agent_管得了它建的_还在里面的子房间() {
    let rooms = with_network_agent();
    create(&rooms, by_network(PARENT, 20, "Its own"))
        .await
        .expect("它建了一间");
    admit(&rooms, 20);

    let managed = managed_by(&rooms, NETWORK, 20)
        .await
        .expect("建它的 Agent 管得了")
        .expect("是子房间");
    assert_eq!(managed.catalog().id(), catalog_id(20));

    rooms
        .agent_members
        .set_status(20, network_agent(), PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(
        refused_with(managed_by(&rooms, NETWORK, 20).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "被移出了这一间"
    );
}

#[tokio::test]
async fn 别人建的管不了_归档了的管不了_不在父房间里的说找不到_本机_agent_不走这一路() {
    let rooms = with_network_agent();
    rooms.store.seed(&Seed::open(20, "someone else's"));
    admit(&rooms, 20);
    assert_eq!(
        refused_with(managed_by(&rooms, NETWORK, 20).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "在里面也不行：不是它建的"
    );

    create(&rooms, by_network(PARENT, 21, "Its own"))
        .await
        .expect("它建了一间");
    admit(&rooms, 21);
    archived_by(&rooms, NETWORK, 21)
        .await
        .expect("归档")
        .expect("是子房间");
    assert_eq!(
        refused_with(managed_by(&rooms, NETWORK, 21).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "归档了的不管"
    );

    let outside = Rooms::new();
    outside.agents.own(principal_id(NETWORK), network_agent());
    outside.store.seed(&Seed::open(20, "open"));
    assert_eq!(
        refused_with(managed_by(&outside, NETWORK, 20).await),
        PrivateRoomFailureKind::NotFound
    );

    let by_device = rooms
        .service
        .agent_managed_sub_room(FindAgentManagedSubRoom {
            actor: PrivateRoomActor::Device(device(
                principal_id(OWNER),
                "@user-0123abcd:matrix.test",
            )),
            agent_id: network_agent(),
            catalog_id: catalog_id(20),
        })
        .await;
    assert_eq!(refused_with(by_device), PrivateRoomFailureKind::Forbidden);
}

// ---- 人的申请、把父房间里的人请进来（2h-4b）

fn agent_room(principal: u128, catalog: u128) -> FindAgentManagedSubRoom {
    FindAgentManagedSubRoom {
        actor: PrivateRoomActor::NetworkAgent(device(
            principal_id(principal),
            "@user-0123abcd:matrix.test",
        )),
        agent_id: network_agent(),
        catalog_id: catalog_id(catalog),
    }
}

async fn bring_in(
    rooms: &Rooms,
    principal: u128,
    catalog: u128,
    target: u128,
) -> PrivateRoomResult<()> {
    rooms
        .service
        .admit_person_by_agent(AdmitPersonByAgent {
            room: agent_room(principal, catalog),
            target: principal_id(target),
        })
        .await
}

fn failed_with<T: std::fmt::Debug>(result: PrivateRoomResult<T>) -> PrivateRoomFailureKind {
    result.expect_err("应该被拒").kind()
}

#[tokio::test]
async fn 网络_agent_看得到它管的子房间里的申请_请人进来就是批准_记它的合成主体() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(20, "security")
    });
    rooms
        .service
        .request_sub_room_join(RequestSubRoomJoin {
            actor: person(MEMBER),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(20),
            message: "想来帮忙".to_owned(),
        })
        .await
        .expect("能申请");

    let pending = rooms
        .service
        .agent_sub_room_join_requests(agent_room(OWNER, 20))
        .await
        .expect("合成主体是房主，看得到");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].record.requester, principal_id(MEMBER));
    assert_eq!(pending[0].record.message.as_str(), "想来帮忙");
    assert!(
        rooms
            .service
            .agent_sub_room_join_requests(agent_room(OWNER, PARENT))
            .await
            .expect("整间房")
            .is_empty(),
        "整间房没有申请"
    );

    rooms.clear_events();
    bring_in(&rooms, OWNER, 20, MEMBER).await.expect("请得进来");
    assert_eq!(
        rooms.events().last(),
        Some(&"matrix.invite"),
        "和人请进来同一段，最后才邀请"
    );
    assert!(
        rooms
            .store
            .room(20)
            .member(principal_id(MEMBER))
            .is_some_and(PrivateRoomMember::has_joined),
        "直接进来，不用再点接受"
    );
    let stored = rooms.store.requests_of(20);
    assert_eq!(stored[0].status, JoinRequestStatus::Approved);
    assert_eq!(stored[0].decided_by, Some(principal_id(OWNER)));
    assert!(
        rooms
            .service
            .agent_sub_room_join_requests(agent_room(OWNER, 20))
            .await
            .expect("看得到")
            .is_empty(),
        "批准了就不在等了"
    );

    bring_in(&rooms, OWNER, 20, MEMBER)
        .await
        .expect("已经在里面的再请一次不算错");
}

#[tokio::test]
async fn 请不进来的_不在父房间里的_封禁着的_不是子房间的_它管不了的() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::InviteOnly,
        ..Seed::open(20, "vault")
    });
    rooms.store.change(20, |room| {
        room.ban_member(principal_id(OWNER), principal_id(MEMBER))
            .expect("房主能封禁");
    });
    rooms.clear_events();

    assert_eq!(
        failed_with(bring_in(&rooms, OWNER, 20, OUTSIDER).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::NotInParent)
    );
    assert_eq!(
        failed_with(bring_in(&rooms, OWNER, 20, MEMBER).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::TargetBanned)
    );
    assert_eq!(
        failed_with(bring_in(&rooms, OWNER, PARENT, MEMBER).await),
        PrivateRoomFailureKind::NotFound,
        "整间房不走这一路"
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    // 凭口令进了父房间的网络 Agent：它建的、还在里面的请得进来，别人建的管不了。
    let rooms = with_network_agent();
    create(&rooms, by_network(PARENT, 21, "Its own"))
        .await
        .expect("它建了一间");
    admit(&rooms, 21);
    bring_in(&rooms, NETWORK, 21, VIEWER)
        .await
        .expect("它建的请得进来");
    assert!(
        rooms
            .store
            .room(21)
            .member(principal_id(VIEWER))
            .is_some_and(PrivateRoomMember::has_joined)
    );
    rooms.store.seed(&Seed::open(22, "someone else's"));
    admit(&rooms, 22);
    assert_eq!(
        failed_with(bring_in(&rooms, NETWORK, 22, VIEWER).await),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly)
    );
    assert_eq!(
        failed_with(
            rooms
                .service
                .agent_sub_room_join_requests(agent_room(NETWORK, 22))
                .await
        ),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
        "管不了的也看不到申请"
    );
}
