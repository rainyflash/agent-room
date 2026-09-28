//! 缺房间密钥的一方请 Agent 重发它自己建的会话（设计见 `specs/room-key-recovery/design.md`）。
//!
//! 人这边用普通的 matrix-sdk 客户端扮演：它在 Agent 发消息时还没建立加密身份，房间密钥被扣下；
//! 建好身份后经 Olm 请 Agent 重发，导入后解开这条旧消息。Agent 由 `restore_with_handoffs` 打开，
//! 和本机 Bridge、网络 Agent 网关一样挂着应答者。

use agent_room_protocol_conformance::generated::RoomKeysEvent;
use matrix_sdk::{
    Client, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession as SdkMatrixSession,
    config::{RequestConfig, SyncSettings},
    deserialized_responses::{AlgorithmInfo, EncryptionInfo, TimelineEventKind},
    ruma::{
        OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedUserId,
        events::{AnyToDeviceEvent, AnyToDeviceEventContent},
        serde::Raw,
    },
    store::RoomLoadSettings,
};
use matrix_sdk_base::crypto::{CollectStrategy, encrypt_room_key_export, olm::ExportedRoomKey};
use tokio::sync::mpsc;

use super::*;

const ROOM_KEY_REQUEST: &str = "io.github.rainyflash.agentroom.room_keys.request.v1";
const ROOM_KEYS: &str = "io.github.rainyflash.agentroom.room_keys.v1";

type ReceivedToDevice = (Raw<AnyToDeviceEvent>, Option<EncryptionInfo>);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 缺密钥的一方请_agent_重发后解开签名之前的消息() {
    timeout(Duration::from_mins(5), exercise_room_key_recovery())
        .await
        .expect("重发房间密钥的场景必须在预算内完成");
}

async fn exercise_room_key_recovery() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let agent = managed_device(&provisioner, &factory).await;
    establish_identity(&agent).await;
    let (human_user, human, mut received) = unsigned_human(&provisioner, &base_url).await;
    let agent_user = agent.matrix().session().metadata().user_id().clone();
    let room_id = PrivateRoomMatrixProvisioner::create(
        &provisioner,
        &private_room_creation(&agent_user, &human_user),
    )
    .await
    .expect("Application Service 必须能创建私人房间");
    let sdk_room_id = OwnedRoomId::try_from(room_id.as_str()).expect("房间标识有效");
    join_with_retry(agent.matrix().gateway(), &room_id).await;
    human
        .join_room_by_id(&sdk_room_id)
        .await
        .expect("人可以加入房间");
    provisioner
        .set_speaking_batch(
            &room_id,
            &[PrivateMatrixSpeakingAssignment::new(agent_user, true)],
        )
        .await
        .expect("Agent 应获得发言能力");
    human
        .sync_once(settings())
        .await
        .expect("人的设备上传设备密钥");
    sync_until_room(
        agent.matrix().gateway(),
        &room_id,
        MatrixRoomSyncKind::Joined,
    )
    .await;
    // 人的设备还没有由主人签名，Agent 发这条消息时扣下房间密钥。
    agent
        .security_gateway_handle()
        .ensure_room_ready(&room_id)
        .await
        .expect("Agent 可以发送");
    let sent = send_with_retry(
        agent.matrix().gateway(),
        &room_id,
        &message_event(unique_value("withheld"), "人签名之前的消息"),
    )
    .await;
    human
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
        .expect("人可以建立加密身份");
    human.sync_once(settings()).await.expect("人同步到这条消息");
    let event_id = OwnedEventId::try_from(sent.event_id().as_str()).expect("事件标识有效");
    let room = human.get_room(&sdk_room_id).expect("人已经在房间里");
    let session_id = undecryptable_session(&room, &event_id).await;

    request_keys(&human, &agent, &sdk_room_id, &session_id).await;
    let response = await_keys(&agent, &human, &mut received).await;
    import_keys(&human, &sdk_room_id, &response).await;

    let event = room
        .event(&event_id, None)
        .await
        .expect("人可以取回这条消息");
    assert!(
        matches!(event.kind, TimelineEventKind::Decrypted(_)),
        "导入 Agent 重发的密钥后，签名之前的消息必须解得开"
    );
    assert_eq!(
        event
            .raw()
            .get_field::<String>("type")
            .expect("事件类型可读")
            .as_deref(),
        Some("io.github.rainyflash.agentroom.message.preview.v1")
    );
}

fn settings() -> SyncSettings {
    SyncSettings::default().timeout(Duration::from_millis(TEST_SYNC_TIMEOUT_MILLIS))
}

