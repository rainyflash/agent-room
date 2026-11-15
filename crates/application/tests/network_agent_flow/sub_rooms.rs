//! 网络 Agent 在它建的房间下面建子房间（`specs/redesign/sub-rooms.md` 第 2h 步）。

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentInvitation, NetworkAgentOwnedRoom, NetworkAgentOwnedSubRoom,
    NetworkAgentRoomCode, NetworkAgentRoomUseCases, NetworkAgentSubRoomActivity,
    NetworkAgentSubRoomList, NetworkAgentSubRoomRequest,
};
use agent_room_application::{
    ports::{
        JoinRequestRecord, NetworkAgentRoomParent, NetworkAgentRoomRecord, PendingJoinRequest,
        SubRoomEntry,
    },
    private_rooms::{AgentSubRoomItem, AgentSubRooms, ListAgentSubRooms},
};
use agent_room_domain::room_join_requests::{JoinRequestMessage, JoinRequestStatus};

use super::*;

async fn agent(harness: &Harness) -> String {
    let created = harness.create("Scout", None).await.expect("起名成功");
    created.token.expose().to_owned()
}

async fn parent(harness: &Harness, token: &str, name: &str) -> NetworkAgentOwnedRoom {
    harness
        .service
        .create_room(token, name)
        .await
        .expect("建房成功")
}

async fn sub_room(
    harness: &Harness,
    token: &str,
    parent: &str,
    name: &str,
    category: Option<&str>,
) -> Result<NetworkAgentOwnedSubRoom, NetworkAgentFailure> {
    harness
        .service
        .create_sub_room(
            token,
            NetworkAgentSubRoomRequest {
                parent,
                name,
                access: RoomJoinMode::Request,
                category,
            },
        )
        .await
}

fn kind<T>(result: Result<T, NetworkAgentFailure>) -> NetworkAgentFailureKind {
    result.err().expect("应当被拒").kind()
}

#[tokio::test]
async fn 在它建的房间下面建子房间_交给私人房间用例_分类按名字给_还没进去() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let room = parent(&harness, &token, "项目").await;
    let number = room.room.catalog_id.to_string();

    let owned = sub_room(&harness, &token, &number, " 设计 ", Some("进行中"))
        .await
        .unwrap();

    assert!(owned.created);
    assert!(!owned.joined, "由网关让加密客户端就绪后再进");
    assert_eq!(owned.room.name, "设计", "名字去掉首尾空白");
    assert_eq!(owned.parent, room.room);
    assert_eq!(owned.category.as_deref(), Some("进行中"));
    assert!(!owned.category_missing);
    let created = harness.rooms.sub_rooms_created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].actor.principal_id(), record.principal_id);
    assert_eq!(created[0].parent_catalog_id, room.room.catalog_id);
    assert_eq!(created[0].catalog_id, owned.room.catalog_id);
    assert_eq!(created[0].access, RoomJoinMode::Request);
    assert_eq!(created[0].category.as_deref(), Some("进行中"));
    assert_eq!(Some(created[0].created_by_agent), record.agent_id);

    let missing = sub_room(&harness, &token, &number, "测试", Some("别的"))
        .await
        .unwrap();
    assert!(missing.category_missing, "没有这个分类：放进未分类，照实说");
    assert_eq!(missing.category, None);
}

#[tokio::test]
async fn 同名的没归档子房间回到那间_不分大小写_子房间不算它的整间房() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let room = parent(&harness, &token, "项目").await;
    let number = room.room.catalog_id.to_string();
    let first = sub_room(&harness, &token, &number, "Design", None)
        .await
        .unwrap();

    let again = sub_room(&harness, &token, &number, "design", Some("进行中"))
        .await
        .unwrap();

    assert!(!again.created);
    assert_eq!(again.room, first.room);
    assert_eq!(again.category, None, "回到已有的那间不说分类");
    assert_eq!(harness.rooms.sub_rooms_created.lock().unwrap().len(), 1);

    let whole = parent(&harness, &token, "Design").await;
    assert!(whole.created, "同名的子房间不算它没关的整间房，整间房照建");
    assert_ne!(whole.room.catalog_id, first.room.catalog_id);
}

