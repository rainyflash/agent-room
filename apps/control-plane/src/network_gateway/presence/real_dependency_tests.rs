//! 真实 Synapse 上确认网关报网络 Agent 在线状态靠的几件事（`specs/agent-liveness/design.md`
//! 第 3 步）：轻量客户端报的、读回来的对得上，读得到房间里自己的名片；网关不同步的时候，只靠
//! 每 20 秒报一次就撑得过 Synapse 30 秒的离线判定，不报了才变离线；同步带的在线状态算数。
//! 只在派发 `suite=all` 的集成作业里跑（`python tools/control-plane.py test`）。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
    MatrixAgentUserRegistration, MatrixCreateRoom, MatrixDeviceId, MatrixEventType,
    MatrixFailureKind, MatrixRoomPreset, MatrixRoomVisibility, MatrixStateEvent, MatrixStateKey,
    MatrixUserId, NetworkAgentMatrixGateway, NetworkAgentSyncRequest, PublicLobbyMatrixReader,
    RoomMembershipGateway, RoomProvisioningGateway, SecretValue,
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState::{self, Offline, Online, Unavailable},
    ids::AgentId,
    rooms::MatrixRoomReference,
};
use agent_room_matrix_provisioning_adapter::{
    MatrixAgentSessionClient, MatrixApplicationServiceProvisioner,
};
use serde_json::json;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

use super::HEARTBEAT;
use crate::config::ControlPlaneConfig;

const STATUS_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.agent.status.v1";
/// 服务器约 30 秒没动静就改成离线，再加上它每隔几秒才检查一次。
const OFFLINE_WITHIN: Duration = Duration::from_secs(90);

#[tokio::test]
#[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
async fn 真实_synapse_上网络_agent_只靠定时报在线状态撑着_不报才变离线_同步带的算数() {
    let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");
    let service = crate::build_matrix_identity_provisioner(&config, config.dependencies.timeout)
        .expect("Application Service 配置有效");
    let room = service
        .create_room(
            &MatrixCreateRoom::new(
                Some("在线状态验收大厅".to_owned()),
                None,
                MatrixRoomVisibility::Private,
                MatrixRoomPreset::PublicChat,
                false,
                Vec::new(),
            )
            .unwrap()
            .with_member_writable_state_event_type(
                MatrixEventType::new(STATUS_EVENT_TYPE).unwrap(),
            ),
        )
        .await
        .expect("应用服务建房");
    let (user, token) = agent_in(&service, &room).await;
    let agent = MatrixAgentSessionClient::new(
        config.dependencies.matrix_base_url.as_str(),
        Duration::from_secs(10),
    )
    .expect("Matrix 地址有效");

    // 读房间里自己的那条状态：还没写时没有，写了读得回。
    let event_type = MatrixEventType::new(STATUS_EVENT_TYPE).unwrap();
    let state_key = MatrixStateKey::new(Uuid::now_v7().to_string()).unwrap();
    assert_eq!(
        agent
            .state_event(&token, &room, &event_type, &state_key)
            .await
            .expect("读得了房间状态"),
        None
    );
    agent
        .send_state_event(
            &token,
            &room,
            &MatrixStateEvent::new(
                event_type.clone(),
                state_key.clone(),
                json!({"liveness": "presence"}),
            )
            .unwrap(),
        )
        .await
        .expect("写名片");
    assert_eq!(
        agent
            .state_event(&token, &room, &event_type, &state_key)
            .await
            .expect("读得了房间状态"),
        Some(json!({"liveness": "presence"}))
    );

    // 报离开，读回自己的、别人问到的都是离开。
    report(&agent, &token, &user, Unavailable).await;
    assert_eq!(
        agent.own_presence(&token, &user).await.expect("读回自己的"),
        Unavailable
    );
    assert_eq!(asked(&service, &user).await, Unavailable);

    // 不同步，只每 20 秒报一次：每次报之前（离上一次最久的时候）看一眼，过了 Synapse
    // 30 秒的离线判定也还是离开。
    for _ in 0..2 {
        sleep(HEARTBEAT).await;
        assert_eq!(
            asked(&service, &user).await,
            Unavailable,
            "隔 {HEARTBEAT:?} 报一次撑得住"
        );
        report(&agent, &token, &user, Unavailable).await;
    }

    // 不报了：约 30 秒后变离线。
    let deadline = Instant::now() + OFFLINE_WITHIN;
    while asked(&service, &user).await != Offline {
        assert!(Instant::now() < deadline, "{OFFLINE_WITHIN:?} 内没变离线");
        sleep(Duration::from_secs(2)).await;
    }

    // 等消息的同步带在线：算数。
    agent
        .sync(
            &token,
            &NetworkAgentSyncRequest {
                since: None,
                timeout_millis: 0,
                timeline_limit: 1,
                presence: Online,
            },
        )
        .await
        .expect("同步");
    assert_eq!(asked(&service, &user).await, Online);
}

/// 一个新的网络 Agent 进大厅，交回它的 Matrix 用户和访问令牌。
async fn agent_in(
    service: &MatrixApplicationServiceProvisioner,
    room: &agent_room_application::ports::MatrixRoomId,
) -> (MatrixUserId, SecretValue) {
    let user = service
        .ensure_user(&MatrixAgentUserRegistration::new(
            MatrixAgentLocalpart::from_agent_id(AgentId::from_uuid(Uuid::now_v7())),
        ))
        .await
        .expect("注册 Agent 的 Matrix 用户");
    service
        .room_membership(user.clone())
        .expect("受管 Agent 用户")
        .join(&MatrixRoomReference::new(room.as_str().to_owned()).unwrap())
        .await
        .expect("Agent 进大厅");
    let session = service
        .issue_device_session(
            &MatrixAgentDeviceSessionRequest::new(
                user.clone(),
                MatrixDeviceId::new(format!("AR_{}", Uuid::now_v7().simple())).unwrap(),
                "在线状态验收".to_owned(),
            )
            .unwrap(),
        )
        .await
        .expect("签发设备会话");
    (user, session.access_token().clone())
}

/// 报一次在线状态。Synapse 每个用户 10 秒只认一次，被限速就按它说的等一会儿再报。
async fn report(
    agent: &MatrixAgentSessionClient,
    token: &SecretValue,
    user: &MatrixUserId,
    presence: MatrixPresenceState,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match agent.report_presence(token, user, presence).await {
            Ok(()) => return,
            Err(failure) if failure.kind() == MatrixFailureKind::RateLimited => {
                assert!(Instant::now() < deadline, "一直被限速");
                let wait = failure
                    .retry_after()
                    .map_or(Duration::from_secs(1), |wait| {
                        Duration::from_millis(wait.value())
                    });
                sleep(wait).await;
            }
            Err(failure) => panic!("报在线状态失败：{failure:?}"),
        }
    }
}

/// 建大厅的应用服务账号问到的这个 Agent 的在线状态，和围观页、别人看到的一样。
async fn asked(
    service: &MatrixApplicationServiceProvisioner,
    user: &MatrixUserId,
) -> MatrixPresenceState {
    service
        .user_presence(user)
        .await
        .expect("应用服务账号问得到大厅里 Agent 的在线状态")
        .state()
}
