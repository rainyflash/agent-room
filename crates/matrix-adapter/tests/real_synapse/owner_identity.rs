//! 主人在 Agent 核对过之后重建了签名身份（人的设备自动签名找不到能用的钥匙时会这样做，
//! ADR 0011）：认得主人的 Agent 撤销旧核对、记住新身份，照常收发；不认得的仍按 ADR 0009 拒绝。

use agent_room_application::ports::MatrixUserId;
use agent_room_bridge_core::matrix_security::MatrixSecurityFailure;
use matrix_sdk::{
    encryption::CrossSigningResetAuthType,
    ruma::{
        UserId,
        api::client::uiaa::{AuthData, MatrixUserIdentifier, Password, UserIdentifier},
    },
};
use serde_json::json;

use super::{
    establish_identity, message_event, required_environment, send_with_retry,
    stale_device_view_room, sync_until_event, unique_value,
};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置和管理员账号"]
async fn 真实_synapse_主人核对过之后重建签名身份_agent_撤销旧核对照常收发() {
    let (agent, owner, room_id) = stale_device_view_room().await;
    establish_identity(&owner).await;
    let owner_user = owner.matrix().session().metadata().user_id().clone();
    let owner_sdk_user = UserId::parse(owner_user.as_str()).expect("主人的 Matrix ID 有效");

    // Agent 以前核对过主人：和在 MCP 里核对安全码的结果一样，用自己的用户签名钥匙签了主人的主密钥。
    let identity = agent
        .sdk_client()
        .encryption()
        .request_user_identity(&owner_sdk_user)
        .await
        .expect("可以查询主人的身份")
        .expect("主人已有签名身份");
    identity.verify().await.expect("Agent 可以核对主人");
    // 重新查一次主人的身份，签名回到本机后 SDK 才记下“核对过”。
    assert!(
        agent
            .sdk_client()
            .encryption()
            .request_user_identity(&owner_sdk_user)
            .await
            .expect("可以查询主人的身份")
            .expect("主人已有签名身份")
            .is_verified(),
        "Agent 核对主人之后，本机要认为主人核对过了"
    );

    reset_owner_identity(&owner.sdk_client(), &owner_user).await;

    // 不认得主人时，核对过的人换了身份就发不出去（ADR 0009）。
    assert!(matches!(
        agent
            .security_gateway_handle()
            .ensure_room_ready(&room_id)
            .await,
        Err(MatrixSecurityFailure::IdentityChanged)
    ));

    // 认得主人：撤销旧核对、记住新身份，照常发；主人换过身份的设备拿得到房间密钥。
    agent.set_owner(&owner_user);
    agent
        .security_gateway_handle()
        .ensure_room_ready(&room_id)
        .await
        .expect("主人换了签名身份也能照常发");
    let sent = send_with_retry(
        agent.matrix().gateway(),
        &room_id,
        &message_event(
            unique_value("after-owner-reset"),
            "主人换了签名身份之后的消息",
        ),
    )
    .await;
    let received = sync_until_event(owner.matrix().gateway(), &room_id, sent.event_id()).await;
    assert_eq!(
        received.event_type().as_str(),
        "io.github.rainyflash.agentroom.message.preview.v1",
        "主人的设备必须拿到房间密钥并解开这条消息"
    );

    // 主人换了身份之后发的消息，Agent 照常当作可信。
    owner
        .security_gateway_handle()
        .ensure_room_ready(&room_id)
        .await
        .expect("主人可以发送");
    let sent = send_with_retry(
        owner.matrix().gateway(),
        &room_id,
        &message_event(
            unique_value("owner-after-reset"),
            "主人换了身份之后发的消息",
        ),
    )
    .await;
    let received = sync_until_event(agent.matrix().gateway(), &room_id, sent.event_id()).await;
    assert!(received.end_to_end_encrypted());
    assert!(
        received.end_to_end_sender_trusted(),
        "主人换了签名身份之后发的消息不能被隔离"
    );
}

/// 主人重建签名身份（新身份签好他这台设备）。产品里人的设备经控制面、由应用服务上传新的签名
/// 钥匙；这里只关心 Agent 怎么应对，所以用最直接的办法：管理员给这个测试账户设一个口令，主人
/// 用口令通过交互认证再换。
async fn reset_owner_identity(owner: &matrix_sdk::Client, owner_user: &MatrixUserId) {
    let password = unique_value("owner-password");
    set_password(owner_user, &password).await;
    let handle = owner
        .encryption()
        .reset_cross_signing()
        .await
        .expect("可以开始换签名身份")
        .expect("已有签名身份时换身份要交互认证");
    let CrossSigningResetAuthType::Uiaa(info) = handle.auth_type() else {
        panic!("测试服务器没有接 OAuth，换身份走口令认证");
    };
    let mut auth = Password::new(
        UserIdentifier::Matrix(MatrixUserIdentifier::new(owner_user.as_str().to_owned())),
        password,
    );
    auth.session.clone_from(&info.session);
    handle
        .auth(Some(AuthData::Password(auth)))
        .await
        .expect("口令认证后可以换签名身份");
}

/// 用管理员账号给测试账户设口令（不让它的设备退出登录）。
async fn set_password(user: &MatrixUserId, password: &str) {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let admin = matrix_sdk::Client::builder()
        .homeserver_url(&base_url)
        .build()
        .await
        .expect("可以建立管理员客户端");
    let login = admin
        .matrix_auth()
        .login_username(
            required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_USER"),
            &required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_PASSWORD"),
        )
        .send()
        .await
        .expect("管理员可以登录");
    let response = admin
        .http_client()
        .put(format!(
            "{}/_synapse/admin/v2/users/{}",
            base_url.trim_end_matches('/'),
            user.as_str()
        ))
        .bearer_auth(login.access_token)
        .header("content-type", "application/json")
        .body(json!({ "password": password, "logout_devices": false }).to_string())
        .send()
        .await
        .expect("管理接口可以访问");
    assert!(
        response.status().is_success(),
        "管理员设口令失败：{}",
        response.status()
    );
}
