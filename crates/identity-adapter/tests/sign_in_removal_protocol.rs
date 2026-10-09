use std::{sync::Arc, time::Duration};

use agent_room_application::ports::{
    OidcFailure, OidcFailureKind, SecretValue, SignInAccount, SignInAccountRemoval,
};
use agent_room_identity_adapter::{KeycloakAccountRemovalConfig, KeycloakSignInAccountRemoval};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{delete, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

const CLIENT_ID: &str = "agent-room-account-admin";
const CLIENT_SECRET: &str = "删除账户专用的测试密钥";
const ACCESS_TOKEN: &str = "service-account-token";
const ISSUER: &str = "https://auth.agent-room.test/realms/agent-room";

#[derive(Clone, Copy)]
enum 删除结果 {
    删掉了,
    本来就没有,
    暂时出错,
}

#[derive(Clone)]
struct FakeKeycloak {
    outcome: 删除结果,
    token_requests: Arc<Mutex<u16>>,
    deleted: Arc<Mutex<Vec<String>>>,
}

struct Running {
    keycloak: FakeKeycloak,
    internal_url: String,
    task: JoinHandle<()>,
}

impl Running {
    async fn start(outcome: 删除结果) -> Self {
        let keycloak = FakeKeycloak {
            outcome,
            token_requests: Arc::new(Mutex::new(0)),
            deleted: Arc::new(Mutex::new(Vec::new())),
        };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("应能绑定本地测试端口");
        let internal_url = format!("http://{}", listener.local_addr().expect("测试地址可读"));
        let router = Router::new()
            .route(
                "/realms/agent-room/protocol/openid-connect/token",
                post(发令牌),
            )
            .route("/admin/realms/agent-room/users/{id}", delete(删用户))
            .with_state(keycloak.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("假 Keycloak 不应异常退出");
        });
        Self {
            keycloak,
            internal_url,
            task,
        }
    }

    fn removal(&self, client_secret: &str) -> KeycloakSignInAccountRemoval {
        KeycloakSignInAccountRemoval::new(KeycloakAccountRemovalConfig {
            internal_url: self.internal_url.clone(),
            issuer_url: ISSUER.to_owned(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: SecretValue::new(client_secret).expect("测试密钥有效"),
            request_timeout: Duration::from_secs(5),
        })
        .expect("配置有效")
    }

    async fn deleted(&self) -> Vec<String> {
        self.keycloak.deleted.lock().await.clone()
    }

    async fn token_requests(&self) -> u16 {
        *self.keycloak.token_requests.lock().await
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn 发令牌(
    State(keycloak): State<FakeKeycloak>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    *keycloak.token_requests.lock().await += 1;
    let expected = format!(
        "Basic {}",
        STANDARD.encode(format!(
            "{}:{}",
            urlencoding(CLIENT_ID),
            urlencoding(CLIENT_SECRET)
        ))
    );
    let form = String::from_utf8_lossy(&body);
    if headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized_client"})),
        )
            .into_response();
    }
    assert!(form.contains("grant_type=client_credentials"), "{form}");
    Json(json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "Bearer",
        "expires_in": 60
    }))
    .into_response()
}

async fn 删用户(
    State(keycloak): State<FakeKeycloak>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> StatusCode {
    let expected = format!("Bearer {ACCESS_TOKEN}");
    let authorized = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str());
    if !authorized {
        return StatusCode::UNAUTHORIZED;
    }
    keycloak.deleted.lock().await.push(id);
    match keycloak.outcome {
        删除结果::删掉了 => StatusCode::NO_CONTENT,
        删除结果::本来就没有 => StatusCode::NOT_FOUND,
        删除结果::暂时出错 => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// oauth2 的 Basic 认证先对客户端标识和密钥做表单编码，再拼起来做 base64。
fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'*' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn account(issuer: &str, subject: &str) -> SignInAccount {
    SignInAccount {
        issuer: issuer.to_owned(),
        subject: subject.to_owned(),
    }
}

#[tokio::test]
async fn 删掉本部署签发方的登录账户() {
    let keycloak = Running::start(删除结果::删掉了).await;

    keycloak
        .removal(CLIENT_SECRET)
        .remove(&account(ISSUER, "0b9a1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d"))
        .await
        .expect("删除成功");

    assert_eq!(
        keycloak.deleted().await,
        ["0b9a1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d"]
    );
}

#[tokio::test]
async fn 用户已经不在也算删掉了() {
    let keycloak = Running::start(删除结果::本来就没有).await;

    keycloak
        .removal(CLIENT_SECRET)
        .remove(&account(&format!("{ISSUER}/"), "already-gone"))
        .await
        .expect("已经不在也算成功");

    assert_eq!(keycloak.deleted().await, ["already-gone"]);
}

#[tokio::test]
async fn 别的签发方的账户不碰_网络_agent_没有登录账户() {
    let keycloak = Running::start(删除结果::删掉了).await;

    keycloak
        .removal(CLIENT_SECRET)
        .remove(&account("urn:agent-room:network-agent", "network-agent"))
        .await
        .expect("没有可删的也算成功");

    assert_eq!(keycloak.token_requests().await, 0);
    assert!(keycloak.deleted().await.is_empty());
}

#[tokio::test]
async fn 客户端密钥不对时算被拒绝_不删任何人() {
    let keycloak = Running::start(删除结果::删掉了).await;

    let failure = keycloak
        .removal("不对的密钥")
        .remove(&account(ISSUER, "someone"))
        .await
        .map_err(OidcFailure::kind)
        .expect_err("拿不到令牌");

    assert_eq!(failure, OidcFailureKind::ProviderRejected);
    assert!(keycloak.deleted().await.is_empty());
}

#[tokio::test]
async fn keycloak_暂时出错时下次再试() {
    let keycloak = Running::start(删除结果::暂时出错).await;

    let failure = keycloak
        .removal(CLIENT_SECRET)
        .remove(&account(ISSUER, "someone"))
        .await
        .map_err(OidcFailure::kind)
        .expect_err("服务端出错");

    assert_eq!(failure, OidcFailureKind::DependencyUnavailable);
}

#[tokio::test]
async fn 用户号里的斜杠拼不出别的地址() {
    let keycloak = Running::start(删除结果::删掉了).await;

    keycloak
        .removal(CLIENT_SECRET)
        .remove(&account(ISSUER, "a/../../clients"))
        .await
        .expect("编码后仍是一个用户号");

    assert_eq!(keycloak.deleted().await, ["a/../../clients"]);
}

#[tokio::test]
async fn 连不上_keycloak_时下次再试() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("应能绑定本地测试端口");
    let internal_url = format!("http://{}", listener.local_addr().expect("测试地址可读"));
    drop(listener);
    let removal = KeycloakSignInAccountRemoval::new(KeycloakAccountRemovalConfig {
        internal_url,
        issuer_url: ISSUER.to_owned(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: SecretValue::new(CLIENT_SECRET).expect("测试密钥有效"),
        request_timeout: Duration::from_secs(2),
    })
    .expect("配置有效");

    let failure = removal
        .remove(&account(ISSUER, "someone"))
        .await
        .map_err(OidcFailure::kind)
        .expect_err("连不上");

    assert_eq!(failure, OidcFailureKind::DependencyUnavailable);
}
