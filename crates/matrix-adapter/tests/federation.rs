use std::{env, num::NonZeroU16, time::Duration};

use agent_room_application::ports::{
    MatrixClientFactory, MatrixConnection, MatrixCreateRoom, MatrixEvent, MatrixEventType,
    MatrixGateway, MatrixLogin, MatrixRoomId, MatrixRoomPreset, MatrixRoomSyncKind,
    MatrixRoomVisibility, MatrixSyncRequest, MatrixSyncToken, MatrixTransactionId, MatrixUserId,
    SecretValue,
};
use agent_room_domain::time::DurationMillis;
use agent_room_matrix_adapter::{MatrixSdkClientFactory, MatrixSdkConfiguration};
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SYNC_TIMEOUT_MILLIS: u64 = 500;
const FEDERATION_ATTEMPTS: u16 = 90;

#[tokio::test]
#[ignore = "需要由 tools/federation.py 提供两个真实联邦 Homeserver"]
async fn 双_homeserver_通过_sdk_交换并解密端到端加密事件() {
    let alpha = login(
        "AGENT_ROOM_FEDERATION_ALPHA_URL",
        "AGENT_ROOM_FEDERATION_ALPHA_USER",
        "AGENT_ROOM_FEDERATION_ALPHA_PASSWORD",
    )
    .await;
    let beta = login(
        "AGENT_ROOM_FEDERATION_BETA_URL",
        "AGENT_ROOM_FEDERATION_BETA_USER",
        "AGENT_ROOM_FEDERATION_BETA_PASSWORD",
    )
    .await;

    let alpha_since = Some(sync(alpha.gateway(), None).await);
    let mut beta_since = Some(sync(beta.gateway(), None).await);
    let room_id = create_encrypted_room(alpha.gateway()).await;
    invite(
        alpha.gateway(),
        &room_id,
        beta.session().metadata().user_id(),
    )
    .await;

    beta_since = Some(
        wait_for_membership(
            beta.gateway(),
            &room_id,
            MatrixRoomSyncKind::Invited,
            beta_since,
        )
        .await,
    );
    join(beta.gateway(), &room_id).await;
    beta_since = Some(
        wait_for_membership(
            beta.gateway(),
            &room_id,
            MatrixRoomSyncKind::Joined,
            beta_since,
        )
        .await,
    );
    let alpha_joined_since = wait_for_membership(
        alpha.gateway(),
        &room_id,
        MatrixRoomSyncKind::Joined,
        alpha_since,
    )
    .await;

    // 两端都完成一次入房后的同步，确保设备密钥与成员列表已进入 SDK 加密机。
    beta_since = Some(sync(beta.gateway(), beta_since).await);
    let _ = sync(alpha.gateway(), Some(alpha_joined_since)).await;

    let event = MatrixEvent::new(
        MatrixEventType::new("org.agentroom.message.preview.v1").expect("事件类型有效"),
        MatrixTransactionId::new(format!("federation_{}", Uuid::now_v7().simple()))
            .expect("事务标识有效"),
        json!({
            "schemaVersion": "1.0",
            "body": "跨服务端到端加密预览",
        }),
    )
    .expect("消息事件有效");
    let accepted = alpha
        .gateway()
        .send_event(&room_id, &event)
        .await
        .expect("发送端必须接受加密房间事件");

    let received = wait_for_event(beta.gateway(), &room_id, accepted.event_id(), beta_since).await;
    assert_eq!(received.content()["body"], "跨服务端到端加密预览");
    assert!(received.end_to_end_encrypted());
    assert!(
        !received.end_to_end_sender_trusted(),
        "未执行 SAS/交叉签名前不得把远端设备抬高为可信"
    );

    beta.gateway()
        .leave(&room_id)
        .await
        .expect("接收端离房成功");
    alpha
        .gateway()
        .leave(&room_id)
        .await
        .expect("发送端离房成功");
}