/// 它凭口令或敲门进了的房间：记在它进过的房间里。
async fn record_entered(harness: &Harness, catalog_id: RoomCatalogId, archived: bool) {
    let id = harness.store.only().id;
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id,
                matrix_room_id: owned_matrix_room(catalog_id),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
                archived,
                parent: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn 凭口令进了别人的房间_在它下面建子房间交给私人房间用例_同名的回到它自己建的那间() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let other = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    record_entered(&harness, other, false).await;

    let owned = sub_room(&harness, &token, &other.to_string(), "设计", Some("进行中"))
        .await
        .unwrap();

    assert!(owned.created);
    assert!(!owned.joined, "由网关让加密客户端就绪后再进");
    assert_eq!(owned.parent.catalog_id, other);
    assert_eq!(owned.parent.matrix_room_id, owned_matrix_room(other));
    assert_eq!(owned.parent.name, "别人的房间");
    assert_eq!(owned.category.as_deref(), Some("进行中"));
    let created = harness.rooms.sub_rooms_created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].actor.principal_id(), record.principal_id);
    assert_eq!(created[0].parent_catalog_id, other);

    let again = sub_room(
        &harness,
        &token,
        owned_matrix_room(other).as_str(),
        "设计",
        None,
    )
    .await
    .unwrap();
    assert!(!again.created, "按 Matrix 房间 ID 认也行，回到它建的那间");
    assert!(!again.joined, "还没进去");
    assert_eq!(again.room, owned.room);

    record_entered(&harness, owned.room.catalog_id, false).await;
    let entered = sub_room(&harness, &token, &other.to_string(), "设计", None)
        .await
        .unwrap();
    assert!(entered.joined, "进去了的照实说");
}

