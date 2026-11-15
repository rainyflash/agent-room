//! 网络 Agent 只离开一间（`specs/redesign/sub-rooms.md` 第 2h-3b 步）。

use agent_room_application::{
    agent_room_placements::AgentRoomActor,
    network_agents::{NetworkAgentFailure, NetworkAgentRoomUseCases},
    ports::{
        MatrixFailure, MatrixFailureKind, MatrixOperation, NetworkAgentLobbyAway,
        NetworkAgentRoomRecord,
    },
};

use super::*;

async fn agent(harness: &Harness) -> String {
    let created = harness.create("Scout", None).await.expect("起名成功");
    created.token.expose().to_owned()
}

/// 它凭口令进了的一间私人房间。
async fn entered(harness: &Harness, seed: u128) -> RoomCatalogId {
    let catalog = catalog_id(seed);
    let id = harness.store.only().id;
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id: catalog,
                matrix_room_id: owned_matrix_room(catalog),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
                archived: false,
                parent: None,
            },
        )
        .await
        .unwrap();
    catalog
}

fn kind<T>(result: Result<T, NetworkAgentFailure>) -> NetworkAgentFailureKind {
    result.err().expect("应当被拒").kind()
}

#[tokio::test]
async fn 只离开一间_按房间号或_room_id_交给离开那一套_忘掉这间_别的房间还在() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let first = entered(&harness, 501).await;
    let second = entered(&harness, 502).await;

    let left = harness
        .service
        .leave_room(&token, &format!("https://agentroom.chat/lobby/{first}"))
        .await
        .expect("离开了");

    assert_eq!(left.catalog_id, first);
    let requests = harness.placements.left.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(Some(requests[0].agent_id), record.agent_id);
    assert_eq!(requests[0].catalog_id, first);
    let AgentRoomActor::Device {
        device,
        agent_instance_id,
    } = &requests[0].actor
    else {
        panic!("网络 Agent 以它的设备和连接离开：{:?}", requests[0].actor);
    };
    assert_eq!(device.device_id, record.device_id);
    assert_eq!(Some(*agent_instance_id), record.agent_instance_id);
    let rooms: Vec<RoomCatalogId> = harness
        .store
        .rooms(record.id)
        .await
        .unwrap()
        .into_iter()
        .map(|room| room.catalog_id)
        .collect();
    assert!(!rooms.contains(&first), "忘掉了离开的这间");
    assert!(rooms.contains(&second), "别的房间还在");

    let by_matrix = harness
        .service
        .leave_room(&token, owned_matrix_room(second).as_str())
        .await
        .expect("Matrix 房间 ID 也行");
    assert_eq!(by_matrix.catalog_id, second);
}

#[tokio::test]
async fn 不在这间房里说不在_离开没做成时不忘掉() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();

    assert_eq!(
        kind(
            harness
                .service
                .leave_room(&token, &catalog_id(599).to_string())
                .await
        ),
        NetworkAgentFailureKind::RoomNotJoined
    );
    assert_eq!(
        kind(
            harness
                .service
                .leave_room(&token, "!nowhere:matrix.test")
                .await
        ),
        NetworkAgentFailureKind::RoomNotJoined
    );
    assert!(harness.placements.left.lock().unwrap().is_empty());

    let room = entered(&harness, 503).await;
    *harness.placements.failure.lock().unwrap() = Some(AgentLobbyEntryFailure::Membership(
        MatrixFailure::new(MatrixOperation::Leave, MatrixFailureKind::Timeout),
    ));
    assert_eq!(
        kind(harness.service.leave_room(&token, &room.to_string()).await),
        NetworkAgentFailureKind::DependencyUnavailable
    );
    assert!(
        harness
            .store
            .rooms(record.id)
            .await
            .unwrap()
            .iter()
            .any(|joined| joined.catalog_id == room),
        "聊天服务器上没离开成：还记着，再来一次"
    );
    assert_eq!(
        kind(harness.service.leave_room("nope", &room.to_string()).await),
        NetworkAgentFailureKind::Unauthorized
    );
}

#[tokio::test]
async fn 闲置时请出的公开大厅也能离开_不再替它回去() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let lobby = record_of(&harness).await;
    harness
        .store
        .mark_lobby_away(record.id, &lobby, UtcMillis::new(START + 10).unwrap())
        .await
        .unwrap();
    assert_eq!(
        harness.store.lobbies_away(record.id).await.unwrap(),
        [NetworkAgentLobbyAway {
            catalog_id: lobby.catalog_id,
            matrix_room_id: lobby.matrix_room_id.clone(),
            left_at: UtcMillis::new(START + 10).unwrap(),
        }]
    );

    let left = harness
        .service
        .leave_room(&token, &lobby.catalog_id.to_string())
        .await
        .expect("离开了");

    assert_eq!(left.catalog_id, lobby.catalog_id);
    assert!(
        harness.placements.left.lock().unwrap().is_empty(),
        "Matrix 上早离开了，位置也放了"
    );
    assert!(
        harness
            .store
            .lobbies_away(record.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// 它起名时进的那个公开大厅。
async fn record_of(harness: &Harness) -> NetworkAgentRoomRecord {
    let id = harness.store.only().id;
    harness
        .store
        .rooms(id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("起名时进了大厅")
}
