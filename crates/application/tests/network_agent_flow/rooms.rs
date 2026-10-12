//! 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）。

use agent_room_application::network_agents::{
    NetworkAgentOwnedRoom, NetworkAgentRoomCode, NetworkAgentRoomUseCases,
};

use super::*;

/// 起一个在公开大厅里的网络 Agent，拿到它的令牌。
async fn agent(harness: &Harness) -> String {
    let created = harness.create("Scout", None).await.expect("起名成功");
    created.token.expose().to_owned()
}

async fn create_room(
    harness: &Harness,
    token: &str,
    name: &str,
) -> Result<NetworkAgentOwnedRoom, NetworkAgentFailureKind> {
    harness
        .service
        .create_room(token, name)
        .await
        .map_err(|failure| failure.kind())
}

async fn close_room(
    harness: &Harness,
    token: &str,
    room: &str,
) -> Result<(), NetworkAgentFailureKind> {
    harness
        .service
        .close_room(token, room)
        .await
        .map_err(|failure| failure.kind())
}

fn visible(code: &NetworkAgentRoomCode) -> &str {
    match code {
        NetworkAgentRoomCode::Visible(code) => code,
        NetworkAgentRoomCode::Hidden => panic!("口令应当看得到"),
    }
}

async fn record_joined(harness: &Harness, room: &NetworkAgentRoom) {
    let id = harness.store.only().id;
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id: room.catalog_id,
                matrix_room_id: room.matrix_room_id.clone(),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn 建房_房主是它自己_记下建房的_agent_顺手生成口令_还没进去() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();

    let owned = create_room(&harness, &token, "  周报 ").await.unwrap();

    assert!(owned.created);
    assert!(!owned.joined, "由网关让加密客户端就绪后再进");
    assert_eq!(owned.room.name, "周报", "名字去掉首尾空白");
    assert_eq!(
        owned.room.matrix_room_id,
        owned_matrix_room(owned.room.catalog_id)
    );
    {
        let created = harness.rooms.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].actor.principal_id(),
            record.principal_id,
            "房主是它的合成主体"
        );
        assert_eq!(created[0].created_by_agent, record.agent_id);
        assert_eq!(created[0].catalog_id, owned.room.catalog_id);
        assert!(created[0].invitations.is_empty());
    }
    assert_eq!(
        *harness.access.managed.lock().unwrap(),
        [(record.principal_id, owned.room.catalog_id, "generate")],
        "新房间直接生成口令，不先看"
    );
    assert_eq!(
        visible(&owned.code),
        harness.access.codes.lock().unwrap()[&owned.room.catalog_id].display()
    );
}

#[tokio::test]
async fn 同名的没关的房间回到那间_口令照旧给_进过的标出来() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let first = create_room(&harness, &token, "周报").await.unwrap();

    let again = create_room(&harness, &token, "周报 ").await.unwrap();
    assert!(!again.created);
    assert!(!again.joined, "上次没进去，这次由网关接着进");
    assert_eq!(again.room, first.room);
    assert_eq!(visible(&again.code), visible(&first.code));
    assert_eq!(harness.rooms.created.lock().unwrap().len(), 1, "没多建一间");
    assert_eq!(harness.access.generated_codes(), 1, "看得到就不换口令");

    record_joined(&harness, &first.room).await;
    assert!(
        create_room(&harness, &token, "周报").await.unwrap().joined,
        "已经在里面：网关不用再进一次"
    );

    let other = create_room(&harness, &token, "Weekly").await.unwrap();
    assert!(other.created);
    assert_ne!(other.room.catalog_id, first.room.catalog_id);
}

#[tokio::test]
async fn 口令看不到时不替它换_没有了就再生成一个() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let first = create_room(&harness, &token, "周报").await.unwrap();
    let catalog_id = first.room.catalog_id;

    harness.access.hidden.lock().unwrap().insert(catalog_id);
    let hidden = create_room(&harness, &token, "周报").await.unwrap();
    assert_eq!(hidden.code, NetworkAgentRoomCode::Hidden);
    assert_eq!(
        harness.access.generated_codes(),
        1,
        "换了口令，已经拿到旧口令、还没进来的 Agent 就进不来了"
    );

    harness.access.hidden.lock().unwrap().clear();
    harness.access.codes.lock().unwrap().clear();
    let regenerated = create_room(&harness, &token, "周报").await.unwrap();
    assert_eq!(harness.access.generated_codes(), 2);
    assert_ne!(visible(&regenerated.code), visible(&first.code));
}

