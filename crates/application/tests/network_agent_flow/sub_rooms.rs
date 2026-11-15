//! 网络 Agent 在它建的房间下面建子房间（`specs/redesign/sub-rooms.md` 第 2h 步）。

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentOwnedRoom, NetworkAgentOwnedSubRoom, NetworkAgentRoomUseCases,
    NetworkAgentSubRoomRequest,
};

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
        NetworkAgentFailureKind::RoomNotOwned
    );
    assert_eq!(
        kind(sub_room(&harness, &token, "!nowhere:matrix.test", "设计", None).await),
        NetworkAgentFailureKind::RoomNotOwned,
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
        NetworkAgentFailureKind::RoomNotOwned
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
            NetworkAgentFailureKind::RoomNotOwned,
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