async fn login(url_name: &str, user_name: &str, password_name: &str) -> MatrixConnection {
    let url = required_environment(url_name);
    let user = required_environment(user_name);
    let password = required_environment(password_name);
    let configuration = MatrixSdkConfiguration::new(&url, REQUEST_TIMEOUT)
        .expect("联邦 Homeserver 地址有效")
        .with_sync_timeline_limit(NonZeroU16::new(20).expect("时间线上限非零"))
        .expect("时间线上限有效");
    MatrixSdkClientFactory::new(configuration)
        .login(
            &MatrixLogin::new(
                user,
                SecretValue::new(password).expect("测试密码有效"),
                None,
                Some("Agent Room 联邦验收".to_owned()),
            )
            .expect("登录参数有效"),
        )
        .await
        .expect("联邦测试用户必须能登录")
}

async fn create_encrypted_room(gateway: &dyn MatrixGateway) -> MatrixRoomId {
    let request = MatrixCreateRoom::new(
        Some(format!("Agent Room 联邦 E2EE {}", Uuid::now_v7().simple())),
        Some("双 Homeserver SDK 端到端加密验收".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        Vec::new(),
    )
    .expect("建房参数有效")
    .with_end_to_end_encryption();
    gateway
        .create_room(&request)
        .await
        .expect("必须能建立端到端加密房间")
}

async fn invite(gateway: &dyn MatrixGateway, room_id: &MatrixRoomId, user_id: &MatrixUserId) {
    for attempt in 1..=FEDERATION_ATTEMPTS {
        match gateway.invite(room_id, user_id).await {
            Ok(()) => return,
            Err(error) if attempt < FEDERATION_ATTEMPTS => {
                let _ = error;
                sleep(Duration::from_secs(1)).await;
            }
            Err(error) => panic!("跨服邀请重试耗尽：{error:?}"),
        }
    }
}

async fn join(gateway: &dyn MatrixGateway, room_id: &MatrixRoomId) {
    for attempt in 1..=FEDERATION_ATTEMPTS {
        match gateway.join(room_id).await {
            Ok(()) => return,
            Err(error) if attempt < FEDERATION_ATTEMPTS => {
                let _ = error;
                sleep(Duration::from_secs(1)).await;
            }
            Err(error) => panic!("跨服加入重试耗尽：{error:?}"),
        }
    }
}

async fn wait_for_membership(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    expected: MatrixRoomSyncKind,
    mut since: Option<MatrixSyncToken>,
) -> MatrixSyncToken {
    for _ in 0..FEDERATION_ATTEMPTS {
        let batch = sync_batch(gateway, since).await;
        if batch
            .rooms()
            .iter()
            .any(|room| room.room_id() == room_id && room.kind() == expected)
        {
            return batch.next_batch().clone();
        }
        since = Some(batch.next_batch().clone());
        sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "联邦同步始终缺少房间 {} 的 {expected:?} 状态",
        room_id.as_str()
    )
}

async fn wait_for_event(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    event_id: &agent_room_application::ports::MatrixEventId,
    mut since: Option<MatrixSyncToken>,
) -> agent_room_application::ports::MatrixTimelineEvent {
    for _ in 0..FEDERATION_ATTEMPTS {
        let batch = sync_batch(gateway, since).await;
        if let Some(event) = batch
            .rooms()
            .iter()
            .filter(|room| room.room_id() == room_id)
            .flat_map(agent_room_application::ports::MatrixRoomSync::timeline)
            .find(|event| event.event_id() == Some(event_id))
        {
            return event.clone();
        }
        since = Some(batch.next_batch().clone());
        sleep(Duration::from_millis(250)).await;
    }
    panic!("接收端未在预算内解密事件 {}", event_id.as_str())
}

async fn sync(gateway: &dyn MatrixGateway, since: Option<MatrixSyncToken>) -> MatrixSyncToken {
    sync_batch(gateway, since).await.next_batch().clone()
}

async fn sync_batch(
    gateway: &dyn MatrixGateway,
    since: Option<MatrixSyncToken>,
) -> agent_room_application::ports::MatrixSyncBatch {
    gateway
        .sync_once(
            &MatrixSyncRequest::new(
                since,
                DurationMillis::new(SYNC_TIMEOUT_MILLIS).expect("同步超时有效"),
                false,
            )
            .expect("同步请求有效"),
        )
        .await
        .expect("联邦同步必须成功")
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少联邦测试配置 {name}"))
}