#[tokio::test]
async fn 房间名不对时什么都不建_也不占当天的名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        rooms_per_agent_per_day: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let token = agent(&harness).await;

    for name in ["", "   ", &"长".repeat(129), "周\n报"] {
        assert_eq!(
            create_room(&harness, &token, name).await.unwrap_err(),
            NetworkAgentFailureKind::RoomNameInvalid,
            "{name:?}"
        );
    }
    assert!(harness.rooms.created.lock().unwrap().is_empty());
    assert!(
        create_room(&harness, &token, &"长".repeat(128))
            .await
            .unwrap()
            .created,
        "128 个字符可以，当天的名额还在"
    );
}

#[tokio::test]
async fn 每个网络_agent_同时五个没关的房间_关掉一个就能再建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let mut rooms = Vec::new();
    for index in 0..5 {
        rooms.push(
            create_room(&harness, &token, &format!("房间 {index}"))
                .await
                .unwrap(),
        );
    }

    assert_eq!(
        create_room(&harness, &token, "第六间").await.unwrap_err(),
        NetworkAgentFailureKind::RoomLimitReached
    );
    assert_eq!(
        create_room(&harness, &token, "房间 0").await.unwrap().room,
        rooms[0].room,
        "回到已有的同名房间不算新建"
    );

    close_room(&harness, &token, &rooms[0].room.catalog_id.to_string())
        .await
        .unwrap();
    assert!(
        create_room(&harness, &token, "第六间")
            .await
            .unwrap()
            .created
    );
}

#[tokio::test]
async fn 每天最多建十个_超了告诉什么时候能再建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    for index in 0..10 {
        let owned = create_room(&harness, &token, &format!("房间 {index}"))
            .await
            .unwrap();
        close_room(&harness, &token, &owned.room.catalog_id.to_string())
            .await
            .unwrap();
    }

    let failure = harness
        .service
        .create_room(&token, "第十一间")
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        failure.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );

    harness.runtime.advance(24 * HOUR);
    assert!(
        create_room(&harness, &token, "第十一间")
            .await
            .unwrap()
            .created
    );
}

#[tokio::test]
async fn 全站网络_agent_建的没关的房间满了就请稍后_不占当天的名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        rooms_per_agent_per_day: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let token = agent(&harness).await;
    *harness.store.owned_private_rooms.lock().unwrap() = 500;

    assert_eq!(
        create_room(&harness, &token, "周报").await.unwrap_err(),
        NetworkAgentFailureKind::RoomCapacityReached
    );
    assert!(harness.rooms.created.lock().unwrap().is_empty());

    *harness.store.owned_private_rooms.lock().unwrap() = 499;
    assert!(create_room(&harness, &token, "周报").await.unwrap().created);
}

#[tokio::test]
async fn 关房间只认它当房主的_房间号和_room_id_都行_关了的再关一次也成功() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let number = owned.room.catalog_id.to_string();

    close_room(&harness, &token, &number).await.unwrap();
    close_room(
        &harness,
        &token,
        &format!("https://agentroom.chat/lobby/{number}"),
    )
    .await
    .unwrap();
    assert_eq!(
        *harness.rooms.archived.lock().unwrap(),
        [(record.principal_id, owned.room.catalog_id); 2]
    );
    assert!(
        create_room(&harness, &token, "周报").await.unwrap().created,
        "关了的不算同名"
    );

    let second = create_room(&harness, &token, "第二间").await.unwrap();
    record_joined(&harness, &second.room).await;
    close_room(&harness, &token, second.room.matrix_room_id.as_str())
        .await
        .unwrap();
    assert_eq!(harness.rooms.archived.lock().unwrap().len(), 3);

    let others = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    for room in [
        others.to_string(),
        catalog_id(78).to_string(),
        "!unknown:matrix.test".to_owned(),
        "不是房间".to_owned(),
    ] {
        assert_eq!(
            close_room(&harness, &token, &room).await.unwrap_err(),
            NetworkAgentFailureKind::RoomNotOwned,
            "别人的房间、没有的房间、没进过的房间都只说不是它的：{room}"
        );
    }
    assert_eq!(harness.rooms.archived.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn 令牌不对或总开关关着时不建也不关() {
    let harness = Harness::enabled();
    agent(&harness).await;
    assert_eq!(
        create_room(&harness, "wrong", "周报").await.unwrap_err(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert_eq!(
        close_room(&harness, "wrong", &catalog_id(9).to_string())
            .await
            .unwrap_err(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert!(harness.rooms.created.lock().unwrap().is_empty());

    let disabled = Harness::new(NetworkAgentPolicy::default_limits(false));
    assert_eq!(
        create_room(&disabled, "token-1", "周报").await.unwrap_err(),
        NetworkAgentFailureKind::Disabled
    );
}
