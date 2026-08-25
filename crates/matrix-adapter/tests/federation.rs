use std::{env, time::Duration};

use matrix_sdk::{
    Client, RoomState,
    config::{RequestConfig, SyncSettings, SyncToken},
    deserialized_responses::VerificationState,
    encryption::identities::Device,
    ruma::{
        OwnedDeviceId, OwnedEventId, OwnedUserId, UserId,
        api::client::room::{
            Visibility,
            create_room::v3::{Request as CreateRoomRequest, RoomPreset},
        },
        events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
    },
};
use matrix_sdk_base::crypto::{CollectStrategy, DecryptionSettings, LocalTrust, TrustRequirement};
use serde_json::{Value, json};
use tokio::time::sleep;
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SYNC_TIMEOUT: Duration = Duration::from_millis(500);
const FEDERATION_ATTEMPTS: u16 = 90;

struct Participant {
    client: Client,
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    since: Option<String>,
}

#[tokio::test]
#[ignore = "需要由 tools/federation.py 提供两个真实联邦 Homeserver"]
async fn 双_homeserver_经指纹核验后交换并解密端到端加密事件() {
    let mut alpha = participant(
        "AGENT_ROOM_FEDERATION_ALPHA_URL",
        "AGENT_ROOM_FEDERATION_ALPHA_USER",
        "AGENT_ROOM_FEDERATION_ALPHA_PASSWORD",
    )
    .await;
    let mut beta = participant(
        "AGENT_ROOM_FEDERATION_BETA_URL",
        "AGENT_ROOM_FEDERATION_BETA_USER",
        "AGENT_ROOM_FEDERATION_BETA_PASSWORD",
    )
    .await;

    sync(&mut alpha).await;
    sync(&mut beta).await;
    let room_id = create_encrypted_room(&alpha.client).await;
    alpha
        .client
        .get_room(&room_id)
        .expect("创建端必须立即持有房间")
        .invite_user_by_id(&beta.user_id)
        .await
        .expect("跨服邀请必须成功");
    wait_for_room_state(&mut beta, &room_id, RoomState::Invited).await;
    beta.client
        .join_room_by_id(&room_id)
        .await
        .expect("跨服加入必须成功");
    wait_for_room_state(&mut beta, &room_id, RoomState::Joined).await;
    wait_for_room_state(&mut alpha, &room_id, RoomState::Joined).await;

    // OnlyTrustedDevices 不会向未知设备泄露 Megolm 会话。测试通过独立客户端持有的
    // Ed25519/Curve25519 公钥做带外比对，完全一致后才在双方本地建立信任。
    establish_mutual_fingerprint_trust(&alpha, &beta).await;

    let transaction_id = format!("federation_{}", Uuid::now_v7().simple());
    let response = alpha
        .client
        .get_room(&room_id)
        .expect("发送端房间仍存在")
        .send_raw(
            "org.agentroom.message.preview.v1",
            json!({
                "schemaVersion": "1.0",
                "body": "跨服务端端到端加密预览",
            }),
        )
        .with_transaction_id(transaction_id.as_str().into())
        .await
        .expect("已核验设备必须能接收 Megolm 会话");

    let (event, verification) =
        wait_for_decrypted_event(&mut beta, &response.response.event_id).await;
    assert_eq!(event["type"], "org.agentroom.message.preview.v1");
    assert_eq!(event["content"]["body"], "跨服务端端到端加密预览");
    assert!(matches!(verification, VerificationState::Unverified(_)));

    beta.client
        .get_room(&room_id)
        .expect("接收端房间仍存在")
        .leave()
        .await
        .expect("接收端离房成功");
    alpha
        .client
        .get_room(&room_id)
        .expect("发送端房间仍存在")
        .leave()
        .await
        .expect("发送端离房成功");
}

async fn participant(url_name: &str, user_name: &str, password_name: &str) -> Participant {
    let user = required_environment(user_name);
    let password = required_environment(password_name);
    let client = Client::builder()
        .homeserver_url(required_environment(url_name))
        .request_config(
            RequestConfig::new()
                .disable_retry()
                .timeout(REQUEST_TIMEOUT),
        )
        .with_room_key_recipient_strategy(CollectStrategy::OnlyTrustedDevices)
        .with_decryption_settings(DecryptionSettings {
            sender_device_trust_requirement: TrustRequirement::Untrusted,
        })
        .build()
        .await
        .expect("联邦 Matrix 客户端必须能初始化");
    client
        .matrix_auth()
        .login_username(user, &password)
        .initial_device_display_name("Agent Room 联邦验收")
        .send()
        .await
        .expect("联邦测试用户必须能登录");
    Participant {
        user_id: client.user_id().expect("登录后必须有用户标识").to_owned(),
        device_id: client.device_id().expect("登录后必须有设备标识").to_owned(),
        client,
        since: None,
    }
}

