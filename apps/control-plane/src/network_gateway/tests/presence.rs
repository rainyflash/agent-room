//! 服务器开着 Matrix 在线状态时，网络 Agent 的名片和在线状态（`specs/agent-liveness/design.md`
//! 第 3 步）。没开时照旧写租约，见上一层的“在线状态”一节。

use std::{collections::BTreeSet, time::Duration};

use agent_room_domain::agent_lifecycle::MatrixPresenceState::{Offline, Online, Unavailable};
use serde_json::Value;

use super::{
    Harness, NetworkAgentAdmission, NetworkAgentMessaging as _, NetworkAgentPendingExit,
    NetworkAgentRoomRequest, NetworkAgentTarget, OTHER_AGENT, ROOM, RoomCatalogId, SECOND_ROOM,
    Step, TOKEN, batch, entered, everything, harness, harness_in, restarted, uuid,
};

fn states(harness: &Harness) -> Vec<(String, Value)> {
    harness.matrix.states.lock().unwrap().clone()
}

fn session_rooms(harness: &Harness) -> BTreeSet<String> {
    harness
        .agents
        .own_session()
        .rooms
        .iter()
        .map(|room| room.matrix_room_id.as_str().to_owned())
        .collect()
}

fn assert_cards(states: &[(String, Value)]) {
    for (room, card) in states {
        assert_eq!(card["liveness"], "presence", "{room} 里是名片");
        assert_eq!(card["status"], "idle");
        assert!(card.get("listeningUntil").is_none(), "名片不说在不在等");
        assert!(card.get("waitingUntil").is_none());
    }
}

/// 第一次取消息只看一眼（不等），之后才真的等。
async fn peek_first(harness: &Harness) {
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn 每个房间一张名片_等消息报在线_等完一分钟报离开_五分钟后不再报_停用报离线() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness.matrix.enable_presence();
    peek_first(&harness).await;
    assert!(states(&harness).is_empty(), "第一次同步不等，也就不说在等");
    assert_eq!(
        harness.matrix.requests()[0].presence,
        Offline,
        "还没等过消息，同步不报"
    );

    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    let written = states(&harness);
    assert_eq!(
        written
            .iter()
            .map(|(room, _)| room.as_str())
            .collect::<Vec<_>>(),
        [ROOM, SECOND_ROOM],
        "每个房间一张名片"
    );
    assert_cards(&written);
    assert!(
        harness.matrix.requests()[1..]
            .iter()
            .all(|request| request.presence == Online),
        "等消息的同步带在线"
    );
    assert_eq!(
        harness.matrix.reported_presence(),
        [Online, Online],
        "开始等时探一次（报的就是在线），之后每 20 秒报一次"
    );

    // 等完一分钟内还算在等，之后报离开。
    tokio::time::sleep(Duration::from_secs(55)).await;
    assert_eq!(harness.matrix.reported_presence().last(), Some(&Online));
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(
        harness.matrix.reported_presence().last(),
        Some(&Unavailable)
    );

    // 等完 5 分钟以后不再报，Synapse 约 30 秒后改成离线。
    tokio::time::sleep(Duration::from_mins(5)).await;
    let reported = harness.matrix.reported_presence();
    tokio::time::sleep(Duration::from_mins(1)).await;
    assert_eq!(
        harness.matrix.reported_presence(),
        reported,
        "5 分钟以后不再报"
    );
    assert_eq!(reported.last(), Some(&Unavailable));
    assert_eq!(states(&harness).len(), 2, "名片之后不再写");

    harness.gateway.leave_and_disable(TOKEN).await.unwrap();
    assert_eq!(harness.matrix.reported_presence().last(), Some(&Offline));
    assert_eq!(states(&harness).len(), 2, "停用时不写房间状态");
}

#[tokio::test(start_paused = true)]
async fn 进房间就放名片报离开_不说在等() {
    let harness = harness();
    harness.matrix.enable_presence();
    harness
        .agents
        .admissions
        .lock()
        .unwrap()
        .push_back(Ok(NetworkAgentAdmission::Admitted(
            NetworkAgentTarget::Lobby(RoomCatalogId::from_uuid(uuid(OTHER_AGENT))),
        )));

    entered(
        harness
            .gateway
            .enter_room(
                TOKEN,
                NetworkAgentRoomRequest::Lobby(Some("rust-night".to_owned())),
                [1; 32],
            )
            .await
            .unwrap(),
    );

    let written = states(&harness);
    assert_eq!(
        written
            .iter()
            .map(|(room, _)| room.clone())
            .collect::<BTreeSet<_>>(),
        session_rooms(&harness),
        "进房间后每个房间都有名片：{written:?}"
    );
    assert_cards(&written);
    assert_eq!(
        harness.matrix.reported_presence(),
        [Unavailable],
        "进来时还没在等消息"
    );

    tokio::time::sleep(Duration::from_secs(400)).await;
    let reported = harness.matrix.reported_presence();
    assert!(reported.iter().all(|presence| *presence == Unavailable));
    tokio::time::sleep(Duration::from_mins(1)).await;
    assert_eq!(
        harness.matrix.reported_presence(),
        reported,
        "进来 5 分钟后不再报"
    );
}

#[tokio::test(start_paused = true)]
async fn 旧的租约换成名片_控制面重启以后房间里已经有一样的就不再写() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    peek_first(&harness).await;
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    let leases = states(&harness);
    assert_eq!(leases.len(), 2, "服务器没开在线状态：照旧写租约");
    assert!(
        leases
            .iter()
            .all(|(_, lease)| lease.get("liveness").is_none())
    );
    assert!(
        harness
            .matrix
            .requests()
            .iter()
            .all(|request| request.presence == Offline),
        "没开在线状态时同步不报"
    );
    assert!(harness.matrix.reported_presence().is_empty());

    // 服务器开了在线状态、控制面重启：房间里是旧的租约，换成名片。
    harness.matrix.enable_presence();
    let gateway = restarted(&harness);
    gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    let written = states(&harness);
    assert_eq!(written.len(), 4);
    assert_cards(&written[2..]);

    // 再重启一次：房间里已经是一样的名片，不再写。
    let gateway = restarted(&harness);
    gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert_eq!(states(&harness).len(), 4, "房间里已经有一样的名片");
}

#[tokio::test(start_paused = true)]
async fn 读不出房间里的那条就先不写名片_下次等消息时再看() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness.matrix.enable_presence();
    peek_first(&harness).await;
    *harness.matrix.state_read_fails.lock().unwrap() = true;
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert!(states(&harness).is_empty(), "不知道房间里有没有就先不写");

    *harness.matrix.state_read_fails.lock().unwrap() = false;
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    let written = states(&harness);
    assert_eq!(written.len(), 2, "下次等消息时补上：{written:?}");
    assert_cards(&written);
}

#[tokio::test(start_paused = true)]
async fn 控制面重启以后的定时清理替它离开_房间里是名片就不补写_只报离线() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness.matrix.enable_presence();
    peek_first(&harness).await;
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert_eq!(states(&harness).len(), 2, "每个房间一张名片");

    // 停用时没离开成，控制面重启以后由定时清理替它离开：这个进程还没探过开没开在线状态。
    let gateway = restarted(&harness);
    *harness.agents.exits.lock().unwrap() = vec![NetworkAgentPendingExit::Session(Box::new(
        harness.agents.own_session(),
    ))];
    gateway.clean_up().await.unwrap();

    assert_eq!(
        states(&harness).len(),
        2,
        "名片只写一次，不补“已离线”的租约"
    );
    assert_eq!(harness.matrix.reported_presence().last(), Some(&Offline));
}
