//! 真实 Synapse 上确认：建公开大厅的应用服务账号不冒充任何人，就读得到大厅里最近的消息和状态
//! （specs/public-lobby-watch/design.md 第 1 步），也问得到大厅里 Agent 的在线状态
//! （specs/agent-liveness/design.md 第 2 步）。只在派发 `suite=all` 的集成作业里跑
//! （`python tools/control-plane.py test`）。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
    MatrixAgentUserRegistration, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventType,
    MatrixRoomId, MatrixRoomPreset, MatrixRoomVisibility, MatrixStateEvent, MatrixStateKey,
    MatrixTransactionId, MatrixUserId, NetworkAgentMatrixGateway, PublicLobbyMatrixReader,
    RoomMembershipGateway, RoomProvisioningGateway,
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState, ids::AgentId, rooms::MatrixRoomReference,
};
use agent_room_matrix_provisioning_adapter::{
    MatrixAgentSessionClient, MatrixApplicationServiceProvisioner,
};
use serde_json::json;
use uuid::Uuid;

use crate::config::ControlPlaneConfig;

const PREVIEW_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.message.preview.v1";
const STATUS_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.agent.status.v1";

#[tokio::test]
#[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
async fn 真实_synapse_上建大厅的应用服务账号读得到最近的消息和房间状态() {
    let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");
    let service = crate::build_matrix_identity_provisioner(&config, config.dependencies.timeout)
        .expect("Application Service 配置有效");
    // 像公开大厅那样：应用服务自己建房（不带 user_id），在线状态允许成员自己写。
    let room = service
        .create_room(
            &MatrixCreateRoom::new(
                Some("围观验收大厅".to_owned()),
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
    let user = agent_speaks_in(&config, &service, &room).await;

    let events = service
        .recent_messages(&room, 60)
        .await
        .expect("应用服务账号读得到最近的消息");
    assert!(
        events
            .iter()
            .any(|event| event.sender() == Some(&user) && event.content()["probe"] == "围观"),
        "读到 Agent 刚说的那句"
    );
    let state = service
        .current_state(&room)
        .await
        .expect("应用服务账号读得到房间状态");
    let creator = state
        .iter()
        .find(|event| event.event_type().as_str() == "m.room.create")
        .and_then(|event| event.sender())
        .expect("有建房事件");
    assert_ne!(creator, &user, "大厅是应用服务账号建的");
    assert!(
        state.iter().any(|event| {
            event.event_type().as_str() == "m.room.member"
                && event.state_key() == Some(user.as_str())
                && event.content()["membership"] == "join"
        }),
        "看得到 Agent 在大厅里"
    );
    assert!(
        state.iter().any(|event| {
            event.event_type().as_str() == STATUS_EVENT_TYPE && event.sender() == Some(&user)
        }),
        "看得到 Agent 的在线状态"
    );
    let presence = service
        .user_presence(&user)
        .await
        .expect("应用服务账号问得到大厅里 Agent 的 Matrix 在线状态");
    assert_eq!(presence.state(), MatrixPresenceState::Online);
}

/// 一个新的 Agent 进大厅，说一句，写一条在线状态，报一次 Matrix 在线；交回它的 Matrix 用户。
async fn agent_speaks_in(
    config: &ControlPlaneConfig,
    service: &MatrixApplicationServiceProvisioner,
    room: &MatrixRoomId,
) -> MatrixUserId {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let user = service
        .ensure_user(&MatrixAgentUserRegistration::new(
            MatrixAgentLocalpart::from_agent_id(agent_id),
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
                "围观验收".to_owned(),
            )
            .unwrap(),
        )
        .await
        .expect("签发设备会话");
    let agent = MatrixAgentSessionClient::new(
        config.dependencies.matrix_base_url.as_str(),
        Duration::from_secs(10),
    )
    .expect("Matrix 地址有效");
    agent
        .send_event(
            session.access_token(),
            room,
            &MatrixEvent::new(
                MatrixEventType::new(PREVIEW_EVENT_TYPE).unwrap(),
                MatrixTransactionId::new(Uuid::now_v7().to_string()).unwrap(),
                json!({"probe": "围观"}),
            )
            .unwrap(),
        )
        .await
        .expect("Agent 发言");
    agent
        .send_state_event(
            session.access_token(),
            room,
            &MatrixStateEvent::new(
                MatrixEventType::new(STATUS_EVENT_TYPE).unwrap(),
                MatrixStateKey::new(Uuid::now_v7().to_string()).unwrap(),
                json!({"probe": true}),
            )
            .unwrap(),
        )
        .await
        .expect("Agent 写在线状态");
    agent
        .report_presence(session.access_token(), &user, MatrixPresenceState::Online)
        .await
        .expect("Agent 报在线");
    user
}
