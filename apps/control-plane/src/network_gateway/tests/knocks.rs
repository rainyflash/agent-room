//! 有人敲网络 Agent 建的房间的门，它正在等消息就叫醒它（`specs/agent-rooms/design.md` 的“敲门叫醒
//! 房主 Agent”）：同一次敲门只叫醒一次，回答里一直列着在等的；这个进程里有人敲门马上叫醒，一间房都
//! 没有的不被打断。

use std::{sync::Arc, time::Duration};

use agent_room_application::{
    network_agents::{
        NetworkAgentAdmission, NetworkAgentFailure, NetworkAgentFailureKind,
        NetworkAgentIncomingKnock, NetworkAgentIncomingKnocks, NetworkAgentKnock,
        NetworkAgentKnockStatus, NetworkAgentRoomRequest,
    },
    ports::PrivateRoomAgentKnockRecord,
};
use agent_room_bridge_ipc::wake::WakeReason;
use agent_room_domain::{ids::AgentId, join_codes::PrivateRoomAgentKnockStatus, time::UtcMillis};
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    Harness, NetworkAgentMessaging, Step, TOKEN, batch, chat, everything, harness, knock, other,
    private_room, related, settle_in, texts,
};
use crate::network_gateway::NetworkAgentMessages;

/// 拿房间号敲门的另一个网络 Agent。
const KNOCKER: &str = "knocker-token";

/// 有人在第 `minute` 分钟敲了 `private_room()` 的门。
fn knocking(name: &str, minute: i64) -> NetworkAgentIncomingKnock {
    NetworkAgentIncomingKnock {
        room: private_room(),
        knock: PrivateRoomAgentKnockRecord {
            catalog_id: private_room().catalog_id,
            agent_id: AgentId::from_uuid(Uuid::now_v7()),
            display_name: name.to_owned(),
            status: PrivateRoomAgentKnockStatus::Waiting,
            knocked_at: UtcMillis::new(1_000 + minute * 60_000).unwrap(),
            expires_at: UtcMillis::new(3_601_000 + minute * 60_000).unwrap(),
            decided_at: None,
        },
    }
}

/// 它有没关的房间，房间里在等的是这几个。
fn owns(harness: &Harness, knocks: &[NetworkAgentIncomingKnock]) {
    *harness.agents.incoming.lock().unwrap() = Ok(NetworkAgentIncomingKnocks {
        owns_rooms: true,
        knocks: knocks.to_vec(),
    });
}

fn checks(harness: &Harness) -> usize {
    *harness.agents.incoming_checks.lock().unwrap()
}

/// 等 5 秒消息；同步正等着的时候，房间里先换成 `then` 这几个在等，另一个网络 Agent 再拿房间号敲门，
/// 敲门的结果是 `status`。返回等到的和这次等消息里去看了几次敲门。
async fn knocked_while_waiting(
    harness: &Arc<Harness>,
    then: Option<&[NetworkAgentIncomingKnock]>,
    status: NetworkAgentKnockStatus,
) -> (NetworkAgentMessages, usize) {
    let before = checks(harness);
    let synced = harness.matrix.requests().len();
    let waiting = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .gateway
                .wait_for_messages(TOKEN, related(Duration::from_secs(5)))
                .await
        })
    };
    while harness.matrix.requests().len() <= synced {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    if let Some(knocks) = then {
        owns(harness, knocks);
    }
    harness
        .agents
        .admissions
        .lock()
        .unwrap()
        .push_back(Ok(NetworkAgentAdmission::Knocked(NetworkAgentKnock {
            status,
            ..knock()
        })));
    harness
        .gateway
        .enter_room(
            KNOCKER,
            NetworkAgentRoomRequest::Lobby(Some(private_room().catalog_id.to_string())),
            [2; 32],
        )
        .await
        .expect("敲了门");
    let page = waiting.await.unwrap().expect("等到了");
    (page, checks(harness) - before)
}

