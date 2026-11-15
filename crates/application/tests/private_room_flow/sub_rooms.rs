//! 子房间（`specs/redesign/sub-rooms.md` 第 2a 步）：在一间房下面建、读名单、三项设置，和只有
//! 父房间有的那几件事。存储是内存里的一份，重名、上限这些规则照真实存储的写；真实存储自己的
//! 测试在 `postgres-adapter` 的 `tests/private_rooms/sub_rooms.rs`。人加入、退出和改名（2b）的
//! 用例在旁边的 `sub_room_membership.rs`，父房间变了以后聊天服务器上跟着做（2c-2）的在
//! `sub_room_sync.rs`，门（2d-1）的在 `sub_room_doors.rs`，申请和批准（2d-2）的在
//! `sub_room_join_requests.rs`，分类（2e-1）的在 `sub_room_categories.rs`，用的都是这里的夹具。

use std::collections::BTreeSet;

use agent_room_application::{
    ports::{
        JoinRequestRecord, NewSubRoom, SubRoomAdmission, SubRoomArchiveRecord,
        SubRoomCreationBlock, SubRoomCreationCheck, SubRoomEntry, SubRoomJoinRequestOutcome,
        SubRoomLink, SubRoomManagerChange, SubRoomParentFacts, SubRoomPolicyRecord,
        SubRoomRevision, SubRoomRevisionBlock, SubRoomStore, SubRoomSyncScope, SubRoomSyncTask,
    },
    private_rooms::{
        ChangeSubRoomPolicy, CreateSubRoom, InspectSubRoomPolicy, ListSubRooms, ReviseSubRoom,
        SubRoomMembershipAction, SubRoomRefusal, SubRoomSyncOutcome, SubRoomSyncUseCases,
        SubRoomUseCases,
    },
};
use agent_room_domain::{
    ids::SubRoomCategoryId,
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus, PrivateRoomMember},
    room_listing::RoomJoinMode,
    rooms::{
        RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility, RoomInstance, RoomInstanceFields, RoomInstanceState,
    },
    sub_room_archive::SubRoomArchiveReason,
    sub_rooms::{
        MAXIMUM_OPEN_SUB_ROOMS, MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR, SubRoomCreators,
        SubRoomPolicy, follow_parent_room,
    },
};

use super::*;

#[path = "sub_room_accessible.rs"]
mod accessible;
#[path = "sub_room_agent_owner.rs"]
mod agent_owner;
#[path = "sub_room_archive.rs"]
mod archive;
#[path = "sub_room_archived_list.rs"]
mod archived_list;
#[path = "sub_room_categories.rs"]
mod categories;
#[path = "sub_room_doors.rs"]
mod doors;
#[path = "sub_room_join_requests.rs"]
mod join_requests;
#[path = "sub_room_managers.rs"]
mod managers;
#[path = "sub_room_members.rs"]
mod members;
#[path = "sub_room_membership.rs"]
mod membership;
#[path = "sub_room_mutes.rs"]
mod mutes;
#[path = "sub_room_privacy.rs"]
mod privacy;
#[path = "sub_room_sync.rs"]
mod sync;

const PARENT: u128 = 10;
const OWNER: u128 = 1;
const MEMBER: u128 = 2;
const ADMIN: u128 = 3;
const VIEWER: u128 = 4;
const OUTSIDER: u128 = 5;
/// 以前在房间里的人：直接放进存储的子房间记成他建的，不算进现在这些人的限速。
const EARLIER: u128 = 8;

#[tokio::test]
async fn 成员建子房间_房主和他都在里面_他管这一间_加密和保留期跟父房间() {
    let rooms = Rooms::new();
    let created = rooms
        .create(MEMBER, 20, "release")
        .await
        .expect("能发言的成员能建");

    assert_eq!(created.parent_catalog_id(), Some(catalog_id(PARENT)));
    assert_eq!(created.catalog().retention_days(), Some(30));
    let room = created.room();
    assert_eq!(room.owner_principal_id(), principal_id(OWNER));
    let creator = room.member(principal_id(MEMBER)).expect("建的人在里面");
    assert!(creator.has_joined(), "不用再点接受");
    assert_eq!(
        creator.permissions(),
        permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Automate,
            PrivateRoomCapability::Manage,
        ]),
        "照他在父房间里的，加上管这一间；子房间不能往外请人"
    );
    assert_eq!(room.members().len(), 2, "别的人要自己加入");

    let creation = rooms.matrix.creation().expect("建了 Matrix 房间");
    let invited: Vec<_> = creation
        .request()
        .invite()
        .iter()
        .map(MatrixUserId::as_str)
        .collect();
    assert_eq!(
        invited,
        [
            "@owner:matrix.test",
            "@member:matrix.test",
            "@content-authority:matrix.test"
        ]
    );
    assert_eq!(
        creation.request().encryption(),
        agent_room_application::ports::MatrixRoomEncryption::EndToEnd
    );
    assert_eq!(creation.request().retention_days(), Some(30));
    assert_eq!(
        rooms.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );
    assert_eq!(
        rooms
            .matrix
            .speaking_allowed("@content-authority:matrix.test"),
        Some(false)
    );

    let link = rooms.store.link(20);
    assert_eq!(link.link.parent_catalog_id, catalog_id(PARENT));
    assert_eq!(link.link.access, RoomJoinMode::Open);
    assert!(!link.link.hidden);
    assert_eq!(link.link.created_by, principal_id(MEMBER));
    assert_eq!(link.managers, [principal_id(MEMBER)]);
    assert_eq!(
        rooms.events(),
        [
            "sub_rooms.block",
            "matrix.create",
            "matrix.speak.batch",
            "sub_rooms.create"
        ],
        "重名和上限在建 Matrix 房间之前先问一遍"
    );
}

#[tokio::test]
async fn 房主自己建_里面只有他_不另记管理员_公开的父房间下面不加密() {
    let rooms = Rooms::new();
    rooms.store.set_encrypted(PARENT, false);
    let created = rooms.create(OWNER, 20, "release").await.expect("房主能建");

    assert_eq!(created.room().members().len(), 1);
    assert!(rooms.store.link(20).managers.is_empty());
    let creation = rooms.matrix.creation().expect("建了 Matrix 房间");
    assert_eq!(
        creation.request().encryption(),
        agent_room_application::ports::MatrixRoomEncryption::Unencrypted,
        "没装过门的子房间跟父房间"
    );
    assert!(
        creation
            .alias()
            .as_str()
            .starts_with("agent-room-public-room-")
    );
}

#[tokio::test]
async fn 不在房间里的人建不了_只能看的也建不了_归档的房间下面不能建() {
    let rooms = Rooms::new();
    assert_eq!(
        rooms
            .create(OUTSIDER, 20, "release")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::NotFound,
        "这间房对他来说不存在"
    );
    assert_eq!(
        rooms
            .create(VIEWER, 20, "release")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::Forbidden
    );
    assert!(rooms.events().is_empty(), "都没走到 Matrix");

    rooms.store.archive(PARENT);
    assert_eq!(
        rooms.create(OWNER, 20, "release").await.unwrap_err().kind(),
        PrivateRoomFailureKind::Conflict
    );
}

#[tokio::test]
async fn 房主定了只有管理员能建_成员建不了_管理员和房主照样能建() {
    let rooms = Rooms::new();
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
        rooms
            .create(MEMBER, 20, "release")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AdminsOnly)
    );
    rooms
        .create(ADMIN, 21, "release")
        .await
        .expect("管理员能建");
    rooms.create(OWNER, 22, "website").await.expect("房主能建");
}