#[tokio::test]
async fn 进了的房间关了_或者没进过_不去建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let other = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));

    assert_eq!(
        kind(sub_room(&harness, &token, &other.to_string(), "设计", None).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable,
        "没进过"
    );
    record_entered(&harness, other, true).await;
    assert_eq!(
        kind(sub_room(&harness, &token, &other.to_string(), "设计", None).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable,
        "关了"
    );
    assert!(harness.rooms.sub_rooms_created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 父房间不是它建的_关了的_或者本身是子房间_什么都不建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let other = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    let room = parent(&harness, &token, "项目").await;
    let number = room.room.catalog_id.to_string();
    let inner = sub_room(&harness, &token, &number, "设计", None)
        .await
        .unwrap();
    harness.rooms.sub_rooms_created.lock().unwrap().clear();

    assert_eq!(
        kind(sub_room(&harness, &token, &other.to_string(), "设计", None).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable
    );
    assert_eq!(
        kind(sub_room(&harness, &token, "!nowhere:matrix.test", "设计", None).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable,
        "不是房间号、也不是它进过的房间"
    );
    assert_eq!(
        kind(
            sub_room(
                &harness,
                &token,
                &inner.room.catalog_id.to_string(),
                "再一层",
                None
            )
            .await
        ),
        NetworkAgentFailureKind::SubRoomNested
    );
    harness
        .service
        .close_room(&token, &number)
        .await
        .expect("关房成功");
    assert_eq!(
        kind(sub_room(&harness, &token, &number, "设计", None).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable
    );
    assert!(harness.rooms.sub_rooms_created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 名字和分类不对时不去建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let number = parent(&harness, &token, "项目")
        .await
        .room
        .catalog_id
        .to_string();

    assert_eq!(
        kind(sub_room(&harness, &token, &number, "  ", None).await),
        NetworkAgentFailureKind::RoomNameInvalid
    );
    for category in ["", " 进行中", "两\n行", &"长".repeat(65)] {
        assert_eq!(
            kind(sub_room(&harness, &token, &number, "设计", Some(category)).await),
            NetworkAgentFailureKind::SubRoomCategoryInvalid,
            "{category:?}"
        );
    }
    assert!(harness.rooms.sub_rooms_created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 私人房间用例拒绝时照子房间的规矩说清楚() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let number = parent(&harness, &token, "项目")
        .await
        .room
        .catalog_id
        .to_string();
    let retry_at = UtcMillis::new(START + 60_000).unwrap();
    let cases = [
        (
            room_failure(PrivateRoomFailureKind::InvalidRequest),
            NetworkAgentFailureKind::RoomNameInvalid,
        ),
        (
            sub_room_refused(SubRoomRefusal::AdminsOnly),
            NetworkAgentFailureKind::SubRoomAdminsOnly,
        ),
        (
            sub_room_refused(SubRoomRefusal::AgentsMayNotCreate),
            NetworkAgentFailureKind::SubRoomAgentsMayNotCreate,
        ),
        (
            sub_room_refused(SubRoomRefusal::NameTaken),
            NetworkAgentFailureKind::SubRoomNameTaken,
        ),
        (
            sub_room_refused(SubRoomRefusal::LimitReached),
            NetworkAgentFailureKind::SubRoomLimitReached,
        ),
        (
            sub_room_refused(SubRoomRefusal::Archived),
            NetworkAgentFailureKind::SubRoomParentUnavailable,
        ),
        (
            room_failure(PrivateRoomFailureKind::NotFound),
            NetworkAgentFailureKind::SubRoomParentUnavailable,
        ),
        (
            room_failure(PrivateRoomFailureKind::Forbidden),
            NetworkAgentFailureKind::SubRoomParentUnavailable,
        ),
        (
            room_failure(PrivateRoomFailureKind::Conflict),
            NetworkAgentFailureKind::DependencyUnavailable,
        ),
        (
            room_failure(PrivateRoomFailureKind::DependencyUnavailable),
            NetworkAgentFailureKind::DependencyUnavailable,
        ),
    ];
    for (refusal, expected) in cases {
        *harness.rooms.sub_room_refusal.lock().unwrap() = Some(refusal);
        assert_eq!(
            kind(sub_room(&harness, &token, &number, "设计", None).await),
            expected
        );
    }

    *harness.rooms.sub_room_refusal.lock().unwrap() =
        Some(sub_room_refused(SubRoomRefusal::RateLimited).retrying_at(retry_at));
    let limited = sub_room(&harness, &token, &number, "设计", None)
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(limited.retry_at(), Some(retry_at));
}

#[tokio::test]
async fn 令牌不对时不建() {
    let harness = Harness::enabled();
    assert_eq!(
        kind(sub_room(&harness, "nope", "!room:matrix.test", "设计", None).await),
        NetworkAgentFailureKind::Unauthorized
    );
    assert!(harness.rooms.sub_rooms_created.lock().unwrap().is_empty());
}

// ---- 看子房间名单（2h-2）

fn entry(
    catalog: RoomCatalogId,
    name: &str,
    access: RoomJoinMode,
    created_by: PrincipalId,
    category: Option<&str>,
) -> AgentSubRoomItem {
    AgentSubRoomItem {
        entry: SubRoomEntry {
            catalog_id: catalog,
            matrix_room: owned_matrix_room(catalog),
            name: name.to_owned(),
            description: format!("{name}的用途"),
            access,
            hidden: false,
            category_id: None,
            archived: false,
            archive_record: None,
            created_by,
            created_at: UtcMillis::new(START).unwrap(),
            managers: Vec::new(),
            viewer_status: None,
            member_count: 3,
            agent_count: 2,
            last_message_at: Some(UtcMillis::new(START + 5).unwrap()),
            viewer_requested_at: None,
            pending_requests: 0,
        },
        category: category.map(str::to_owned),
    }
}

async fn list(
    harness: &Harness,
    token: &str,
    parent: &str,
    archived: bool,
) -> Result<NetworkAgentSubRoomList, NetworkAgentFailure> {
    harness
        .service
        .list_sub_rooms(token, parent, archived)
        .await
}

#[tokio::test]
async fn 看子房间名单_交给私人房间用例_按它进过的房间说在不在里面_有门又不在里面的不给人数() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let someone = PrincipalId::from_uuid(Uuid::now_v7());
    let other = harness.rooms.seed_other(someone);
    record_entered(&harness, other, false).await;
    let parent = harness
        .rooms
        .rooms
        .lock()
        .unwrap()
        .iter()
        .find(|snapshot| snapshot.catalog().id() == other)
        .unwrap()
        .clone();
    let (open, door, its) = (catalog_id(201), catalog_id(202), catalog_id(203));
    *harness.rooms.sub_room_list.lock().unwrap() = Some(Ok(AgentSubRooms {
        parent,
        categories: vec!["进行中".to_owned()],
        rooms: vec![
            entry(open, "大家的", RoomJoinMode::Open, someone, Some("进行中")),
            entry(door, "有门的", RoomJoinMode::Request, someone, None),
            entry(
                its,
                "它的",
                RoomJoinMode::InviteOnly,
                record.principal_id,
                None,
            ),
        ],
    }));
    record_entered(&harness, its, false).await;

    let listed = list(&harness, &token, &other.to_string(), true)
        .await
        .unwrap();

    assert_eq!(
        harness.rooms.sub_rooms_listed.lock().unwrap().clone(),
        [ListAgentSubRooms {
            viewer: record.principal_id,
            agent_id: record.agent_id.unwrap(),
            catalog_id: other,
            archived: true,
        }]
    );
    assert_eq!(listed.parent.catalog_id, other);
    assert_eq!(listed.parent.name, "别人的房间");
    assert_eq!(listed.categories, ["进行中"]);
    let [everyone, behind_door, own] = listed.rooms.as_slice() else {
        panic!("三间都在：{:?}", listed.rooms);
    };
    assert_eq!(everyone.room.matrix_room_id, owned_matrix_room(open));
    assert_eq!(everyone.description, "大家的的用途");
    assert_eq!(everyone.category.as_deref(), Some("进行中"));
    assert!(!everyone.joined && !everyone.created_by_you);
    assert_eq!(
        everyone.activity,
        Some(NetworkAgentSubRoomActivity {
            people: 3,
            agents: 2,
            last_message_at: Some(UtcMillis::new(START + 5).unwrap()),
        }),
        "没有门的给人数"
    );
    assert!(!behind_door.joined);
    assert_eq!(behind_door.activity, None, "有门又不在里面：不给人数");
    assert!(own.joined && own.created_by_you);
    assert!(own.activity.is_some(), "在里面的给人数");
}

#[tokio::test]
async fn 看不了名单时说父房间用不了_别的照管房间的说() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let other = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()))
        .to_string();

    assert_eq!(
        kind(list(&harness, &token, "!nowhere:matrix.test", false).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable,
        "不是房间号、也不是它进过的房间"
    );
    assert_eq!(
        kind(list(&harness, &token, &other, false).await),
        NetworkAgentFailureKind::SubRoomParentUnavailable,
        "私人房间用例说找不到"
    );
    *harness.rooms.sub_room_list.lock().unwrap() = Some(Err(room_failure(
        PrivateRoomFailureKind::DependencyUnavailable,
    )));
    assert_eq!(
        kind(list(&harness, &token, &other, false).await),
        NetworkAgentFailureKind::DependencyUnavailable
    );
    assert_eq!(
        kind(list(&harness, "nope", &other, false).await),
        NetworkAgentFailureKind::Unauthorized
    );
}

#[tokio::test]
async fn 查看自己时_进了的子房间带着上面那一间() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let id = harness.store.only().id;
    let (above, below) = (catalog_id(301), catalog_id(302));
    let parent = NetworkAgentRoomParent {
        catalog_id: above,
        name: "项目".to_owned(),
    };
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id: below,
                matrix_room_id: owned_matrix_room(below),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
                archived: false,
                parent: Some(parent.clone()),
            },
        )
        .await
        .unwrap();

    let me = harness.service.me(&token).await.unwrap();

    let joined = me
        .rooms
        .iter()
        .find(|joined| joined.room.catalog_id == below)
        .expect("进了的子房间在列");
    assert_eq!(joined.parent, Some(parent));
    assert!(
        me.rooms
            .iter()
            .filter(|joined| joined.room.catalog_id != below)
            .all(|joined| joined.parent.is_none()),
        "不是子房间的不带"
    );
}

#[tokio::test]
async fn 关子房间交给私人房间用例归档_不再照整间房关_管不了的说不是它的() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let room = parent(&harness, &token, "项目").await;
    let below = sub_room(
        &harness,
        &token,
        &room.room.catalog_id.to_string(),
        "设计",
        None,
    )
    .await
    .unwrap();
    let snapshot = harness
        .rooms
        .rooms
        .lock()
        .unwrap()
        .iter()
        .find(|snapshot| snapshot.catalog().id() == room.room.catalog_id)
        .unwrap()
        .clone();
    *harness.rooms.sub_room_archive.lock().unwrap() = Some(Ok(Some(snapshot)));

    harness
        .service
        .close_room(&token, &below.room.catalog_id.to_string())
        .await
        .expect("归档了");

    let requests = harness.rooms.sub_rooms_archived.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].actor.principal_id(), record.principal_id);
    assert_eq!(Some(requests[0].agent_id), record.agent_id);
    assert_eq!(requests[0].catalog_id, below.room.catalog_id);
    assert!(
        harness.rooms.archived.lock().unwrap().is_empty(),
        "子房间不照整间房关"
    );

    for (failure, expected) in [
        (
            sub_room_refused(SubRoomRefusal::ManagersOnly),
            NetworkAgentFailureKind::RoomNotOwned,
        ),
        (
            room_failure(PrivateRoomFailureKind::NotFound),
            NetworkAgentFailureKind::RoomNotOwned,
        ),
        (
            room_failure(PrivateRoomFailureKind::DependencyUnavailable),
            NetworkAgentFailureKind::DependencyUnavailable,
        ),
    ] {
        *harness.rooms.sub_room_archive.lock().unwrap() = Some(Err(failure));
        assert_eq!(
            kind(
                harness
                    .service
                    .close_room(&token, &below.room.catalog_id.to_string())
                    .await
            ),
            expected
        );
    }
    assert!(harness.rooms.archived.lock().unwrap().is_empty());
}

