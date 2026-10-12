//! 网络 Agent 自己建房间、管房间（`specs/agent-rooms/design.md` 第 4 步）：建好以后和凭口令进私人
//! 房间一样先切到加密客户端再进；回到已经进过的同名房间不再切。放行敲它房间的门时替敲门的进去，
//! 进去了才记放进来了。

use agent_room_application::{
    network_agents::{
        NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentOwnedRoom, NetworkAgentRoom,
        NetworkAgentRoomCode, NetworkAgentTarget,
    },
    ports::{MatrixUserId, PrivateRoomAgentMemberRecord},
    private_rooms::{AdmittedKnock, RedeemedRoom},
};
use agent_room_domain::{
    join_codes::PrivateRoomAgentMemberStatus, private_rooms::PrivateRoomPermissions,
    time::UtcMillis,
};

use super::{
    NetworkGatewayFailure, OTHER_AGENT, OWN_AGENT, TOKEN, agent, assert_online_announced, harness,
    private_room,
};
use crate::network_gateway::NetworkAgentRooms;

const ROOM_NUMBER: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e54";

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

/// 放行记成员的结果：敲门的 Agent 记为这个房间的成员，还没进去。
fn admitted(agent_id: &str) -> AdmittedKnock {
    let room = private_room();
    AdmittedKnock {
        room: RedeemedRoom {
            catalog_id: room.catalog_id,
            matrix_room_id: room.matrix_room_id,
            name: room.name,
        },
        agent: PrivateRoomAgentMemberRecord {
            catalog_id: room.catalog_id,
            agent_id: agent(agent_id),
            display_name: "Scout".to_owned(),
            matrix_user_id: MatrixUserId::new("@scout:matrix.test".to_owned()).unwrap(),
            owner_display_name: None,
            status: PrivateRoomAgentMemberStatus::Joined,
            permissions: PrivateRoomPermissions::AGENT_MEMBER,
            joined_at: UtcMillis::new(1).unwrap(),
            status_changed_at: UtcMillis::new(1).unwrap(),
        },
    }
}

#[tokio::test(start_paused = true)]
async fn 放行敲门先记成员_再替敲门的进房间_进去了才记放进来了() {
    let harness = harness();
    harness
        .agents
        .knock_admissions
        .lock()
        .unwrap()
        .push_back(Ok(admitted(OWN_AGENT)));

    let member = harness
        .gateway
        .admit_knock(TOKEN, ROOM_NUMBER, agent(OWN_AGENT))
        .await
        .expect("放进来了");

    assert_eq!(member.agent_id, agent(OWN_AGENT));
    assert_eq!(member.display_name, "Scout");
    assert_eq!(
        harness.agents.log(),
        [
            format!("admit_knock {ROOM_NUMBER}"),
            "session_of_agent".to_owned(),
            "mark_encrypted".to_owned(),
            "prepare".to_owned(),
            "enter_admitted".to_owned(),
            "session_of_agent".to_owned(),
            "refresh".to_owned(),
            format!("complete_knock {ROOM_NUMBER}"),
        ],
        "和管理者在网页上放行一样：身份建好之前不进，进去了才记放进来了"
    );
    assert_eq!(
        *harness.agents.entered.lock().unwrap(),
        [NetworkAgentTarget::Private(private_room())]
    );
}

#[tokio::test(start_paused = true)]
async fn 敲门的已经停用_说敲门不在了_不说房主的令牌不对() {
    let harness = harness();
    harness
        .agents
        .knock_admissions
        .lock()
        .unwrap()
        .push_back(Ok(admitted(OTHER_AGENT)));

    assert_eq!(
        harness
            .gateway
            .admit_knock(TOKEN, ROOM_NUMBER, agent(OTHER_AGENT))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::KnockNotFound
        ))
    );
    assert!(
        !harness
            .agents
            .log()
            .contains(&format!("complete_knock {ROOM_NUMBER}")),
        "没进去就不记放进来了"
    );
}

#[tokio::test(start_paused = true)]
async fn 替它进房间暂时没成_请房主稍后再放行一次_敲门还在等() {
    let harness = harness();
    *harness.encrypted.prepare_failure.lock().unwrap() = Some(NetworkGatewayFailure::Unavailable);
    harness
        .agents
        .knock_admissions
        .lock()
        .unwrap()
        .push_back(Ok(admitted(OWN_AGENT)));

    assert_eq!(
        harness
            .gateway
            .admit_knock(TOKEN, ROOM_NUMBER, agent(OWN_AGENT))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert!(harness.agents.entered.lock().unwrap().is_empty());
    assert!(
        !harness
            .agents
            .log()
            .contains(&format!("complete_knock {ROOM_NUMBER}"))
    );
    assert!(harness.agents.disabled.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 进去了却没记成放进来了_照样回答放进来了() {
    let harness = harness();
    *harness.agents.completion_failure.lock().unwrap() = Some(NetworkAgentFailure::new(
        NetworkAgentFailureKind::DependencyUnavailable,
    ));
    harness
        .agents
        .knock_admissions
        .lock()
        .unwrap()
        .push_back(Ok(admitted(OWN_AGENT)));

    let member = harness
        .gateway
        .admit_knock(TOKEN, ROOM_NUMBER, agent(OWN_AGENT))
        .await
        .expect("它已经在房间里了");
    assert_eq!(member.agent_id, agent(OWN_AGENT));
}

#[tokio::test(start_paused = true)]
async fn 建房和放行的失败照原样交给接口_不进房间() {
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
    let gone = NetworkAgentFailure::new(NetworkAgentFailureKind::KnockNotFound);
    harness
        .agents
        .knock_admissions
        .lock()
        .unwrap()
        .push_back(Err(gone.clone()));
    assert_eq!(
        harness
            .gateway
            .admit_knock(TOKEN, ROOM_NUMBER, agent(OWN_AGENT))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(gone)
    );
    assert_eq!(
        harness.agents.log(),
        [
            "create_room 第六间".to_owned(),
            format!("admit_knock {ROOM_NUMBER}")
        ]
    );
    assert!(harness.agents.entered.lock().unwrap().is_empty());
}