#[tokio::test]
async fn agent_替主人建_记下是哪个_agent_房主关掉以后_agent_建不了_人照样能建() {
    let rooms = Rooms::new();
    let agent = agent_id(71);
    rooms.agents.own(principal_id(MEMBER), agent);
    let by_agent = |catalog: u128, agent: Option<AgentId>| CreateSubRoom {
        actor: PrivateRoomActor::Device(device(principal_id(MEMBER), "@member:matrix.test")),
        parent_catalog_id: catalog_id(PARENT),
        catalog_id: catalog_id(catalog),
        name: format!("release-{catalog}"),
        description: String::new(),
        access: RoomJoinMode::Open,
        hidden: false,
        category_id: None,
        created_by_agent: agent,
    };

    rooms
        .service
        .create_sub_room(by_agent(20, Some(agent)))
        .await
        .expect("Agent 替主人建");
    let link = rooms.store.link(20);
    assert_eq!(link.link.created_by, principal_id(MEMBER));
    assert_eq!(link.link.created_by_agent, Some(agent));
    assert_eq!(
        link.managers,
        [principal_id(MEMBER)],
        "主人是这一间的管理员"
    );

    // 设备替账号建要说是哪个 Agent，还得是这个账号名下的。
    for (request, expected) in [
        (by_agent(21, None), PrivateRoomFailureKind::InvalidRequest),
        (
            by_agent(21, Some(agent_id(72))),
            PrivateRoomFailureKind::Forbidden,
        ),
    ] {
        assert_eq!(
            rooms
                .service
                .create_sub_room(request)
                .await
                .unwrap_err()
                .kind(),
            expected
        );
    }

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
        rooms
            .service
            .create_sub_room(by_agent(21, Some(agent)))
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AgentsMayNotCreate)
    );
    rooms
        .create(MEMBER, 22, "by hand")
        .await
        .expect("人自己照样能建");
}

#[tokio::test]
async fn 重名不分大小写_名字里带斜杠或两头带空白的不收_都在建_matrix_房间之前拒() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "Release").await.expect("第一间");
    rooms.clear_events();

    assert_eq!(
        rooms.create(ADMIN, 21, "release").await.unwrap_err().kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::NameTaken)
    );
    assert_eq!(rooms.events(), ["sub_rooms.block"]);
    rooms.clear_events();

    for name in ["a/b", " padded", "padded ", ""] {
        assert_eq!(
            rooms.create(ADMIN, 21, name).await.unwrap_err().kind(),
            PrivateRoomFailureKind::InvalidRequest,
            "{name:?}"
        );
    }
    assert!(rooms.events().is_empty());

    // 归档的让出名字。
    rooms.store.archive(20);
    rooms
        .create(ADMIN, 21, "release")
        .await
        .expect("名字让出来了");
}

#[tokio::test]
async fn 同一个请求重发回同一间_不再建_matrix_房间_换了名字或换了人算撞号() {
    let rooms = Rooms::new();
    let first = rooms.create(MEMBER, 20, "release").await.expect("第一次");
    rooms.clear_events();

    let again = rooms.create(MEMBER, 20, "release").await.expect("重发");
    assert_eq!(again, first);
    assert!(rooms.events().is_empty());

    assert_eq!(
        rooms
            .create(MEMBER, 20, "website")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::Conflict
    );
    assert_eq!(
        rooms.create(ADMIN, 20, "release").await.unwrap_err().kind(),
        PrivateRoomFailureKind::Conflict
    );
}

#[tokio::test]
async fn 存的时候才撞上同一个房间号_按重试处理() {
    let rooms = Rooms::new();
    // 另一个一模一样的请求在这次问过“有没有”以后、存之前抢先存好了。
    rooms.store.race_same_request_once();
    let created = rooms.create(MEMBER, 20, "release").await.expect("算成功");
    assert_eq!(created.parent_catalog_id(), Some(catalog_id(PARENT)));
    assert_eq!(rooms.store.link(20).link.created_by, principal_id(MEMBER));
}

#[tokio::test]
async fn 子房间下面不能再开_一间房最多一百间_一个人一小时最多建十间() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("第一间");
    let nested = rooms
        .service
        .create_sub_room(CreateSubRoom {
            parent_catalog_id: catalog_id(20),
            ..Rooms::request(MEMBER, 21, "deeper")
        })
        .await
        .unwrap_err();
    assert_eq!(
        nested.kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::Nested)
    );

    for index in 1..MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR {
        let catalog = 100 + u128::from(index);
        rooms
            .create(MEMBER, catalog, &format!("topic-{index}"))
            .await
            .expect("一小时里的前十间");
    }
    assert_eq!(
        rooms
            .create(MEMBER, 199, "one more")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::RateLimited)
    );
    rooms
        .create(ADMIN, 199, "one more")
        .await
        .expect("别人不受影响");

    // 别的人早些时候建的把这间房填到一百间。
    let open = u128::from(MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR) + 1;
    for index in open..u128::from(MAXIMUM_OPEN_SUB_ROOMS) {
        rooms
            .store
            .seed(&Seed::open(1_000 + index, &format!("old-{index}")));
    }
    assert_eq!(
        rooms
            .create(OWNER, 198, "too many")
            .await
            .unwrap_err()
            .kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::LimitReached)
    );
}

#[tokio::test]
async fn 名单给房间里的人看_没进的也列出来_归档的不列_隐藏的只给里面的人和管房间的人() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.store.seed(&Seed::open(21, "website"));
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "done")
    });
    rooms.store.seed(&Seed {
        access: RoomJoinMode::InviteOnly,
        hidden: true,
        ..Seed::open(23, "security")
    });

    let names = |list: &agent_room_application::private_rooms::SubRoomList| {
        list.sub_rooms
            .iter()
            .map(|view| view.entry.name.clone())
            .collect::<Vec<_>>()
    };
    let list = rooms.list(MEMBER, PARENT).await.expect("成员能读");
    assert_eq!(list.parent_catalog_id, catalog_id(PARENT));
    assert_eq!(list.parent_name, "Project");
    assert!(list.can_create);
    assert_eq!(names(&list), ["release", "website"]);
    let release = &list.sub_rooms[0];
    assert_eq!(
        release.entry.viewer_status,
        Some(PrivateRoomMembershipStatus::Joined)
    );
    assert_eq!(release.entry.member_count, 2);
    assert_eq!(release.entry.managers, [principal_id(MEMBER)]);
    assert!(release.inside_visible);
    assert_eq!(
        list.sub_rooms[1].entry.viewer_status, None,
        "没进的也看得到"
    );
    assert!(list.sub_rooms[1].inside_visible, "没有门的给人数");

    // 拿子房间的房间号来问，回的是它父房间的名单。
    let through_sub_room = rooms.list(MEMBER, 20).await.expect("子房间的页面这样问");
    assert_eq!(through_sub_room, list);

    let for_admin = rooms.list(ADMIN, PARENT).await.expect("管理员能读");
    assert_eq!(names(&for_admin), ["release", "website", "security"]);
    assert!(
        !for_admin.sub_rooms[2].inside_visible,
        "有门又没进的不给人数"
    );
    let for_viewer = rooms.list(VIEWER, PARENT).await.expect("只能看的也能读");
    assert!(!for_viewer.can_create);

    for (viewer, catalog) in [
        (OUTSIDER, PARENT),
        (OUTSIDER, 20),
        (MEMBER, 23),
        (MEMBER, 999),
    ] {
        assert_eq!(
            rooms.list(viewer, catalog).await.unwrap_err().kind(),
            PrivateRoomFailureKind::NotFound,
            "{viewer} 问 {catalog}"
        );
    }
    rooms.list(ADMIN, 23).await.expect("管房间的人看得到隐藏的");
    rooms
        .list(MEMBER, 22)
        .await
        .expect("归档的还查得到挂在谁下面");
}

