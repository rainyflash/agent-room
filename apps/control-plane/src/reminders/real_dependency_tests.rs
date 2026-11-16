//! 真实 Synapse 上确认：提醒那一轮以应用服务冒充本人读得到他的个人工作区，读得到他在的房间里某一刻
//! 以后的消息，读不到他不在的房间（specs/redesign/reminders.md 的“怎么算”）。只在派发 `suite=all`
//! 的集成作业里跑（`python tools/control-plane.py test`）。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_room_application::ports::{
    MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
    MatrixAgentUserRegistration, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventType,
    MatrixFailureKind, MatrixRoomId, MatrixRoomPreset, MatrixRoomVisibility, MatrixTransactionId,
    MatrixUserId, NetworkAgentMatrixGateway, ReminderMatrixReader, RoomMembershipGateway,
    RoomProvisioningGateway, SecretValue,
};
use agent_room_domain::{ids::AgentId, rooms::MatrixRoomReference, time::UtcMillis};
use agent_room_matrix_provisioning_adapter::{
    MatrixAgentSessionClient, MatrixApplicationServiceProvisioner,
};
use reqwest::Url;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::ControlPlaneConfig;

const WORKSPACE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.personal-workspace.v1";

/// 房间里的一个人：Matrix 用户和一台设备的登录。
struct Member {
    user: MatrixUserId,
    token: SecretValue,
}

#[tokio::test]
#[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
async fn 真实_synapse_上替本人读个人工作区和房间里的新消息() {
    let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");
    let service = crate::real_dependency_identity_provisioner(&config);
    let room = open_room(&service).await;
    let reader = member_of(&service, &room).await;
    let speaker = member_of(&service, &room).await;
    let since = now();

    assert_eq!(
        service
            .personal_workspace(&reader.user)
            .await
            .expect("没写过也读得出"),
        None
    );
    let document = json!({ "schema": 1, "entries": [{
        "kind": "dnd", "id": "global", "value": 0, "clock": 1, "writerId": "real-synapse",
    }] });
    put_workspace(&config, &reader, &document).await;
    assert_eq!(
        service
            .personal_workspace(&reader.user)
            .await
            .expect("替本人读个人工作区"),
        Some(document)
    );

    let agent = session_client(&config);
    let mention = json!({ "preview": { "conversation": { "mentions": [reader.user.as_str()] } } });
    say(&agent, &speaker, &room, "message.preview.v1", mention).await;
    say(&agent, &speaker, &room, "message.preview.v2", json!({})).await;
    say(&agent, &speaker, &room, "message.revision.v1", json!({})).await;
    agent
        .send_event(
            &speaker.token,
            &room,
            &MatrixEvent::new(
                MatrixEventType::new("m.room.encrypted").unwrap(),
                MatrixTransactionId::new(Uuid::now_v7().to_string()).unwrap(),
                json!({ "algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "probe" }),
            )
            .unwrap(),
        )
        .await
        .expect("发一条看起来加密的");

    let plain = service
        .messages_since(&reader.user, &room, false, since)
        .await
        .expect("替本人读房间");
    assert!(!plain.truncated);
    assert_eq!(plain.messages.len(), 2, "只算两条消息预览，修订不算");
    assert!(
        plain
            .messages
            .iter()
            .all(|message| message.sender == speaker.user)
    );
    assert_eq!(
        plain
            .messages
            .iter()
            .filter(|message| message.mentions_reader)
            .count(),
        1
    );
    let sealed = service
        .messages_since(&reader.user, &room, true, since)
        .await
        .expect("替本人读房间");
    assert_eq!(sealed.messages.len(), 1, "加密的房间只看外面那一层");
    assert!(!sealed.messages[0].mentions_reader);

    let elsewhere = open_room(&service).await;
    member_of(&service, &elsewhere).await;
    let refused = service
        .messages_since(&reader.user, &elsewhere, false, since)
        .await
        .expect_err("不在的房间读不了");
    assert_eq!(refused.kind(), MatrixFailureKind::Forbidden);
}

/// 应用服务自己建一间谁都能直接进的房间，像公开大厅那样。
async fn open_room(service: &MatrixApplicationServiceProvisioner) -> MatrixRoomId {
    service
        .create_room(
            &MatrixCreateRoom::new(
                Some("提醒验收".to_owned()),
                None,
                MatrixRoomVisibility::Private,
                MatrixRoomPreset::PublicChat,
                false,
                Vec::new(),
            )
            .unwrap(),
        )
        .await
        .expect("应用服务建房")
}

/// 一个新的受管用户进房间，拿一台设备的登录。
async fn member_of(service: &MatrixApplicationServiceProvisioner, room: &MatrixRoomId) -> Member {
    let user = service
        .ensure_user(&MatrixAgentUserRegistration::new(
            MatrixAgentLocalpart::from_agent_id(AgentId::from_uuid(Uuid::now_v7())),
        ))
        .await
        .expect("注册 Matrix 用户");
    service
        .room_membership(user.clone())
        .expect("受管用户")
        .join(&MatrixRoomReference::new(room.as_str().to_owned()).unwrap())
        .await
        .expect("进房间");
    let session = service
        .issue_device_session(
            &MatrixAgentDeviceSessionRequest::new(
                user.clone(),
                MatrixDeviceId::new(format!("AR_{}", Uuid::now_v7().simple())).unwrap(),
                "提醒验收".to_owned(),
            )
            .unwrap(),
        )
        .await
        .expect("签发设备会话");
    Member {
        user,
        token: SecretValue::new(session.access_token().expose().to_owned()).unwrap(),
    }
}

fn session_client(config: &ControlPlaneConfig) -> MatrixAgentSessionClient {
    MatrixAgentSessionClient::new(
        config.dependencies.matrix_base_url.as_str(),
        Duration::from_secs(10),
    )
    .expect("Matrix 地址有效")
}

async fn say(
    agent: &MatrixAgentSessionClient,
    member: &Member,
    room: &MatrixRoomId,
    kind: &str,
    content: Value,
) {
    agent
        .send_event(
            &member.token,
            room,
            &MatrixEvent::new(
                MatrixEventType::new(format!("io.github.rainyflash.agentroom.{kind}")).unwrap(),
                MatrixTransactionId::new(Uuid::now_v7().to_string()).unwrap(),
                content,
            )
            .unwrap(),
        )
        .await
        .expect("发一条");
}

/// 本人用自己的登录写个人工作区，和网页一样。
async fn put_workspace(config: &ControlPlaneConfig, member: &Member, document: &Value) {
    let mut url = Url::parse(config.dependencies.matrix_base_url.as_str()).unwrap();
    url.path_segments_mut().unwrap().extend([
        "_matrix",
        "client",
        "v3",
        "user",
        member.user.as_str(),
        "account_data",
        WORKSPACE_EVENT_TYPE,
    ]);
    let response = reqwest::Client::new()
        .put(url)
        .bearer_auth(member.token.expose())
        .json(document)
        .send()
        .await
        .expect("写个人工作区");
    assert!(response.status().is_success(), "{}", response.status());
}

fn now() -> UtcMillis {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("时钟在 1970 年以后");
    UtcMillis::new(i64::try_from(elapsed.as_millis()).unwrap() - 1_000).unwrap()
}