async fn create_encrypted_room(client: &Client) -> matrix_sdk::ruma::OwnedRoomId {
    let mut request = CreateRoomRequest::new();
    request.name = Some(format!("Agent Room 联邦 E2EE {}", Uuid::now_v7().simple()));
    request.topic = Some("双 Homeserver 端到端加密验收".to_owned());
    request.visibility = Visibility::Private;
    request.preset = Some(RoomPreset::PrivateChat);
    request.initial_state.push(
        InitialStateEvent::with_empty_state_key(
            RoomEncryptionEventContent::with_recommended_defaults(),
        )
        .to_raw_any(),
    );
    client
        .create_room(request)
        .await
        .expect("必须能建立端到端加密房间")
        .room_id()
        .to_owned()
}

async fn wait_for_room_state(
    participant: &mut Participant,
    room_id: &matrix_sdk::ruma::RoomId,
    expected: RoomState,
) {
    for _ in 0..FEDERATION_ATTEMPTS {
        sync(participant).await;
        if participant
            .client
            .get_room(room_id)
            .is_some_and(|room| room.state() == expected)
        {
            return;
        }
        sleep(Duration::from_millis(250)).await;
    }
    panic!("房间 {room_id} 未在预算内进入 {expected:?} 状态")
}

async fn establish_mutual_fingerprint_trust(alpha: &Participant, beta: &Participant) {
    let alpha_own = own_device(alpha).await;
    let beta_own = own_device(beta).await;
    let alpha_view_of_beta = remote_device(alpha, &beta.user_id, &beta.device_id).await;
    let beta_view_of_alpha = remote_device(beta, &alpha.user_id, &alpha.device_id).await;

    assert_eq!(
        alpha_view_of_beta.ed25519_key(),
        beta_own.ed25519_key(),
        "Alpha 观察到的 Beta 签名指纹必须与 Beta 自持指纹一致"
    );
    assert_eq!(
        alpha_view_of_beta.curve25519_key(),
        beta_own.curve25519_key(),
        "Alpha 观察到的 Beta 加密指纹必须与 Beta 自持指纹一致"
    );
    assert_eq!(
        beta_view_of_alpha.ed25519_key(),
        alpha_own.ed25519_key(),
        "Beta 观察到的 Alpha 签名指纹必须与 Alpha 自持指纹一致"
    );
    assert_eq!(
        beta_view_of_alpha.curve25519_key(),
        alpha_own.curve25519_key(),
        "Beta 观察到的 Alpha 加密指纹必须与 Alpha 自持指纹一致"
    );
    alpha_view_of_beta
        .set_local_trust(LocalTrust::Verified)
        .await
        .expect("Alpha 必须能持久化已核验的 Beta 指纹");
    beta_view_of_alpha
        .set_local_trust(LocalTrust::Verified)
        .await
        .expect("Beta 必须能持久化已核验的 Alpha 指纹");
}

async fn own_device(participant: &Participant) -> Device {
    participant
        .client
        .encryption()
        .get_own_device()
        .await
        .expect("必须能读取本机密码学设备")
        .expect("本机密码学设备必须存在")
}

async fn remote_device(
    participant: &Participant,
    user_id: &UserId,
    device_id: &matrix_sdk::ruma::DeviceId,
) -> Device {
    participant
        .client
        .encryption()
        .get_device(user_id, device_id)
        .await
        .expect("必须能读取远端密码学设备")
        .expect("远端密码学设备必须已通过同步发现")
}

async fn wait_for_decrypted_event(
    participant: &mut Participant,
    event_id: &OwnedEventId,
) -> (Value, VerificationState) {
    for _ in 0..FEDERATION_ATTEMPTS {
        let response = sync_response(participant).await;
        for event in response
            .rooms
            .joined
            .values()
            .flat_map(|room| &room.timeline.events)
        {
            let value: Value =
                serde_json::from_str(event.raw().json().get()).expect("解密事件必须是 JSON");
            if value["event_id"].as_str() != Some(event_id.as_str()) {
                continue;
            }
            let verification = event
                .encryption_info()
                .expect("匹配事件必须携带 Matrix E2EE 元数据")
                .verification_state
                .clone();
            return (value, verification);
        }
        sleep(Duration::from_millis(250)).await;
    }
    panic!("接收端未在预算内解密事件 {event_id}")
}

async fn sync(participant: &mut Participant) {
    let _ = sync_response(participant).await;
}

async fn sync_response(participant: &mut Participant) -> matrix_sdk::sync::SyncResponse {
    let token = participant
        .since
        .as_ref()
        .map_or(SyncToken::NoToken, |value| {
            SyncToken::Specific(value.clone())
        });
    let response = participant
        .client
        .sync_once(SyncSettings::new().token(token).timeout(SYNC_TIMEOUT))
        .await
        .expect("联邦同步必须成功");
    participant.since = Some(response.next_batch.clone());
    response
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少联邦测试配置 {name}"))
}