#[tokio::test]
async fn 三项设置房间里的人都能读_只有房主能改_没变的不写_子房间上没有这几项() {
    let rooms = Rooms::new();
    let read = |viewer: u128, parent: u128| {
        rooms.service.sub_room_policy(InspectSubRoomPolicy {
            actor: person(viewer),
            parent_catalog_id: catalog_id(parent),
        })
    };

    let default = read(MEMBER, PARENT).await.expect("成员能读");
    assert_eq!(default.policy, SubRoomPolicy::DEFAULT);
    assert!(!default.can_change);
    assert!(read(OWNER, PARENT).await.expect("房主能读").can_change);
    assert_eq!(
        read(OUTSIDER, PARENT).await.unwrap_err().kind(),
        PrivateRoomFailureKind::NotFound
    );

    let strict = SubRoomPolicy {
        creators: SubRoomCreators::Admins,
        agents_may_create: false,
        auto_archive: false,
    };
    assert_eq!(
        rooms.change_policy(ADMIN, strict).await.unwrap_err().kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::PolicyOwnerOnly),
        "管理员也改不了"
    );
    rooms
        .change_policy(OWNER, SubRoomPolicy::DEFAULT)
        .await
        .expect("和默认值一样");
    assert_eq!(rooms.store.policy_writes(), 0, "没变的不写");

    let changed = rooms.change_policy(OWNER, strict).await.expect("房主能改");
    assert_eq!(changed.policy, strict);
    assert_eq!(
        read(MEMBER, PARENT).await.expect("读得到新的").policy,
        strict
    );
    rooms.change_policy(OWNER, strict).await.expect("再存一次");
    rooms
        .change_policy(OWNER, SubRoomPolicy::DEFAULT)
        .await
        .expect("改回来");
    assert_eq!(rooms.store.policy_writes(), 2);
    assert_eq!(rooms.store.policy_version(), 2, "每改一次版本加一");

    rooms.create(OWNER, 20, "release").await.expect("建一间");
    assert_eq!(
        read(OWNER, 20).await.unwrap_err().kind(),
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ParentOnly)
    );
}

#[tokio::test]
async fn 往外请人_单独改权限_转让_整间归档_改名_子房间都没有_到父房间去做() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.clear_events();
    let sub_room = catalog_id(20);
    let parent_only = PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ParentOnly);
    let owner = || person(OWNER);
    let service = &rooms.service;

    let invited = service
        .invite(InvitePrivateRoomMember {
            actor: owner().into(),
            catalog_id: sub_room,
            target_principal_id: principal_id(ADMIN),
            permissions: speaker_permissions(),
        })
        .await;
    assert_eq!(invited.unwrap_err().kind(), parent_only);
    let changed = service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: owner(),
            catalog_id: sub_room,
            target_principal_id: principal_id(MEMBER),
            permissions: viewer_permissions(),
        })
        .await;
    assert_eq!(changed.unwrap_err().kind(), parent_only);
    let transferred = service
        .transfer_ownership(TransferPrivateRoomOwnership {
            actor: owner(),
            catalog_id: sub_room,
            target_principal_id: principal_id(MEMBER),
            former_owner_permissions: speaker_permissions(),
        })
        .await;
    assert_eq!(transferred.unwrap_err().kind(), parent_only);
    let archived = service
        .archive(ArchivePrivateRoom {
            actor: owner().into(),
            catalog_id: sub_room,
        })
        .await;
    assert_eq!(archived.unwrap_err().kind(), parent_only);
    let renamed = service
        .rename(RenamePrivateRoom {
            actor: owner(),
            catalog_id: sub_room,
            name: "renamed".to_owned(),
        })
        .await;
    assert_eq!(renamed.unwrap_err().kind(), parent_only);
    assert!(rooms.events().is_empty(), "什么都没动");

    // 不在这间子房间里的人拿到的回答和别的进不去的房间一样，看不出这是一间子房间。
    let by_stranger = service
        .rename(RenamePrivateRoom {
            actor: person(ADMIN),
            catalog_id: sub_room,
            name: "renamed".to_owned(),
        })
        .await;
    assert_eq!(
        by_stranger.unwrap_err().kind(),
        PrivateRoomFailureKind::Forbidden
    );
}

#[tokio::test]
async fn 房间列表里没有子房间_按房间号照样读得到() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");

    let listed = rooms
        .service
        .list(ListPrivateRooms {
            actor: person(MEMBER),
        })
        .await
        .expect("列得出");
    let ids: Vec<_> = listed.iter().map(|room| room.catalog().id()).collect();
    assert_eq!(
        ids,
        [catalog_id(PARENT)],
        "旧版客户端不会把子房间当成普通房间摆出来"
    );

    let inspected = rooms
        .service
        .inspect(InspectPrivateRoom {
            actor: person(MEMBER),
            catalog_id: catalog_id(20),
        })
        .await
        .expect("里面的人按房间号读得到");
    assert_eq!(inspected.parent_catalog_id(), Some(catalog_id(PARENT)));
}

// ---- 夹具

struct Rooms {
    service: PrivateRoomService,
    store: Arc<MemoryRooms>,
    matrix: Arc<TestMatrix>,
    agents: Arc<TestAgentDirectory>,
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Rooms {
    /// 一间父房间：房主、一个能发言的成员、一个管理员、一个只能看的；还有一个不在房间里的人。
    fn new() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let store = Arc::new(MemoryRooms::new(events.clone()));
        store.put(parent_room());
        let matrix = Arc::new(TestMatrix::new(events.clone()));
        let principals = Arc::new(TestPrincipalDirectory::new(
            [OWNER, MEMBER, ADMIN, VIEWER, OUTSIDER]
                .into_iter()
                .map(|person| (principal_id(person), matrix_user(person)))
                .collect(),
        ));
        let agents = Arc::new(TestAgentDirectory::default());
        let listings = Arc::new(TestListings::new(
            Arc::new(TestStore::new(events.clone())),
            events.clone(),
        ));
        let runtime = Arc::new(TestRuntime);
        let service = PrivateRoomService::new(PrivateRoomDependencies {
            store: store.clone(),
            matrix_provisioner: matrix.clone(),
            matrix: matrix.clone(),
            principals,
            agents: agents.clone(),
            agent_members: Arc::new(agent_members::TestAgentMembers::default()),
            trusted_matrix_readers: vec![
                MatrixUserId::new("@content-authority:matrix.test").expect("服务身份有效"),
            ],
            // 装门开加密、拆门写下那一刻走的是公开设置那两个口子：落在这里的存储和 Matrix 替身上。
            listings: store.clone(),
            listing_matrix: matrix.clone(),
            secrets: listings,
            sub_rooms: store.clone(),
            topics: matrix.clone(),
            moderation: store.clone(),
            join_requests: store.clone(),
            identifiers: runtime.clone(),
            clock: runtime,
        });
        Self {
            service,
            store,
            matrix,
            agents,
            events,
        }
    }