#[tokio::test(start_paused = true)]
async fn 有人敲它建的房间的门_等消息时马上叫醒_同一次敲门不再叫醒_回答里一直列着() {
    let harness = harness();
    settle_in(&harness).await;
    let scout = knocking("Scout", 1);
    owns(&harness, std::slice::from_ref(&scout));

    let started = Instant::now();
    let woken = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(woken.wake.reason, WakeReason::Knocks);
    assert!(woken.messages.is_empty());
    assert_eq!(woken.incoming_knocks, std::slice::from_ref(&scout));
    assert_eq!(started.elapsed(), Duration::ZERO, "不等，马上叫醒");

    let again = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(
        again.wake.reason,
        WakeReason::Timeout,
        "同一次敲门不再叫醒它"
    );
    assert_eq!(
        again.incoming_knocks,
        std::slice::from_ref(&scout),
        "还在等它回答，照旧列着"
    );

    let critic = knocking("Critic", 2);
    owns(&harness, &[scout.clone(), critic.clone()]);
    let started = Instant::now();
    let woken = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(woken.wake.reason, WakeReason::Knocks, "又有人敲门");
    assert_eq!(woken.incoming_knocks, [scout, critic]);
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn 交消息时也列着在等的敲门_交过以后不再为它叫醒() {
    let harness = harness();
    settle_in(&harness).await;
    let scout = knocking("Scout", 1);
    owns(&harness, std::slice::from_ref(&scout));
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$hello:matrix.test",
            other(),
            Uuid::now_v7(),
            "大家好",
            [1; 64],
        )],
    ))));

    let delivered = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(texts(&delivered.messages), ["大家好"]);
    assert_eq!(delivered.wake.reason, WakeReason::Messages);
    assert_eq!(delivered.incoming_knocks, std::slice::from_ref(&scout));

    let quiet = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(quiet.wake.reason, WakeReason::Timeout);
    assert_eq!(quiet.incoming_knocks, [scout]);
}

#[tokio::test(start_paused = true)]
async fn 这个进程里有人敲门_正在等的房主马上被叫醒() {
    let harness = Arc::new(harness());
    settle_in(&harness).await;
    owns(&harness, &[]);
    let scout = knocking("Scout", 1);

    let started = Instant::now();
    let (page, checked) = knocked_while_waiting(
        &harness,
        Some(std::slice::from_ref(&scout)),
        NetworkAgentKnockStatus::Waiting,
    )
    .await;
    assert_eq!(page.wake.reason, WakeReason::Knocks);
    assert_eq!(page.incoming_knocks, [scout]);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(2),
        "敲门那一刻就叫醒，不等这次同步到点"
    );
    assert_eq!(checked, 2, "开始时看一次，听到铃再看一次");
}

#[tokio::test(start_paused = true)]
async fn 一间房都没有的_或者敲门的以前没让进_都不打断它的同步() {
    let harness = Arc::new(harness());
    settle_in(&harness).await;
    let (page, checked) =
        knocked_while_waiting(&harness, None, NetworkAgentKnockStatus::Waiting).await;
    assert_eq!(page.wake.reason, WakeReason::Timeout);
    assert!(page.incoming_knocks.is_empty());
    assert_eq!(checked, 1, "谁敲门都跟它无关，只在开始时看了一眼");

    owns(&harness, &[]);
    let (page, checked) =
        knocked_while_waiting(&harness, None, NetworkAgentKnockStatus::Declined).await;
    assert_eq!(page.wake.reason, WakeReason::Timeout);
    assert_eq!(checked, 1, "以前没让进的再敲，不打扰房主");
}

#[tokio::test(start_paused = true)]
async fn 看不成有没有人敲门时照常等消息() {
    let harness = harness();
    settle_in(&harness).await;
    *harness.agents.incoming.lock().unwrap() = Err(NetworkAgentFailure::new(
        NetworkAgentFailureKind::DependencyUnavailable,
    ));

    let page = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(5)))
        .await
        .expect("照常等到时间");
    assert_eq!(page.wake.reason, WakeReason::Timeout);
    assert!(page.incoming_knocks.is_empty());
}