// ---- 管它管得了的子房间（2h-4a）

/// 它在这间子房间里做过的管房间的事（看敲门、放行、移出……）。
fn managed_actions(harness: &Harness, catalog_id: RoomCatalogId) -> Vec<&'static str> {
    harness
        .access
        .managed
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, room, _)| *room == catalog_id)
        .map(|(_, _, action)| *action)
        .collect()
}

#[tokio::test]
async fn 它自己房间下面的子房间_看口令说是子房间_换口令说没有口令_看敲门放行移出照做() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let room = parent(&harness, &token, "项目").await;
    let below = sub_room(
        &harness,
        &token,
        &room.room.catalog_id.to_string(),
        "设计",
        None,
    )
    .await
    .unwrap()
    .room;
    let number = below.catalog_id.to_string();

    let view = harness.service.room_code(&token, &number).await.unwrap();
    assert_eq!(view.room, below);
    assert!(matches!(view.code, NetworkAgentRoomCode::SubRoom));
    for failure in [
        kind(harness.service.replace_code(&token, &number).await),
        kind(harness.service.disable_code(&token, &number).await),
    ] {
        assert_eq!(failure, NetworkAgentFailureKind::SubRoomHasNoCode);
    }
    assert!(
        managed_actions(&harness, below.catalog_id).is_empty(),
        "口令碰都不碰"
    );

    let knocker = AgentId::from_uuid(Uuid::now_v7());
    harness.access.knocks.lock().unwrap().push(waiting_knock_on(
        below.catalog_id,
        knocker,
        "Critic",
    ));
    let waiting = harness
        .service
        .waiting_knocks(&token, &number)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].display_name, "Critic");
    let incoming = harness.service.incoming_knocks(&token).await.unwrap();
    assert!(incoming.owns_rooms);
    assert_eq!(incoming.knocks.len(), 1, "子房间的敲门也叫醒它");
    assert_eq!(incoming.knocks[0].room, below);

    harness
        .service
        .admit_knock(&token, &number, knocker)
        .await
        .expect("放行");
    harness.access.members.lock().unwrap().insert(knocker);
    harness
        .service
        .remove_agent(&token, &number, knocker)
        .await
        .expect("移出");
    assert_eq!(
        managed_actions(&harness, below.catalog_id),
        ["knocks", "knocks", "admit", "remove"]
    );
    let asked = harness.rooms.sub_rooms_managed.lock().unwrap().clone();
    assert!(asked.iter().all(|request| {
        request.catalog_id == below.catalog_id
            && Some(request.agent_id) == record.agent_id
            && request.actor.principal_id() == record.principal_id
    }));
}

