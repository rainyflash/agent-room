use agent_room_application::ports::MatrixBackfillRequest;
use agent_room_bridge_core::{
    matrix_recovery::{MatrixRecoveryCommand, MatrixRecoverySecret},
    matrix_security::{MatrixIdentityState, MatrixSecurityFailure},
};
use agent_room_matrix_adapter::MatrixSdkHandoffConnection;

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 新设备恢复原有身份与旧密文且拒绝错误口令及覆盖备份() {
    timeout(Duration::from_mins(3), exercise_recovery())
        .await
        .expect("恢复场景必须在预算内完成");
}

async fn exercise_recovery() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let user = provisioner
        .ensure_user(&MatrixAgentUserRegistration::new(
            MatrixAgentLocalpart::from_agent_id(AgentId::from_uuid(Uuid::now_v7())),
        ))
        .await
        .expect("建立隔离的恢复验收用户");
    let first = fresh_device(&provisioner, &base_url, &user).await;
    sync(first.matrix().gateway(), None).await;
    let security = first.security_gateway_handle();
    let before = security
        .recover(MatrixRecoveryCommand::Inspect)
        .await
        .expect("读取初始状态");
    assert_eq!(before.state.identity, MatrixIdentityState::Missing);
    let passphrase = format!("Recovery test {}", Uuid::now_v7());
    let enabled = security
        .recover(MatrixRecoveryCommand::Enable {
            passphrase: MatrixRecoverySecret::new(passphrase.clone()),
        })
        .await
        .expect("首次建立恢复备份");
    assert_eq!(enabled.state.identity, MatrixIdentityState::Ready);
    assert!(enabled.state.backup_enabled && enabled.state.recovery_available);
    let key = enabled.recovery_key.expect("设置时只返回一次恢复密钥");
    let original_master = master_key(&base_url, first.matrix()).await;

    let room = prepare_encrypted_history(&base_url, first.matrix()).await;

    let second = fresh_device(&provisioner, &base_url, &user).await;
    let room_sync =
        sync_until_room(second.matrix().gateway(), &room, MatrixRoomSyncKind::Joined).await;
    let second_security = second.security_gateway_handle();
    assert_locked(&second, &passphrase).await;

    let page = MatrixBackfillRequest::new(
        room_sync.previous_batch().cloned().expect("历史游标"),
        NonZeroU16::new(50).expect("非零"),
    )
    .expect("回填请求");
    let locked_history = second
        .matrix()
        .gateway()
        .backfill(&room, &page)
        .await
        .expect("读取旧密文");
    assert!(
        !locked_history
            .events()
            .iter()
            .any(|event| event.content()["body"] == "history before device replacement")
    );
    let restored = second_security
        .recover(MatrixRecoveryCommand::Restore {
            credential: MatrixRecoverySecret::new(passphrase),
        })
        .await
        .expect("用口令恢复身份与旧房间密钥");
    assert_eq!(restored.state.identity, MatrixIdentityState::Ready);
    assert!(restored.state.backup_enabled);
    assert!(restored.recovery_key.is_none());
    let recovered_history = second
        .matrix()
        .gateway()
        .backfill(&room, &page)
        .await
        .expect("读取恢复后的旧消息");
    assert!(
        recovered_history
            .events()
            .iter()
            .any(|event| event.content()["body"] == "history before device replacement")
    );
    assert_eq!(
        master_key(&base_url, second.matrix()).await,
        original_master
    );
    let sent = send_with_retry(
        second.matrix().gateway(),
        &room,
        &message_event(unique_value("after-recovery"), "encrypted after recovery"),
    )
    .await;
    encrypted_event_session_id(&base_url, second.matrix(), &room, sent.event_id()).await;

    let third = fresh_device(&provisioner, &base_url, &user).await;
    sync(third.matrix().gateway(), None).await;
    let recovered_with_key = third
        .security_gateway_handle()
        .recover(MatrixRecoveryCommand::Restore { credential: key })
        .await
        .expect("恢复密钥也可解锁独立的新设备");
    assert_eq!(
        recovered_with_key.state.identity,
        MatrixIdentityState::Ready
    );
    assert_eq!(master_key(&base_url, third.matrix()).await, original_master);
    leave_with_retry(third.matrix().gateway(), &room).await;
}

