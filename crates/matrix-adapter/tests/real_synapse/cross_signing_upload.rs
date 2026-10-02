//! 人的设备重建签名身份时，控制面以应用服务的身份替本人上传新的签名公钥（ADR 0011）。
//! 已有签名身份时换身份，Synapse 对普通请求要交互认证；应用服务的请求免（MSC4190）。
//! 这要求应用服务注册里有覆盖本地用户的非独占命名空间（`tools/dev-infra.ps1`）。

use agent_room_application::ports::{
    MatrixCrossSigningKeys, MatrixCrossSigningResetGateway, MatrixUserId, SecretValue,
};
use agent_room_matrix_provisioning_adapter::{
    MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
};
use matrix_sdk::ruma::canonical_json::to_canonical_value;
use matrix_sdk_base::crypto::vodozemac::Ed25519SecretKey;
use serde_json::{Value, json};

use super::{TEST_REQUEST_TIMEOUT, required_environment, unique_value};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置和管理员账号"]
async fn 真实_synapse_应用服务替真人换签名身份不用交互认证() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let admin = admin_token(&base_url).await;
    // 普通的真人账号：不在应用服务的 Agent 独占命名空间里。
    let localpart = unique_value("human")
        .replace(['_', ':'], "-")
        .to_lowercase();
    let server = required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_USER")
        .split_once(':')
        .map(|(_, server)| server.to_owned())
        .expect("管理员 Matrix ID 带服务器名");
    let human = format!("@{localpart}:{server}");
    let password = unique_value("human-password");
    create_user(&base_url, &admin, &human, &password).await;

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
    let user = MatrixUserId::new(human.clone()).expect("用户有效");

    // 第一次是建立签名身份（本来就不用交互认证），第二次是换身份（普通请求要交互认证）。
    let first = Ed25519SecretKey::new();
    provisioner
        .replace_cross_signing_keys(&user, &signing_keys(&human, &first))
        .await
        .expect("应用服务可以替真人建立签名身份");
    let second = Ed25519SecretKey::new();
    provisioner
        .replace_cross_signing_keys(&user, &signing_keys(&human, &second))
        .await
        .expect("应用服务替真人换签名身份不用交互认证");

    let master = query_master_key(&base_url, &human, &password).await;
    assert_eq!(
        master,
        second.public_key().to_base64(),
        "服务器上的主密钥换成了第二次上传的"
    );
}

/// 用一把主密钥派生出三把签名公钥（自签和用户签名两把由主密钥签名），和设备上建的一样。
fn signing_keys(user: &str, master: &Ed25519SecretKey) -> MatrixCrossSigningKeys {
    let self_signing = Ed25519SecretKey::new();
    let user_signing = Ed25519SecretKey::new();
    let mut self_signing_key = public_key(user, "self_signing", &self_signing);
    let mut user_signing_key = public_key(user, "user_signing", &user_signing);
    sign(&mut self_signing_key, user, master);
    sign(&mut user_signing_key, user, master);
    MatrixCrossSigningKeys::new(
        json!({
            "master_key": public_key(user, "master", master),
            "self_signing_key": self_signing_key,
            "user_signing_key": user_signing_key,
        }),
        &MatrixUserId::new(user).expect("用户有效"),
    )
    .expect("签名公钥有效")
}

fn public_key(user: &str, usage: &str, key: &Ed25519SecretKey) -> Value {
    let public = key.public_key().to_base64();
    json!({
        "user_id": user,
        "usage": [usage],
        "keys": { format!("ed25519:{public}"): public },
    })
}

fn sign(object: &mut Value, user: &str, signer: &Ed25519SecretKey) {
    let canonical = to_canonical_value(&*object).expect("可以转成规范 JSON");
    let signature = signer.sign(canonical.to_string().as_bytes());
    let public = signer.public_key().to_base64();
    object["signatures"] = json!({ user: { format!("ed25519:{public}"): signature.to_base64() } });
}

async fn admin_token(base_url: &str) -> String {
    let admin = matrix_sdk::Client::builder()
        .homeserver_url(base_url)
        .build()
        .await
        .expect("可以建立管理员客户端");
    admin
        .matrix_auth()
        .login_username(
            required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_USER"),
            &required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_PASSWORD"),
        )
        .send()
        .await
        .expect("管理员可以登录")
        .access_token
}

async fn create_user(base_url: &str, admin: &str, user: &str, password: &str) {
    let response = http()
        .put(format!(
            "{}/_synapse/admin/v2/users/{user}",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(json!({ "password": password }).to_string())
        .send()
        .await
        .expect("管理接口可以访问");
    assert!(
        response.status().is_success(),
        "建真人账号失败：{}",
        response.status()
    );
}

async fn query_master_key(base_url: &str, user: &str, password: &str) -> String {
    let client = matrix_sdk::Client::builder()
        .homeserver_url(base_url)
        .build()
        .await
        .expect("可以建立客户端");
    let token = client
        .matrix_auth()
        .login_username(user, password)
        .send()
        .await
        .expect("真人可以用口令登录")
        .access_token;
    let response: Value = http()
        .post(format!(
            "{}/_matrix/client/v3/keys/query",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(json!({ "device_keys": { user: [] } }).to_string())
        .send()
        .await
        .expect("可以查询签名公钥")
        .json()
        .await
        .expect("回应是 JSON");
    response["master_keys"][user]["keys"]
        .as_object()
        .and_then(|keys| keys.values().next())
        .and_then(Value::as_str)
        .expect("有主密钥")
        .to_owned()
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}
