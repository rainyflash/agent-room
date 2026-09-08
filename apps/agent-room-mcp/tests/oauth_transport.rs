use agent_room_agent_client::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{IpcErrorCategory, IpcMethod};
use agent_room_mcp::http::{HttpConfig, router};
use axum::{Json, Router, routing::get};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::{Client, StatusCode};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey as _, traits::PublicKeyParts as _};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write as _,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Bridge(AtomicUsize);
impl BridgeToolClient for Bridge {
    fn invoke(&self, _: IpcMethod) -> BridgeToolFuture<'_> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {
            Err(BridgeToolFailure::new(
                "test.unexpected",
                IpcErrorCategory::Validation,
                false,
                BTreeMap::new(),
            ))
        })
    }
}
fn key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap())
}
struct Server {
    url: String,
    issuer: String,
    bridge: Arc<Bridge>,
    client: Client,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    config_file: tempfile::NamedTempFile,
}
impl Server {
    async fn start() -> Self {
        let provider = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", provider.local_addr().unwrap());
        let discovery = json!({"issuer": issuer, "jwks_uri": format!("{issuer}/jwks"),
            "authorization_endpoint": format!("{issuer}/authorize"), "token_endpoint": format!("{issuer}/token"),
            "code_challenge_methods_supported":["S256"]});
        let jwks = json!({"keys":[{"kty":"RSA", "kid":"test-key", "alg":"RS256", "use":"sig",
            "n":URL_SAFE_NO_PAD.encode(key().n().to_bytes_be()), "e":URL_SAFE_NO_PAD.encode(key().e().to_bytes_be())}]});
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || async { Json(discovery) }),
            )
            .route("/jwks", get(move || async { Json(jwks) }));
        let provider_task = tokio::spawn(async move {
            axum::serve(provider, app).await.unwrap();
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind = listener.local_addr().unwrap();
        let url = format!("http://{bind}/mcp");
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", json!({"issuer":issuer,"ownerSubject":"owner","allowedClientIds":["approved-host"],"scope":"agent-room"})).unwrap();
        let config = HttpConfig::oauth(bind, &url, file.path()).await.unwrap();
        let bridge = Arc::new(Bridge(AtomicUsize::new(0)));
        let app = router(bridge.clone(), config);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            issuer,
            bridge,
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            tasks: vec![provider_task, task],
            config_file: file,
        }
    }
    fn claims(&self) -> Value {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        json!({"iss":self.issuer,"aud":self.url,"sub":"owner","exp":now+300,"nbf":now-1,
            "scope":"openid agent-room", "azp":"approved-host", "typ":"Bearer"})
    }
    fn token(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key".into());
        let der = key().to_pkcs1_der().unwrap();
        encode(&header, claims, &EncodingKey::from_rsa_der(der.as_bytes())).unwrap()
    }
    async fn tools(&self, token: Option<&str>) -> reqwest::Response {
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[tokio::test]
async fn oauth_发现与签名令牌经过真实_http_后可使用工具() {
    let server = Server::start().await;
    let denied = server.tools(None).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let metadata_url = format!(
        "{}/.well-known/oauth-protected-resource/mcp",
        server.url.trim_end_matches("/mcp")
    );
    assert!(
        denied.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .contains(&metadata_url)
    );
    let metadata = server.client.get(&metadata_url).send().await.unwrap();
    assert_eq!(metadata.headers()["cache-control"], "no-store");
    let document: Value = metadata.json().await.unwrap();
    assert_eq!(document["resource"], server.url);
    assert_eq!(document["authorization_servers"], json!([server.issuer]));
    let response = server.tools(Some(&Server::token(&server.claims()))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "agent_room_wait_for_messages")
    );
    assert_eq!(server.bridge.0.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn 其他所有者宿主资源过期及_id_token_全部拒绝() {
    let server = Server::start().await;
    for (field, value) in [
        ("iss", json!("https://other.invalid")),
        ("sub", json!("other-owner")),
        ("aud", json!("https://other.invalid/mcp")),
        ("azp", json!("unapproved-host")),
        ("client_id", json!("conflicting-host")),
        ("typ", json!("ID")),
        ("exp", json!(0)),
        (
            "nbf",
            json!(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600
            ),
        ),
    ] {
        let mut claims = server.claims();
        claims[field] = value;
        assert_eq!(
            server.tools(Some(&Server::token(&claims))).await.status(),
            StatusCode::UNAUTHORIZED,
            "{field}"
        );
    }
    let mut claims = server.claims();
    claims["scope"] = json!("openid");
    let response = server.tools(Some(&Server::token(&claims))).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        response.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("insufficient_scope")
    );
    let mut token = Server::token(&server.claims());
    token.push('a');
    assert_eq!(
        server.tools(Some(&token)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(server.bridge.0.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn 公网配置不接受明文_oauth_签发者() {
    let server = Server::start().await;
    assert!(
        HttpConfig::oauth(
            "0.0.0.0:8181".parse().unwrap(),
            "https://agents.example.com/mcp",
            server.config_file.path()
        )
        .await
        .is_err()
    );
}