async fn fresh_device(
    provisioner: &MatrixApplicationServiceProvisioner,
    base_url: &str,
    user: &MatrixUserId,
) -> MatrixSdkHandoffConnection {
    let request = MatrixAgentDeviceSessionRequest::new(
        user.clone(),
        MatrixDeviceId::new(format!("RECOVERY_{}", Uuid::now_v7().simple())).expect("设备 ID"),
        "Isolated recovery test".into(),
    )
    .expect("新设备请求");
    let session = provisioner
        .issue_device_session(&request)
        .await
        .expect("签发独立设备");
    factory(base_url, TEST_REQUEST_TIMEOUT, 1)
        .restore_with_handoffs(&session)
        .await
        .expect("全新内存 Store")
}

async fn prepare_encrypted_history(base_url: &str, first: &MatrixConnection) -> MatrixRoomId {
    let room = create_room_with_retry(
        first.gateway(),
        &room_request().with_end_to_end_encryption(),
    )
    .await;
    sync_until_room(first.gateway(), &room, MatrixRoomSyncKind::Joined).await;
    let original = send_with_retry(
        first.gateway(),
        &room,
        &message_event(
            unique_value("recovery-history"),
            "history before device replacement",
        ),
    )
    .await;
    send_with_retry(
        first.gateway(),
        &room,
        &message_event(unique_value("recovery-marker"), "pagination marker"),
    )
    .await;
    encrypted_event_session_id(base_url, first, &room, original.event_id()).await;
    wait_for_backup(base_url, first).await;
    room
}

async fn assert_locked(second: &MatrixSdkHandoffConnection, passphrase: &str) {
    let security = second.security_gateway_handle();
    let locked = security
        .recover(MatrixRecoveryCommand::Inspect)
        .await
        .expect("新设备状态");
    assert_eq!(locked.state.identity, MatrixIdentityState::RecoveryRequired);
    assert!(locked.state.recovery_available);
    assert!(!locked.state.backup_enabled);
    assert_eq!(
        security
            .recover(MatrixRecoveryCommand::Enable {
                passphrase: MatrixRecoverySecret::new(passphrase.to_owned())
            })
            .await,
        Err(MatrixSecurityFailure::RecoveryAlreadyConfigured)
    );
    assert_eq!(
        security
            .recover(MatrixRecoveryCommand::Restore {
                credential: MatrixRecoverySecret::new("incorrect recovery credential".into())
            })
            .await,
        Err(MatrixSecurityFailure::RecoveryRejected)
    );
    assert_eq!(
        security
            .recover(MatrixRecoveryCommand::Inspect)
            .await
            .expect("错误口令后的状态")
            .state
            .identity,
        MatrixIdentityState::RecoveryRequired
    );
}

async fn master_key(base_url: &str, connection: &MatrixConnection) -> serde_json::Value {
    let user = connection.session().metadata().user_id().as_str();
    let response = reqwest::Client::new()
        .post(format!("{base_url}/_matrix/client/v3/keys/query"))
        .bearer_auth(connection.session().access_token().expose())
        .json(&json!({"device_keys": {user: []}}))
        .send()
        .await
        .expect("查询公钥")
        .error_for_status()
        .expect("公钥查询成功")
        .json::<serde_json::Value>()
        .await
        .expect("公钥 JSON");
    let key = response["master_keys"][user]["keys"].clone();
    assert!(key.is_object());
    key
}

async fn wait_for_backup(base_url: &str, connection: &MatrixConnection) {
    let mut since = None;
    for _ in 0..60 {
        let batch = sync(connection.gateway(), since).await;
        since = Some(batch.next_batch().clone());
        let response = reqwest::Client::new()
            .get(format!("{base_url}/_matrix/client/v3/room_keys/version"))
            .bearer_auth(connection.session().access_token().expose())
            .send()
            .await
            .expect("查询备份")
            .error_for_status()
            .expect("备份存在")
            .json::<serde_json::Value>()
            .await
            .expect("备份 JSON");
        if response["count"].as_u64().is_some_and(|count| count > 0) {
            return;
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!("房间密钥未上传到加密备份");
}