/// 人用普通的 matrix-sdk 客户端，先不建加密身份。收到的重发应答放进通道里。
async fn unsigned_human(
    provisioner: &MatrixApplicationServiceProvisioner,
    base_url: &str,
) -> (
    MatrixUserId,
    Client,
    mpsc::UnboundedReceiver<ReceivedToDevice>,
) {
    let user_id = provisioner
        .ensure_user(&MatrixAgentUserRegistration::new(
            MatrixAgentLocalpart::from_agent_id(AgentId::from_uuid(Uuid::now_v7())),
        ))
        .await
        .expect("可建立隔离的验收用户");
    let request = MatrixAgentDeviceSessionRequest::new(
        user_id.clone(),
        MatrixDeviceId::new(format!("AR_{}", Uuid::now_v7().simple())).expect("设备标识有效"),
        "重发密钥验收的人".to_owned(),
    )
    .expect("验收设备会话请求有效");
    let session = provisioner
        .issue_device_session(&request)
        .await
        .expect("可签发验收会话");
    let client = Client::builder()
        .homeserver_url(base_url)
        .request_config(
            RequestConfig::new()
                .disable_retry()
                .timeout(TEST_REQUEST_TIMEOUT),
        )
        .build()
        .await
        .expect("人的客户端可以初始化");
    client
        .matrix_auth()
        .restore_session(
            SdkMatrixSession {
                meta: SessionMeta {
                    user_id: OwnedUserId::try_from(user_id.as_str()).expect("用户标识有效"),
                    device_id: OwnedDeviceId::from(session.metadata().device_id().as_str()),
                },
                tokens: SessionTokens {
                    access_token: session.access_token().expose().to_owned(),
                    refresh_token: None,
                },
            },
            RoomLoadSettings::default(),
        )
        .await
        .expect("人的会话可以恢复");
    let (sender, receiver) = mpsc::unbounded_channel();
    client.add_event_handler(
        move |raw: Raw<AnyToDeviceEvent>, encryption_info: Option<EncryptionInfo>| {
            let sender = sender.clone();
            async move {
                if raw.get_field::<String>("type").ok().flatten().as_deref() == Some(ROOM_KEYS) {
                    let _ = sender.send((raw, encryption_info));
                }
            }
        },
    );
    (user_id, client, receiver)
}

/// 人这边解不开这条消息，并从加密内容里读出会话标识。
async fn undecryptable_session(room: &matrix_sdk::Room, event_id: &OwnedEventId) -> String {
    let event = room
        .event(event_id, None)
        .await
        .expect("人可以取到这条消息");
    assert!(
        matches!(event.kind, TimelineEventKind::UnableToDecrypt { .. }),
        "Agent 扣下了房间密钥，人这边本来解不开"
    );
    event
        .raw()
        .get_field::<serde_json::Value>("content")
        .ok()
        .flatten()
        .and_then(|content| content.get("session_id")?.as_str().map(str::to_owned))
        .expect("加密内容必须带会话标识")
}

async fn request_keys(
    human: &Client,
    agent: &MatrixSdkHandoffConnection,
    room_id: &OwnedRoomId,
    session_id: &str,
) {
    let metadata = agent.matrix().session().metadata();
    let agent_user = OwnedUserId::try_from(metadata.user_id().as_str()).expect("用户标识有效");
    let agent_device = OwnedDeviceId::from(metadata.device_id().as_str());
    let encryption = human.encryption();
    encryption
        .request_user_identity(&agent_user)
        .await
        .expect("人可以取到 Agent 的加密身份");
    let device = encryption
        .get_device(&agent_user, &agent_device)
        .await
        .expect("人可以读 Agent 的设备")
        .expect("人认识 Agent 的设备");
    let content = Raw::new(&json!({
        "schemaVersion": "1.0",
        "eventType": ROOM_KEY_REQUEST,
        "id": Uuid::now_v7().to_string(),
        "createdAt": "2026-09-28T14:00:00.000Z",
        "roomId": room_id.as_str(),
        "sessionIds": [session_id],
    }))
    .expect("请求可以编码")
    .cast_unchecked::<AnyToDeviceEventContent>();
    let failures = encryption
        .encrypt_and_send_raw_to_device(
            vec![&device],
            ROOM_KEY_REQUEST,
            content,
            CollectStrategy::AllDevices,
        )
        .await
        .expect("请求可以发出");
    assert!(failures.is_empty(), "请求必须送到 Agent 的设备");
}

/// Agent 同步时收到请求，应答在后台任务里导出、发回；人同步到应答为止。
async fn await_keys(
    agent: &MatrixSdkHandoffConnection,
    human: &Client,
    received: &mut mpsc::UnboundedReceiver<ReceivedToDevice>,
) -> RoomKeysEvent {
    let mut since = None;
    for _ in 0..90 {
        let batch = sync(agent.matrix().gateway(), since).await;
        since = Some(batch.next_batch().clone());
        human.sync_once(settings()).await.expect("人可以同步");
        if let Ok((raw, encryption_info)) = received.try_recv() {
            let encryption_info = encryption_info.expect("应答必须经 Olm 加密送达");
            assert!(matches!(
                encryption_info.algorithm_info,
                AlgorithmInfo::OlmV1Curve25519AesSha2 { .. }
            ));
            return raw
                .get_field::<RoomKeysEvent>("content")
                .expect("应答可以解析")
                .expect("应答必须带内容");
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!("Agent 始终没有重发房间密钥");
}

/// 和网页端一样：把应答拼回 Matrix 的房间密钥导出格式再导入。
async fn import_keys(human: &Client, room_id: &OwnedRoomId, response: &RoomKeysEvent) {
    assert_eq!(response.room_id, room_id.as_str());
    let keys = response
        .keys
        .iter()
        .map(|key| {
            serde_json::from_value::<ExportedRoomKey>(json!({
                "algorithm": "m.megolm.v1.aes-sha2",
                "room_id": response.room_id,
                "sender_key": response.sender_key,
                "session_id": key.session_id,
                "session_key": key.session_key,
                "sender_claimed_keys": { "ed25519": response.sender_ed25519_key },
                "forwarding_curve25519_key_chain": [],
            }))
            .expect("应答里的密钥能拼回导出格式")
        })
        .collect::<Vec<_>>();
    let passphrase = "room key recovery acceptance";
    let export = encrypt_room_key_export(&keys, passphrase, 1_000).expect("可以写成导出文件");
    let directory = tempfile::tempdir().expect("临时目录");
    let path = directory.path().join("room-keys.txt");
    std::fs::write(&path, export).expect("可以写导出文件");
    let result = human
        .encryption()
        .import_room_keys(path, passphrase)
        .await
        .expect("人可以导入重发的密钥");
    assert_eq!(result.imported_count, 1);
}
