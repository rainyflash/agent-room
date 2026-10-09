//! Matrix 在线状态（`specs/agent-liveness/design.md` 第 2 步）。读的一边靠的几件事在真实
//! Synapse 上成立：同步里带回同房间的人的在线和离开，`GET /presence` 问得到，停止同步约
//! 30 秒后服务器自己改成离线，首次同步不带离线的人。

use std::time::Duration;

use agent_room_application::ports::{MatrixGateway, MatrixUserId, MatrixUserPresence};
use agent_room_domain::agent_lifecycle::MatrixPresenceState;
use matrix_sdk::ruma::{
    UserId, api::client::presence::set_presence::v3::Request as SetPresenceRequest,
    presence::PresenceState,
};
use tokio::time::{Instant, sleep};

use super::{
    TEST_REQUEST_TIMEOUT, application_service_provisioner, create_room_with_retry, factory,
    invite_with_retry, join_with_retry, managed_device, required_environment, room_request, sync,
};

/// 服务器约 30 秒没见到同步就改成离线，再加上它每隔几秒才检查一次。
const OFFLINE_WITHIN: Duration = Duration::from_secs(90);
const CHANGE_WITHIN: Duration = Duration::from_secs(20);

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 真实_synapse_同步带回同房间的人的在线状态_停止同步后服务器改成离线() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let watcher = managed_device(&provisioner, &factory).await;
    let agent = managed_device(&provisioner, &factory).await;
    let watching = watcher.matrix().gateway();
    let agent_id = agent.matrix().session().metadata().user_id().clone();
    let room_id = create_room_with_retry(watching, &room_request()).await;
    invite_with_retry(watching, &room_id, &agent_id).await;
    join_with_retry(agent.matrix().gateway(), &room_id).await;

    // 同步时默认报在线：同房间的人在同步里看得到，也问得到。
    sync(agent.matrix().gateway(), None).await;
    synced_presence(
        watching,
        &agent_id,
        MatrixPresenceState::Online,
        CHANGE_WITHIN,
    )
    .await;
    let asked = watching
        .user_presence(&agent_id)
        .await
        .expect("同房间的人问得到在线状态");
    assert_eq!(asked.state(), MatrixPresenceState::Online);

    // 报“离开”（连着、没在等消息），同步里看得到。
    agent
        .sdk_client()
        .send(SetPresenceRequest::new(
            UserId::parse(agent_id.as_str()).expect("Agent 的 Matrix ID 有效"),
            PresenceState::Unavailable,
        ))
        .await
        .expect("可以报离开");
    synced_presence(
        watching,
        &agent_id,
        MatrixPresenceState::Unavailable,
        CHANGE_WITHIN,
    )
    .await;

    // 不再同步：服务器过一会儿自己改成离线，带着“上次活动”。
    let offline = synced_presence(
        watching,
        &agent_id,
        MatrixPresenceState::Offline,
        OFFLINE_WITHIN,
    )
    .await;
    assert!(
        offline.last_active_ago_ms().is_some(),
        "离线的在线状态要带“上次活动”，没看到它变离线时靠它算离线多久"
    );

    // 首次同步只带不离线的人；离线的问得到。
    let fresh = sync(watching, None).await;
    assert!(
        fresh
            .presence()
            .iter()
            .all(|presence| presence.user_id() != &agent_id),
        "首次同步不带离线的人"
    );
    let asked = watching
        .user_presence(&agent_id)
        .await
        .expect("离线的人也问得到");
    assert_eq!(asked.state(), MatrixPresenceState::Offline);
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 真实_synapse_不同房间的人问不到在线状态() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let watcher = managed_device(&provisioner, &factory).await;
    let stranger = managed_device(&provisioner, &factory).await;
    let stranger_id = stranger.matrix().session().metadata().user_id().clone();
    sync(stranger.matrix().gateway(), None).await;

    let failure = watcher
        .matrix()
        .gateway()
        .user_presence(&stranger_id)
        .await
        .expect_err("没有同在一个房间就问不到");
    assert_eq!(
        failure.kind(),
        agent_room_application::ports::MatrixFailureKind::Forbidden
    );
}

/// 接着同步，直到同步里带回这个人的这种在线状态。
async fn synced_presence(
    gateway: &dyn MatrixGateway,
    user_id: &MatrixUserId,
    state: MatrixPresenceState,
    within: Duration,
) -> MatrixUserPresence {
    let deadline = Instant::now() + within;
    let mut since = None;
    loop {
        let batch = sync(gateway, since).await;
        if let Some(found) = batch
            .presence()
            .iter()
            .rev()
            .find(|presence| presence.user_id() == user_id)
            .filter(|presence| presence.state() == state)
        {
            return found.clone();
        }
        assert!(
            Instant::now() < deadline,
            "{within:?} 内同步里没带回 {} 的 {state:?}",
            user_id.as_str()
        );
        since = Some(batch.next_batch().clone());
        sleep(Duration::from_millis(500)).await;
    }
}