    fn request(creator: u128, catalog: u128, name: &str) -> CreateSubRoom {
        CreateSubRoom {
            actor: person(creator).into(),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(catalog),
            name: name.to_owned(),
            description: String::new(),
            access: RoomJoinMode::Open,
            hidden: false,
            category_id: None,
            created_by_agent: None,
        }
    }

    async fn create(
        &self,
        creator: u128,
        catalog: u128,
        name: &str,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .create_sub_room(Self::request(creator, catalog, name))
            .await
    }

    async fn list(
        &self,
        viewer: u128,
        catalog: u128,
    ) -> Result<agent_room_application::private_rooms::SubRoomList, PrivateRoomFailure> {
        self.service
            .list_sub_rooms(ListSubRooms {
                actor: person(viewer),
                catalog_id: catalog_id(catalog),
                archived: false,
            })
            .await
    }

    async fn change_policy(
        &self,
        actor: u128,
        policy: SubRoomPolicy,
    ) -> Result<agent_room_application::private_rooms::SubRoomPolicyView, PrivateRoomFailure> {
        self.service
            .change_sub_room_policy(ChangeSubRoomPolicy {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                policy,
            })
            .await
    }

    fn membership(person: u128, catalog: u128) -> SubRoomMembershipAction {
        SubRoomMembershipAction {
            actor: self::person(person),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(catalog),
        }
    }

    async fn join(
        &self,
        person: u128,
        catalog: u128,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .join_sub_room(Self::membership(person, catalog))
            .await
    }

    async fn leave(&self, person: u128, catalog: u128) -> Result<(), PrivateRoomFailure> {
        self.service
            .leave_sub_room(Self::membership(person, catalog))
            .await
    }

    async fn revise(
        &self,
        person: u128,
        catalog: u128,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .revise_sub_room(ReviseSubRoom {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                name: name.map(str::to_owned),
                description: description.map(str::to_owned),
                access: None,
                hidden: None,
                category: None,
            })
            .await
    }

    /// 只换门、勾或者去掉“不在名单里显示”，名字和用途不带。
    async fn change_door(
        &self,
        person: u128,
        catalog: u128,
        access: Option<RoomJoinMode>,
        hidden: Option<bool>,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .revise_sub_room(ReviseSubRoom {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                name: None,
                description: None,
                access,
                hidden,
                category: None,
            })
            .await
    }

    /// 后台的一轮：补对账，再把排着的事领一批出来做。
    async fn sync(&self) -> SubRoomSyncOutcome {
        self.service.sync_sub_rooms().await.expect("这一轮做得完")
    }

    fn clear_events(&self) {
        self.events.lock().expect("事件锁正常").clear();
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("事件锁正常").clone()
    }
}

fn person(value: u128) -> AuthenticatedPrincipal {
    actor(principal_id(value), matrix_user(value).as_str())
}

fn matrix_user(person: u128) -> MatrixUserId {
    let name = match person {
        OWNER => "owner",
        MEMBER => "member",
        ADMIN => "admin",
        VIEWER => "viewer",
        _ => "outsider",
    };
    MatrixUserId::new(format!("@{name}:matrix.test")).expect("用户有效")
}

fn permissions(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).expect("权限有效")
}

fn joined(person: u128, capabilities: &[PrivateRoomCapability]) -> PrivateRoomMember {
    PrivateRoomMember::restore(
        principal_id(person),
        PrivateRoomMembershipStatus::Joined,
        permissions(capabilities),
    )
    .expect("成员有效")
}

fn owner_member() -> PrivateRoomMember {
    PrivateRoomMember::restore(
        principal_id(OWNER),
        PrivateRoomMembershipStatus::Joined,
        PrivateRoomPermissions::ALL,
    )
    .expect("房主有效")
}

fn parent_room() -> PrivateRoomSnapshot {
    use PrivateRoomCapability::{Automate, Manage, Speak, View};
    snapshot(
        PARENT,
        "Project",
        vec![
            owner_member(),
            joined(MEMBER, &[View, Speak, Automate]),
            joined(ADMIN, &[View, Speak, Manage]),
            joined(VIEWER, &[View]),
        ],
        false,
    )
}

/// 一间房的快照，房主是 [`OWNER`]。
fn snapshot(
    catalog: u128,
    name: &str,
    members: Vec<PrivateRoomMember>,
    archived: bool,
) -> PrivateRoomSnapshot {
    let (catalog_status, instance_state, room_status) = if archived {
        (
            RoomCatalogStatus::Archived,
            RoomInstanceState::Archived,
            PrivateRoomLifecycleStatus::Archived,
        )
    } else {
        (
            RoomCatalogStatus::Active,
            RoomInstanceState::Active,
            PrivateRoomLifecycleStatus::Active,
        )
    };
    let catalog_entry = RoomCatalog::new(
        catalog_id(catalog),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: name.to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(principal_id(OWNER)),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: catalog_status,
        },
    )
    .expect("目录有效");
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(uuid(50_000 + catalog)),
        RoomInstanceFields {
            catalog_id: catalog_id(catalog),
            matrix_room_id: MatrixRoomReference::new(format!("!room-{catalog}:matrix.test"))
                .expect("房间有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            allocated_slots: 0,
            state: instance_state,
        },
    )
    .expect("实例有效");
    let room = PrivateRoom::restore(
        catalog_id(catalog),
        principal_id(OWNER),
        room_status,
        members,
        AggregateVersion::INITIAL,
        None,
    )
    .expect("房间有效");
    PrivateRoomSnapshot::new(catalog_entry, instance, room).expect("快照一致")
}

/// 直接放进存储的一间子房间：以前的成员建的，里面只有房主。
struct Seed {
    catalog: u128,
    name: String,
    access: RoomJoinMode,
    hidden: bool,
    archived: bool,
    encrypted: bool,
}

impl Seed {
    fn open(catalog: u128, name: &str) -> Self {
        Self {
            catalog,
            name: name.to_owned(),
            access: RoomJoinMode::Open,
            hidden: false,
            archived: false,
            encrypted: true,
        }
    }
}

#[derive(Clone)]
struct StoredLink {
    link: SubRoomLink,
    managers: Vec<PrincipalId>,
    archived: bool,
    /// 在哪个分类里（2e-1b）。
    category: Option<SubRoomCategoryId>,
}

impl StoredLink {
    /// 归档或者恢复：父子关系上的记录和比重名、数名额用的那一样一起改。
    fn mark_archived(&mut self, record: Option<SubRoomArchiveRecord>) {
        self.archived = record.is_some();
        self.link.archived = record;
    }
}

/// 归档的记录，时间都记在夹具时钟的那一刻。
fn archive_record(reason: SubRoomArchiveReason, by: Option<PrincipalId>) -> SubRoomArchiveRecord {
    SubRoomArchiveRecord {
        at: UtcMillis::new(1_000).expect("时间有效"),
        by,
        reason,
    }
}

/// 排着的一件要在聊天服务器上跟着做的事，和真实存储的 `private_room_sub_room_sync` 一行一样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedSync {
    id: i64,
    catalog_id: RoomCatalogId,
    principal: Option<PrincipalId>,
    generation: i64,
    attempts: u32,
    /// 空着就是马上能领；领走以后是租约到期的时间，没做成以后是下次再试的时间。
    next_attempt_at: Option<UtcMillis>,
    failure_code: Option<&'static str>,
}

impl QueuedSync {
    const fn task(&self) -> SubRoomSyncTask {
        SubRoomSyncTask {
            id: self.id,
            catalog_id: self.catalog_id,
            principal: self.principal,
            generation: self.generation,
            attempt: self.attempts,
        }
    }
}

