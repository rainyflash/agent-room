//! 桌面端的聊天登录由控制面替人发（`specs/desktop-sign-in/design.md`）：以应用服务的身份替一个
//! 本地的人登录 Matrix。第一次用就在桌面端的人还没有 Matrix 账户，先由应用服务注册。
//! 这要求应用服务注册里有覆盖本地用户的非独占命名空间（`tools/dev-infra.ps1`）。

use agent_room_application::ports::{MatrixHumanSessionIssuer, MatrixUserId, SecretValue};
use agent_room_matrix_provisioning_adapter::{
    MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
};
use serde_json::Value;

use super::{TEST_REQUEST_TIMEOUT, required_environment, unique_value};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 真实_synapse_应用服务替还没有账户的人注册并登录出带刷新令牌的设备() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let server = required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_USER")
        .split_once(':')
        .map(|(_, server)| server.to_owned())
        .expect("管理员 Matrix ID 带服务器名");
    // 和控制面算的一样是 `user-` 开头：不在应用服务的 Agent 独占命名空间里。
    let localpart = unique_value("user").replace('_', "-");
    let human = MatrixUserId::new(format!("@{localpart}:{server}")).expect("用户有效");
    let provisioner = MatrixApplicationServiceProvisioner::new(
        MatrixApplicationServiceConfiguration::new(
            &base_url,
            server,
            SecretValue::new(required_environment(
                "AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN",
            ))
            .expect("应用服务令牌有效"),
            TEST_REQUEST_TIMEOUT,
        )
        .expect("应用服务配置有效"),
    )
    .expect("应用服务客户端可以建立");

    let first = provisioner
        .issue_human_session(&human, "桌面上的人")
        .await
        .expect("应用服务替还没有账户的人注册并登录");
    assert!(first.refresh_token().is_some(), "带刷新令牌");
    let whoami = whoami(&base_url, first.access_token().expose()).await;
    assert_eq!(whoami["user_id"], human.as_str());
    assert_eq!(whoami["device_id"], first.metadata().device_id().as_str());
    assert_eq!(
        display_name(&base_url, &human).await.as_deref(),
        Some("桌面上的人"),
        "显示名写成账户的名字"
    );

    // 再要一次：账户已经有了，照样登录出另一台设备，名字不动。
    let second = provisioner
        .issue_human_session(&human, "改过的名字")
        .await
        .expect("已经有账户时照样登录");
    assert_ne!(
        second.metadata().device_id(),
        first.metadata().device_id(),
        "每次都是一台新设备"
    );
    assert_eq!(
        display_name(&base_url, &human).await.as_deref(),
        Some("桌面上的人"),
        "已经有名字就不改"
    );
}

async fn whoami(base_url: &str, token: &str) -> Value {
    reqwest::Client::new()
        .get(format!(
            "{}/_matrix/client/v3/account/whoami",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("可以问自己是谁")
        .json()
        .await
        .expect("回应是 JSON")
}

async fn display_name(base_url: &str, user: &MatrixUserId) -> Option<String> {
    let response: Value = reqwest::Client::new()
        .get(format!(
            "{}/_matrix/client/v3/profile/{}/displayname",
            base_url.trim_end_matches('/'),
            user.as_str()
        ))
        .send()
        .await
        .expect("可以读显示名")
        .json()
        .await
        .expect("回应是 JSON");
    response["displayname"].as_str().map(str::to_owned)
}