#[tokio::test]
async fn 它在别人房间下面建的子房间_看敲门_敲门也叫醒它_管不了的说不是它的() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let id = harness.store.only().id;
    let other = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    record_entered(&harness, other, false).await;
    let below = sub_room(&harness, &token, &other.to_string(), "设计", None)
        .await
        .unwrap()
        .room;
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id: below.catalog_id,
                matrix_room_id: below.matrix_room_id.clone(),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
                archived: false,
                parent: Some(NetworkAgentRoomParent {
                    catalog_id: other,
                    name: "别人的房间".to_owned(),
                }),
            },
        )
        .await
        .unwrap();
    harness.access.knocks.lock().unwrap().push(waiting_knock_on(
        below.catalog_id,
        AgentId::from_uuid(Uuid::now_v7()),
        "Critic",
    ));

    let waiting = harness
        .service
        .waiting_knocks(&token, below.matrix_room_id.as_str())
        .await
        .expect("按进过的 Matrix 房间 ID 认也行");
    assert_eq!(waiting.len(), 1);
    let incoming = harness.service.incoming_knocks(&token).await.unwrap();
    assert!(incoming.owns_rooms);
    assert_eq!(incoming.knocks.len(), 1);
    assert_eq!(incoming.knocks[0].room.catalog_id, below.catalog_id);

    assert_eq!(
        kind(
            harness
                .service
                .waiting_knocks(&token, &other.to_string())
                .await
        ),
        NetworkAgentFailureKind::RoomNotOwned,
        "别人的整间房管不了"
    );
}