/// 好几间房和它们的父子关系。重名、上限、限速照真实存储的规则。
pub(super) struct MemoryRooms {
    events: Arc<Mutex<Vec<&'static str>>>,
    rooms: Mutex<BTreeMap<RoomCatalogId, PrivateRoomSnapshot>>,
    links: Mutex<Vec<StoredLink>>,
    unencrypted: Mutex<BTreeSet<RoomCatalogId>>,
    policy: Mutex<Option<SubRoomPolicyRecord>>,
    policy_writes: Mutex<u32>,
    race_same_request: AtomicBool,
    /// 设上的人，在下一次往子房间里记人之前先被移出父房间。
    leaves_parent_before_admission: Mutex<Option<PrincipalId>>,
    /// 聊天服务器上还没跟着做的事，照排进来的先后。
    sync: Mutex<Vec<QueuedSync>>,
    sync_ids: Mutex<i64>,
    /// 设上以后，这间房再被读到第几次之前先改一下存着的东西：摆出“做的当口房间又变了”。
    interrupts_read: Mutex<Option<(RoomCatalogId, u32, Interruption)>>,
    /// 此刻被禁言着的人，按房间记。父房间里的在它的子房间里一起算。
    muted: Mutex<BTreeSet<(RoomCatalogId, PrincipalId)>>,
    /// 设上以后，再问过这么多次“他是不是被禁言着”，这条禁言才落下来：摆出“给发言级别的当口
    /// 禁言落下来了”。
    mutes_later: Mutex<Option<(u32, RoomCatalogId, PrincipalId)>>,
    /// 设上以后，下一次存改动之前这间子房间的门先被换成这样：摆出“存的当口别人换了门”。
    door_moves_before_revision: Mutex<Option<(RoomCatalogId, RoomJoinMode)>>,
    /// 申请进子房间（和公开房间的申请同一张表），照记下的先后。
    requests: Mutex<Vec<JoinRequestRecord>>,
    /// 设上以后，下一次记申请之前那间子房间的门先被换成这样：摆出“申请的当口别人换了门”。
    door_moves_before_request: Mutex<Option<RoomJoinMode>>,
    /// 设、撤子房间管理员时一起记下的身份变动。
    manager_role_changes: Mutex<Vec<RoomRoleChange>>,
    /// 正公开着的房间。
    listed: Mutex<BTreeSet<RoomCatalogId>>,
    /// 设上以后，下一次读完父房间加不加密之后先改一下存着的东西：摆出“建的当口父房间改了”。
    changes_after_parent_facts: Mutex<Option<Interruption>>,
    /// 分类（2e-1），照存的先后；规则在 `sub_room_categories.rs`。
    categories: Mutex<Vec<categories::StoredCategory>>,
}

type Interruption = Box<dyn FnOnce(&MemoryRooms) + Send>;

