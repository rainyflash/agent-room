//! 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）：建好以后和凭口令进私人
//! 房间一样先切到加密客户端再进；回到已经进过的同名房间不再切。

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentOwnedRoom, NetworkAgentRoom,
    NetworkAgentRoomCode, NetworkAgentTarget,
};

use super::{NetworkGatewayFailure, TOKEN, assert_online_announced, harness, private_room};
use crate::network_gateway::NetworkAgentRooms;

fn owned(room: NetworkAgentRoom, created: bool, joined: bool) -> NetworkAgentOwnedRoom {
    NetworkAgentOwnedRoom {
        room,
        created,
        joined,
        code: NetworkAgentRoomCode::Visible("K7P3-Q9XW-2DMA".to_owned()),
    }
}

#[tokio::test(start_paused = true)]
async fn 建好房间以后和凭口令进私人房间一样_先切到加密客户端再进_说一声在线() {
    let harness = harness();
    harness
        .agents
        .owned
        .lock()
        .unwrap()
        .push_back(Ok(owned(private_room(), true, false)));

    let created = harness
        .gateway
        .create_room(TOKEN, "项目室")
        .await
        .expect("建房成功");

    assert!(created.created);
    assert!(created.joined, "进去了");
    assert_eq!(created.room, private_room());
    assert_eq!(
        harness.agents.log(),
        [
            "create_room 项目室",
            "mark_encrypted",
            "prepare",
            "enter",
            "refresh"
        ],
        "身份建好之前不进：房间密钥只发给由主人交叉签名的设备"
    );
    assert_eq!(
        *harness.agents.entered.lock().unwrap(),
        [NetworkAgentTarget::Private(private_room())]
    );
    assert_online_announced(&harness);
}

#[tokio::test(start_paused = true)]
async fn 回到已经进过的同名房间_不再切加密客户端_也不打断正在等的消息() {
    let harness = harness();
    harness
        .agents
        .owned
        .lock()
        .unwrap()
        .push_back(Ok(owned(private_room(), false, true)));

    let again = harness.gateway.create_room(TOKEN, "项目室").await.unwrap();

    assert!(!again.created);
    assert_eq!(harness.agents.log(), ["create_room 项目室"]);
    assert!(harness.agents.entered.lock().unwrap().is_empty());
    assert!(harness.matrix.states.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 进房间没成就照实报错_不停用它_再调一次回到那间接着进() {
    let harness = harness();
    *harness.encrypted.prepare_failure.lock().unwrap() = Some(NetworkGatewayFailure::Unavailable);
    harness.agents.owned.lock().unwrap().extend([
        Ok(owned(private_room(), true, false)),
        Ok(owned(private_room(), false, false)),
    ]);

    assert_eq!(
        harness
            .gateway
            .create_room(TOKEN, "项目室")
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert!(
        harness.agents.disabled.lock().unwrap().is_empty(),
        "它本来就有令牌，建房没进去不该停用它"
    );
    assert!(harness.agents.entered.lock().unwrap().is_empty());

    *harness.encrypted.prepare_failure.lock().unwrap() = None;
    let retried = harness.gateway.create_room(TOKEN, "项目室").await.unwrap();
    assert!(!retried.created);
    assert_eq!(
        *harness.agents.entered.lock().unwrap(),
        [NetworkAgentTarget::Private(private_room())]
    );
}

#[tokio::test(start_paused = true)]
async fn 建房和关房的失败照原样交给接口() {
    let harness = harness();
    let limit = NetworkAgentFailure::new(NetworkAgentFailureKind::RoomLimitReached);
    harness
        .agents
        .owned
        .lock()
        .unwrap()
        .push_back(Err(limit.clone()));

    assert_eq!(
        harness
            .gateway
            .create_room(TOKEN, "第六间")
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(limit)
    );
    harness
        .gateway
        .close_room(TOKEN, "!private:matrix.test")
        .await
        .unwrap();
    assert_eq!(
        harness.agents.log(),
        ["create_room 第六间", "close_room !private:matrix.test"]
    );
}