// ---- 人的申请、把父房间里的人请进来（2h-4b）

fn pending_request(catalog_id: RoomCatalogId, requester: PrincipalId) -> PendingJoinRequest {
    PendingJoinRequest {
        record: JoinRequestRecord {
            catalog_id,
            requester,
            message: JoinRequestMessage::parse("想来帮忙").unwrap(),
            status: JoinRequestStatus::Pending,
            requested_at: UtcMillis::new(START).unwrap(),
            decided_at: None,
            decided_by: None,
        },
        requester_display_name: "Ada".to_owned(),
    }
}

#[tokio::test]
async fn 它管的子房间里看得到人的申请_请人就是直接请进来_被拒照实说_整间房照旧邀请() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let room = parent(&harness, &token, "项目").await;
    let whole = room.room.catalog_id.to_string();
    let below = sub_room(&harness, &token, &whole, "设计", None)
        .await
        .unwrap()
        .room;
    let number = below.catalog_id.to_string();
    let person = PrincipalId::from_uuid(Uuid::now_v7());
    harness
        .rooms
        .sub_room_requests
        .lock()
        .unwrap()
        .push(pending_request(below.catalog_id, person));

    let requests = harness
        .service
        .waiting_requests(&token, &number)
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].record.requester, person);
    assert!(
        harness
            .service
            .waiting_requests(&token, &whole)
            .await
            .unwrap()
            .is_empty(),
        "它建的整间房不公开，不收申请"
    );

    assert_eq!(
        harness
            .service
            .invite_person(&token, &number, person)
            .await
            .unwrap(),
        NetworkAgentInvitation::BroughtIn
    );
    let brought = harness.rooms.people_brought_in.lock().unwrap().clone();
    assert_eq!(brought.len(), 1);
    assert_eq!(brought[0].target, person);
    assert_eq!(brought[0].room.catalog_id, below.catalog_id);
    assert_eq!(Some(brought[0].room.agent_id), record.agent_id);
    assert!(
        harness.rooms.invited.lock().unwrap().is_empty(),
        "不走整间房的邀请"
    );

    for (refusal, expected) in [
        (
            SubRoomRefusal::NotInParent,
            NetworkAgentFailureKind::SubRoomPersonNotInParent,
        ),
        (
            SubRoomRefusal::TargetBanned,
            NetworkAgentFailureKind::SubRoomPersonBanned,
        ),
        (
            SubRoomRefusal::ManagersOnly,
            NetworkAgentFailureKind::RoomNotOwned,
        ),
    ] {
        *harness.rooms.admission_refusal.lock().unwrap() = Some(sub_room_refused(refusal));
        assert_eq!(
            kind(harness.service.invite_person(&token, &number, person).await),
            expected
        );
    }

    assert_eq!(
        harness
            .service
            .invite_person(&token, &whole, person)
            .await
            .unwrap(),
        NetworkAgentInvitation::Invited,
        "整间房照旧发邀请"
    );
    assert_eq!(harness.rooms.invited.lock().unwrap().len(), 1);
}