impl MemoryRooms {
    pub(super) fn new(events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            events,
            rooms: Mutex::new(BTreeMap::new()),
            links: Mutex::new(Vec::new()),
            unencrypted: Mutex::new(BTreeSet::new()),
            policy: Mutex::new(None),
            policy_writes: Mutex::new(0),
            race_same_request: AtomicBool::new(false),
            leaves_parent_before_admission: Mutex::new(None),
            manager_role_changes: Mutex::new(Vec::new()),
            sync: Mutex::new(Vec::new()),
            sync_ids: Mutex::new(0),
            interrupts_read: Mutex::new(None),
            muted: Mutex::new(BTreeSet::new()),
            mutes_later: Mutex::new(None),
            door_moves_before_revision: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
            door_moves_before_request: Mutex::new(None),
            listed: Mutex::new(BTreeSet::new()),
            changes_after_parent_facts: Mutex::new(None),
            categories: Mutex::new(Vec::new()),
        }
    }

    fn set_listed(&self, catalog: u128, listed: bool) {
        let mut rooms = self.listed.lock().expect("公开锁正常");
        if listed {
            rooms.insert(catalog_id(catalog));
        } else {
            rooms.remove(&catalog_id(catalog));
        }
    }

    /// 下一次读完父房间加不加密以后，另一个请求先这样改了它。
    fn change_after_parent_facts(&self, change: impl FnOnce(&Self) + Send + 'static) {
        *self
            .changes_after_parent_facts
            .lock()
            .expect("中途改动锁正常") = Some(Box::new(change));
    }

    /// 下一次存改动之前，另一个请求先把这间子房间的门换成 `access`。
    fn move_door_before_revision(&self, catalog: u128, access: RoomJoinMode) {
        *self.door_moves_before_revision.lock().expect("门锁正常") =
            Some((catalog_id(catalog), access));
    }

    /// 这个人在这间房里被禁言了。
    fn mute(&self, catalog: u128, person: u128) {
        self.muted
            .lock()
            .expect("禁言锁正常")
            .insert((catalog_id(catalog), principal_id(person)));
    }

    /// 他在这间房里的禁言撤了。
    fn unmute(&self, catalog: u128, person: u128) {
        self.muted
            .lock()
            .expect("禁言锁正常")
            .remove(&(catalog_id(catalog), principal_id(person)));
    }

    /// 再问过 `untouched` 次以后，这个人在这间房里被禁言。
    fn mute_after(&self, untouched: u32, catalog: u128, person: u128) {
        *self.mutes_later.lock().expect("禁言锁正常") =
            Some((untouched, catalog_id(catalog), principal_id(person)));
    }

    /// 排上一件要在聊天服务器上跟着做的事。已经排着的换成新的一版、从头再试。
    fn queue_sync(&self, catalog_id: RoomCatalogId, principal: Option<PrincipalId>) {
        let mut sync = self.sync.lock().expect("任务锁正常");
        if let Some(queued) = sync
            .iter_mut()
            .find(|queued| queued.catalog_id == catalog_id && queued.principal == principal)
        {
            queued.generation += 1;
            queued.attempts = 0;
            queued.next_attempt_at = None;
            queued.failure_code = None;
            return;
        }
        let mut ids = self.sync_ids.lock().expect("编号锁正常");
        *ids += 1;
        sync.push(QueuedSync {
            id: *ids,
            catalog_id,
            principal,
            generation: 1,
            attempts: 0,
            next_attempt_at: None,
            failure_code: None,
        });
    }

    fn queued_sync(&self) -> Vec<QueuedSync> {
        self.sync.lock().expect("任务锁正常").clone()
    }

    /// 过了足够久：租约都到期了，退避的也都到点了。
    fn let_sync_come_due(&self) {
        for queued in self.sync.lock().expect("任务锁正常").iter_mut() {
            queued.next_attempt_at = None;
        }
    }

    /// 这间房先照常被读 `untouched` 次，再读之前做 `change`。
    fn interrupt_read(
        &self,
        catalog: u128,
        untouched: u32,
        change: impl FnOnce(&Self) + Send + 'static,
    ) {
        *self.interrupts_read.lock().expect("打断锁正常") =
            Some((catalog_id(catalog), untouched, Box::new(change)));
    }

    /// 真实存储在存父房间的同一个事务里做的那一半：照它重算下面的子房间，聊天服务器上要跟着做的
    /// 排上。
    fn follow_parent(&self, parent: &PrivateRoom) {
        let below: Vec<StoredLink> = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .filter(|stored| stored.link.parent_catalog_id == parent.catalog_id())
            .cloned()
            .collect();
        for stored in below {
            let sub_room = stored.link.catalog_id;
            let before = self.rooms.lock().expect("房间锁正常")[&sub_room]
                .room()
                .clone();
            let managers: BTreeSet<PrincipalId> = stored.managers.iter().copied().collect();
            let Some(after) = follow_parent_room(parent, &before, &managers).expect("重算有效")
            else {
                continue;
            };
            self.replace(&after);
            for member in after.members() {
                if before.member(member.principal_id()) != Some(member) {
                    self.queue_sync(sub_room, Some(member.principal_id()));
                }
            }
            if before.status() == PrivateRoomLifecycleStatus::Active
                && after.status() == PrivateRoomLifecycleStatus::Archived
            {
                for link in self.links.lock().expect("关系锁正常").iter_mut() {
                    if link.link.catalog_id == sub_room {
                        link.mark_archived(Some(archive_record(
                            SubRoomArchiveReason::Parent,
                            None,
                        )));
                    }
                }
                self.queue_sync(sub_room, None);
            }
        }
    }

    fn room(&self, catalog: u128) -> PrivateRoom {
        self.rooms.lock().expect("房间锁正常")[&catalog_id(catalog)]
            .room()
            .clone()
    }

    /// 直接改存着的一间房，绕开用例：摆出封禁、移出这类别的入口留下的样子。
    fn change(&self, catalog: u128, change: impl FnOnce(&mut PrivateRoom)) {
        let mut room = self.room(catalog);
        change(&mut room);
        self.replace(&room);
    }

    /// 换上改过的房间。子房间管理员得是里面已加入的人，不在了的跟着删，和真实存储一样。
    fn replace(&self, room: &PrivateRoom) {
        let mut rooms = self.rooms.lock().expect("房间锁正常");
        let current = rooms.remove(&room.catalog_id()).expect("房间存在");
        rooms.insert(
            room.catalog_id(),
            current.replacing_room(room.clone()).expect("快照一致"),
        );
        for stored in self.links.lock().expect("关系锁正常").iter_mut() {
            if stored.link.catalog_id == room.catalog_id() {
                stored.managers.retain(|manager| {
                    room.member(*manager)
                        .is_some_and(PrivateRoomMember::has_joined)
                });
            }
        }
    }

    fn leave_parent_before_admission(&self, person: u128) {
        *self
            .leaves_parent_before_admission
            .lock()
            .expect("移出锁正常") = Some(principal_id(person));
    }

    fn put(&self, snapshot: PrivateRoomSnapshot) {
        self.rooms
            .lock()
            .expect("房间锁正常")
            .insert(snapshot.catalog().id(), snapshot);
    }

    fn seed(&self, seed: &Seed) {
        let room = snapshot(
            seed.catalog,
            &seed.name,
            vec![owner_member()],
            seed.archived,
        )
        .under(catalog_id(PARENT));
        self.put(room);
        self.links.lock().expect("关系锁正常").push(StoredLink {
            link: SubRoomLink {
                catalog_id: catalog_id(seed.catalog),
                parent_catalog_id: catalog_id(PARENT),
                name_key: seed.name.to_lowercase(),
                access: seed.access,
                hidden: seed.hidden,
                door_opened_at: None,
                end_to_end_encrypted: seed.encrypted,
                created_by: principal_id(EARLIER),
                created_by_agent: None,
                created_at: UtcMillis::new(0).expect("时间有效"),
                archived: seed
                    .archived
                    .then(|| archive_record(SubRoomArchiveReason::Manual, None)),
            },
            managers: Vec::new(),
            archived: seed.archived,
            category: None,
        });
    }

    fn set_encrypted(&self, catalog: u128, encrypted: bool) {
        let mut unencrypted = self.unencrypted.lock().expect("加密锁正常");
        if encrypted {
            unencrypted.remove(&catalog_id(catalog));
        } else {
            unencrypted.insert(catalog_id(catalog));
        }
    }

    /// 归档一间房：父房间归档，或者子房间归档、让出名字和名额。
    fn archive(&self, catalog: u128) {
        let mut rooms = self.rooms.lock().expect("房间锁正常");
        let current = rooms.remove(&catalog_id(catalog)).expect("房间存在");
        let members = current.room().members().cloned().collect();
        let archived = snapshot(catalog, current.catalog().name(), members, true);
        rooms.insert(
            catalog_id(catalog),
            match current.parent_catalog_id() {
                Some(parent) => archived.under(parent),
                None => archived,
            },
        );
        for stored in self.links.lock().expect("关系锁正常").iter_mut() {
            if stored.link.catalog_id == catalog_id(catalog) {
                stored.mark_archived(Some(archive_record(SubRoomArchiveReason::Manual, None)));
            }
        }
    }

    fn link(&self, catalog: u128) -> StoredLink {
        self.links
            .lock()
            .expect("关系锁正常")
            .iter()
            .find(|stored| stored.link.catalog_id == catalog_id(catalog))
            .cloned()
            .expect("父子关系存在")
    }

    fn race_same_request_once(&self) {
        self.race_same_request.store(true, Ordering::SeqCst);
    }

    fn policy_writes(&self) -> u32 {
        *self.policy_writes.lock().expect("设置锁正常")
    }

    fn policy_version(&self) -> i64 {
        self.policy
            .lock()
            .expect("设置锁正常")
            .expect("存过设置")
            .version
            .value()
    }

    fn record(&self, event: &'static str) {
        self.events.lock().expect("事件锁正常").push(event);
    }

    fn block(&self, check: &SubRoomCreationCheck) -> Option<SubRoomCreationBlock> {
        let rooms = self.rooms.lock().expect("房间锁正常");
        let usable = rooms.get(&check.parent_catalog_id).is_some_and(|parent| {
            parent.room().status() == PrivateRoomLifecycleStatus::Active
                && parent.parent_catalog_id().is_none()
        });
        if !usable {
            return Some(SubRoomCreationBlock::ParentUnavailable);
        }
        let links = self.links.lock().expect("关系锁正常");
        let others = || {
            links
                .iter()
                .filter(|stored| stored.link.catalog_id != check.catalog_id)
        };
        let open: Vec<_> = others()
            .filter(|stored| {
                stored.link.parent_catalog_id == check.parent_catalog_id && !stored.archived
            })
            .collect();
        if open
            .iter()
            .any(|stored| stored.link.name_key == check.name_key)
        {
            return Some(SubRoomCreationBlock::NameTaken);
        }
        if open.len() >= usize::from(MAXIMUM_OPEN_SUB_ROOMS) {
            return Some(SubRoomCreationBlock::RoomLimitReached);
        }
        let recent = others()
            .filter(|stored| {
                stored.link.created_by == check.created_by
                    && stored.link.created_by_agent == check.created_by_agent
                    && stored.link.created_at.value() > check.at.value() - 3_600_000
            })
            .count();
        if recent >= usize::from(MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR) {
            return Some(SubRoomCreationBlock::CreatorLimitReached);
        }
        check
            .category_id
            .filter(|category| !self.has_category(check.parent_catalog_id, *category))
            .map(|_| SubRoomCreationBlock::CategoryMissing)
    }

    fn insert(&self, sub_room: &NewSubRoom) {
        self.put(sub_room.snapshot.clone());
        self.links.lock().expect("关系锁正常").push(StoredLink {
            link: SubRoomLink {
                catalog_id: sub_room.snapshot.catalog().id(),
                parent_catalog_id: sub_room.parent_catalog_id,
                name_key: sub_room.name_key.clone(),
                access: sub_room.access,
                hidden: sub_room.hidden,
                door_opened_at: None,
                end_to_end_encrypted: sub_room.end_to_end_encrypted,
                created_by: sub_room.created_by,
                created_by_agent: sub_room.created_by_agent,
                created_at: sub_room.created_at,
                archived: None,
            },
            managers: sub_room.managers.clone(),
            archived: false,
            category: sub_room.category_id,
        });
    }
}

impl StandingModerationSource for MemoryRooms {
    fn standing_person_actions(
        &self,
        _room_catalog_id: RoomCatalogId,
        _now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<StandingModeration>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn person_muted(
        &self,
        room_catalog_id: RoomCatalogId,
        principal_id: PrincipalId,
        _now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let mut later = self.mutes_later.lock().expect("禁言锁正常");
        match later.take() {
            Some((0, catalog, person)) => {
                self.muted
                    .lock()
                    .expect("禁言锁正常")
                    .insert((catalog, person));
            }
            Some((untouched, catalog, person)) => *later = Some((untouched - 1, catalog, person)),
            None => {}
        }
        drop(later);
        let parent = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .find(|stored| stored.link.catalog_id == room_catalog_id)
            .map(|stored| stored.link.parent_catalog_id);
        let muted = self.muted.lock().expect("禁言锁正常");
        let held = muted.contains(&(room_catalog_id, principal_id))
            || parent.is_some_and(|parent| muted.contains(&(parent, principal_id)));
        Box::pin(async move { Ok(held) })
    }
}

impl SubRoomStore for MemoryRooms {
    fn sub_room_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomLink>>> {
        let link = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .find(|stored| stored.link.catalog_id == catalog_id)
            .map(|stored| stored.link.clone());
        Box::pin(async move { Ok(link) })
    }

    fn sub_room_parent_facts(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomParentFacts>>> {
        let exists = self.rooms.lock().expect("房间锁正常").contains_key(&parent);
        let facts = exists.then(|| SubRoomParentFacts {
            end_to_end_encrypted: !self
                .unencrypted
                .lock()
                .expect("加密锁正常")
                .contains(&parent),
            policy: self
                .policy
                .lock()
                .expect("设置锁正常")
                .map_or(SubRoomPolicy::DEFAULT, |record| record.policy),
        });
        let change = self
            .changes_after_parent_facts
            .lock()
            .expect("中途改动锁正常")
            .take();
        if let Some(change) = change {
            change(self);
        }
        Box::pin(async move { Ok(facts) })
    }

    fn sub_room_creation_block<'a>(
        &'a self,
        check: &'a SubRoomCreationCheck,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>> {
        Box::pin(async move {
            self.record("sub_rooms.block");
            Ok(self.block(check))
        })
    }

    fn create_sub_room<'a>(
        &'a self,
        sub_room: &'a NewSubRoom,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>> {
        Box::pin(async move {
            self.record("sub_rooms.create");
            if self.race_same_request.swap(false, Ordering::SeqCst) {
                self.insert(sub_room);
            }
            if let Some(block) = self.block(&sub_room.check()) {
                return Ok(Some(block));
            }
            let taken = self
                .rooms
                .lock()
                .expect("房间锁正常")
                .contains_key(&sub_room.snapshot.catalog().id());
            if taken {
                return Err(repository_error(
                    "test.sub_room.create",
                    RepositoryErrorKind::Conflict,
                ));
            }
            self.insert(sub_room);
            Ok(None)
        })
    }

    fn list_sub_rooms(
        &self,
        parent: RoomCatalogId,
        viewer: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomEntry>>> {
        let rooms = self.rooms.lock().expect("房间锁正常");
        let entries = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .filter(|stored| stored.link.parent_catalog_id == parent)
            .map(|stored| {
                let snapshot = &rooms[&stored.link.catalog_id];
                let room = snapshot.room();
                SubRoomEntry {
                    catalog_id: stored.link.catalog_id,
                    matrix_room: snapshot.instance().matrix_room_id().clone(),
                    name: snapshot.catalog().name().to_owned(),
                    description: snapshot.catalog().description().to_owned(),
                    access: stored.link.access,
                    hidden: stored.link.hidden,
                    category_id: stored.category,
                    archived: room.status() == PrivateRoomLifecycleStatus::Archived,
                    archive_record: stored.link.archived,
                    created_by: stored.link.created_by,
                    created_at: stored.link.created_at,
                    managers: stored.managers.clone(),
                    viewer_status: room.member(viewer).map(PrivateRoomMember::status),
                    member_count: u32::try_from(
                        room.members().filter(|member| member.has_joined()).count(),
                    )
                    .expect("人数不多"),
                    agent_count: 0,
                    last_message_at: None,
                    viewer_requested_at: self
                        .pending_request(stored.link.catalog_id, viewer)
                        .map(|request| request.requested_at),
                    pending_requests: self.pending_count(stored.link.catalog_id),
                }
            })
            .collect();
        Box::pin(async move { Ok(entries) })
    }

    fn admit_to_sub_room<'a>(
        &'a self,
        admission: &'a SubRoomAdmission<'a>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            self.record("sub_rooms.admit");
            let leaver = self
                .leaves_parent_before_admission
                .lock()
                .expect("移出锁正常")
                .take();
            if let Some(leaver) = leaver {
                self.change(PARENT, |parent| {
                    parent
                        .remove_member(principal_id(OWNER), leaver)
                        .expect("房主能移出");
                });
            }
            let in_parent = self.rooms.lock().expect("房间锁正常")[&admission.parent_catalog_id]
                .room()
                .member(admission.principal)
                .is_some_and(PrivateRoomMember::has_joined);
            if !in_parent {
                return Ok(false);
            }
            self.replace(admission.room);
            Ok(true)
        })
    }

    fn change_sub_room_manager<'a>(
        &'a self,
        change: &'a SubRoomManagerChange<'a>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            self.record("sub_rooms.manager");
            let leaver = self
                .leaves_parent_before_admission
                .lock()
                .expect("移出锁正常")
                .take();
            if let Some(leaver) = leaver {
                self.change(PARENT, |parent| {
                    parent
                        .remove_member(principal_id(OWNER), leaver)
                        .expect("房主能移出");
                });
            }
            let in_parent = self.rooms.lock().expect("房间锁正常")[&change.parent_catalog_id]
                .room()
                .member(change.principal)
                .is_some_and(PrivateRoomMember::has_joined);
            if !in_parent {
                return Ok(false);
            }
            self.replace(change.room);
            for stored in self.links.lock().expect("关系锁正常").iter_mut() {
                if stored.link.catalog_id == change.room.catalog_id() {
                    stored
                        .managers
                        .retain(|manager| *manager != change.principal);
                    if change.grant {
                        stored.managers.push(change.principal);
                    }
                }
            }
            self.manager_role_changes
                .lock()
                .expect("身份变动锁正常")
                .extend_from_slice(change.role_changes);
            Ok(true)
        })
    }

    fn revise_sub_room<'a>(
        &'a self,
        revision: &'a SubRoomRevision,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomRevisionBlock>>> {
        Box::pin(async move {
            self.record("sub_rooms.revise");
            let mut rooms = self.rooms.lock().expect("房间锁正常");
            let mut links = self.links.lock().expect("关系锁正常");
            if let Some((moved, access)) = self
                .door_moves_before_revision
                .lock()
                .expect("门锁正常")
                .take()
            {
                for stored in links.iter_mut() {
                    if stored.link.catalog_id == moved {
                        stored.link.access = access;
                    }
                }
            }
            let siblings = |stored: &&StoredLink| {
                stored.link.parent_catalog_id == revision.parent_catalog_id && !stored.archived
            };
            if !links
                .iter()
                .filter(siblings)
                .any(|stored| stored.link.catalog_id == revision.catalog_id)
            {
                return Ok(Some(SubRoomRevisionBlock::Unavailable));
            }
            let door_moved = revision.door.is_some_and(|door| {
                links.iter().any(|stored| {
                    stored.link.catalog_id == revision.catalog_id
                        && (stored.link.access != door.from.access()
                            || stored.link.hidden != door.from.hidden())
                })
            });
            if door_moved {
                return Ok(Some(SubRoomRevisionBlock::DoorChanged));
            }
            if links.iter().filter(siblings).any(|stored| {
                stored.link.catalog_id != revision.catalog_id
                    && stored.link.name_key == revision.name_key
            }) {
                return Ok(Some(SubRoomRevisionBlock::NameTaken));
            }
            if let Some(Some(category)) = revision.category
                && !self.has_category(revision.parent_catalog_id, category)
            {
                return Ok(Some(SubRoomRevisionBlock::CategoryMissing));
            }
            for stored in links.iter_mut() {
                if stored.link.catalog_id == revision.catalog_id {
                    if let Some(category) = revision.category {
                        stored.category = category;
                    }
                    let link = &mut stored.link;
                    link.name_key.clone_from(&revision.name_key);
                    let Some(door) = revision.door else {
                        continue;
                    };
                    // 从有门改成没有门的记下拆门的时间，只记第一次。
                    let opens = link.access != RoomJoinMode::Open && !door.to.has_door();
                    if opens && link.door_opened_at.is_none() {
                        link.door_opened_at = Some(revision.at);
                    }
                    link.access = door.to.access();
                    link.hidden = door.to.hidden();
                    // 换完不收申请的，在等的一起作废，和真实存储一样在这一次里做。
                    if !door.to.takes_requests() {
                        self.void_requests(revision.catalog_id, revision.at);
                    }
                }
            }
            let current = rooms.remove(&revision.catalog_id).expect("房间存在");
            let revised = current
                .renaming(revision.name.clone())
                .and_then(|renamed| renamed.describing(revision.description.clone()))
                .expect("名字和用途有效");
            rooms.insert(revision.catalog_id, revised);
            Ok(None)
        })
    }

    fn request_sub_room_join<'a>(
        &'a self,
        parent_catalog_id: RoomCatalogId,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<SubRoomJoinRequestOutcome>> {
        Box::pin(async move { Ok(self.store_join_request(parent_catalog_id, record)) })
    }

    fn sub_room_policy(
        &self,
        _parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomPolicyRecord>>> {
        let record = *self.policy.lock().expect("设置锁正常");
        Box::pin(async move { Ok(record) })
    }

    fn save_sub_room_policy<'a>(
        &'a self,
        record: &'a SubRoomPolicyRecord,
        expected: Option<AggregateVersion>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let mut stored = self.policy.lock().expect("设置锁正常");
            if stored.map(|current| current.version) != expected {
                return Ok(false);
            }
            *stored = Some(*record);
            *self.policy_writes.lock().expect("设置锁正常") += 1;
            Ok(true)
        })
    }

    fn follow_stale_sub_rooms(
        &self,
        _limit: u16,
        _at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u32>> {
        Box::pin(async {
            self.record("sub_rooms.follow_stale");
            Ok(0)
        })
    }

    fn claim_sub_room_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomSyncTask>>> {
        let below: Option<BTreeSet<RoomCatalogId>> = match scope {
            SubRoomSyncScope::Everywhere => None,
            SubRoomSyncScope::Under(parent) => Some(
                self.links
                    .lock()
                    .expect("关系锁正常")
                    .iter()
                    .filter(|stored| stored.link.parent_catalog_id == parent)
                    .map(|stored| stored.link.catalog_id)
                    .collect(),
            ),
        };
        let mut sync = self.sync.lock().expect("任务锁正常");
        let mut due: Vec<&mut QueuedSync> = sync
            .iter_mut()
            .filter(|queued| queued.next_attempt_at.is_none_or(|at| at <= now))
            .filter(|queued| {
                below
                    .as_ref()
                    .is_none_or(|below| below.contains(&queued.catalog_id))
            })
            .collect();
        // 没领过的在前，再照排进来的先后。
        due.sort_by_key(|queued| (queued.next_attempt_at, queued.id));
        let mut tasks: Vec<SubRoomSyncTask> = due
            .into_iter()
            .take(usize::from(limit))
            .map(|queued| {
                queued.attempts += 1;
                queued.next_attempt_at = Some(lease_until);
                queued.task()
            })
            .collect();
        tasks.sort_by_key(|task| task.id);
        Box::pin(async move { Ok(tasks) })
    }

    fn complete_sub_room_sync(
        &self,
        task: SubRoomSyncTask,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut sync = self.sync.lock().expect("任务锁正常");
        let before = sync.len();
        sync.retain(|queued| !(queued.id == task.id && queued.generation == task.generation));
        let done = sync.len() < before;
        drop(sync);
        if !done {
            self.queue_sync(task.catalog_id, task.principal);
        }
        Box::pin(async { Ok(()) })
    }

    fn retry_sub_room_sync(
        &self,
        task: SubRoomSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut sync = self.sync.lock().expect("任务锁正常");
        let postponed = sync
            .iter_mut()
            .find(|queued| queued.id == task.id && queued.generation == task.generation)
            .map(|queued| {
                queued.next_attempt_at = Some(retry_at);
                queued.failure_code = Some(failure_code);
            })
            .is_some();
        drop(sync);
        if !postponed {
            self.queue_sync(task.catalog_id, task.principal);
        }
        Box::pin(async { Ok(()) })
    }
}

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.put(snapshot.clone());
            Ok(())
        })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let interruption = {
            let mut pending = self.interrupts_read.lock().expect("打断锁正常");
            let due = match pending.as_mut() {
                Some((room, untouched, _)) if *room == catalog_id => {
                    let due = *untouched == 0;
                    *untouched = untouched.saturating_sub(1);
                    due
                }
                _ => false,
            };
            if due { pending.take() } else { None }
        };
        if let Some((_, _, change)) = interruption {
            change(self);
        }
        let found = self
            .rooms
            .lock()
            .expect("房间锁正常")
            .get(&catalog_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    /// 和真实存储一样不列子房间。
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        let listed = self
            .rooms
            .lock()
            .expect("房间锁正常")
            .values()
            .filter(|snapshot| {
                snapshot.parent_catalog_id().is_none()
                    && snapshot
                        .room()
                        .member(principal_id)
                        .is_some_and(PrivateRoomMember::has_joined)
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(listed) })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = self
            .rooms
            .lock()
            .expect("房间锁正常")
            .values()
            .find(|snapshot| snapshot.instance().matrix_room_id() == matrix_room_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn save<'a>(
        &'a self,
        room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.record("store.save");
            self.replace(room);
            self.void_banned_requests(room);
            self.follow_parent(room);
            Ok(())
        })
    }

    fn save_recording<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected_version: AggregateVersion,
        changed_at: UtcMillis,
        _role_changes: &'a [RoomRoleChange],
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.save(room, expected_version, changed_at)
    }

    fn rename<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.record("store.rename");
            let mut rooms = self.rooms.lock().expect("房间锁正常");
            let current = rooms.remove(&catalog_id).expect("房间存在");
            rooms.insert(
                catalog_id,
                current.renaming(name.to_owned()).expect("名字有效"),
            );
            Ok(())
        })
    }
}
