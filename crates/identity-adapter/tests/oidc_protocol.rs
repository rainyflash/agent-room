use std::{collections::HashMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    OidcAuthorizationOptions, OidcCodeExchange, OidcFailureKind, OidcGateway, SecretValue,
};
use agent_room_domain::time::DurationMillis;
use agent_room_identity_adapter::{DiscoveredOidcGateway, OidcAdapterConfig};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, EmptyExtraTokenFields, EndUserUsername,
    IssuerUrl, LanguageTag, Nonce, StandardClaims, SubjectIdentifier,
    core::{
        CoreHmacKey, CoreIdToken, CoreIdTokenClaims, CoreIdTokenFields, CoreJwsSigningAlgorithm,
        CoreTokenResponse, CoreTokenType,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use url::Url;

const CLIENT_ID: &str = "agent-room-test";
const CLIENT_SECRET: &str = "本地测试专用机密-至少三十二字节-不可用于生产";

#[derive(Debug, Clone, Copy)]
enum 令牌变体 {
    有效,
    错误签发者,
    错误受众,
    已过期,
    错误随机数,
    错误访问令牌哈希,
}

#[derive(Clone)]
struct 提供者状态 {
    issuer: String,
    token_variant: 令牌变体,
    expected_challenge: Arc<Mutex<Option<String>>>,
    expected_nonce: Arc<Mutex<Option<String>>>,
}

struct 假提供者 {
    issuer: String,
    state: 提供者状态,
    task: JoinHandle<()>,
}

impl 假提供者 {
    async fn 启动(token_variant: 令牌变体) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("应能绑定本地测试端口");
        let issuer = format!("http://{}", listener.local_addr().expect("测试地址可读"));
        let state = 提供者状态 {
            issuer: issuer.clone(),
            token_variant,
            expected_challenge: Arc::new(Mutex::new(None)),
            expected_nonce: Arc::new(Mutex::new(None)),
        };
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(发现文档))
            .route("/jwks", get(公钥集))
            .route("/token", post(兑换令牌))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("假 OIDC 提供者不应异常退出");
        });

        Self {
            issuer,
            state,
            task,
        }
    }

    fn 网关(&self) -> DiscoveredOidcGateway {
        DiscoveredOidcGateway::new(OidcAdapterConfig {
            issuer_url: self.issuer.clone(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: SecretValue::new(CLIENT_SECRET).expect("测试密钥有效"),
            redirect_url: "https://app.agent-room.test/auth/callback".to_owned(),
            request_timeout: Duration::from_secs(2),
        })
        .expect("测试网关配置有效")
    }

    async fn 记录授权上下文(&self, authorization_url: &str, expected_nonce: &SecretValue) {
        let url = Url::parse(authorization_url).expect("授权地址有效");
        let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        let challenge = query
            .get("code_challenge")
            .expect("授权请求必须携带 PKCE challenge")
            .clone();
        self.state
            .expected_challenge
            .lock()
            .await
            .replace(challenge);
        self.state
            .expected_nonce
            .lock()
            .await
            .replace(expected_nonce.expose().to_owned());
    }
}

impl Drop for 假提供者 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn 发现文档(State(state): State<提供者状态>) -> Json<serde_json::Value> {
    Json(json!({
        "issuer": state.issuer,
        "authorization_endpoint": format!("{}/authorize", state.issuer),
        "token_endpoint": format!("{}/token", state.issuer),
        "jwks_uri": format!("{}/jwks", state.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["HS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["openid", "profile"]
    }))
}

async fn 公钥集() -> Json<serde_json::Value> {
    Json(json!({ "keys": [] }))
}

async fn 兑换令牌(State(state): State<提供者状态>, body: Bytes) -> Response {
    let form = url::form_urlencoded::parse(&body)
        .into_owned()
        .collect::<HashMap<_, _>>();
    if form.get("grant_type").map(String::as_str) != Some("authorization_code")
        || form.get("code").map(String::as_str) != Some("valid-code")
        || !pkce_有效(&state, form.get("code_verifier")).await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_grant",
                "error_description": "授权码或 PKCE verifier 无效"
            })),
        )
            .into_response();
    }

    Json(创建令牌响应(&state).await).into_response()
}

async fn pkce_有效(state: &提供者状态, verifier: Option<&String>) -> bool {
    let Some(verifier) = verifier else {
        return false;
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    state.expected_challenge.lock().await.as_ref() == Some(&challenge)
}

async fn 创建令牌响应(state: &提供者状态) -> CoreTokenResponse {
    let now = Utc::now();
    let issuer = match state.token_variant {
        令牌变体::错误签发者 => format!("{}/错误签发者", state.issuer),
        _ => state.issuer.clone(),
    };
    let audience = match state.token_variant {
        令牌变体::错误受众 => "another-client",
        _ => CLIENT_ID,
    };
    let expiration = match state.token_variant {
        令牌变体::已过期 => now - TimeDelta::minutes(1),
        _ => now + TimeDelta::minutes(5),
    };
    let expected_nonce = state
        .expected_nonce
        .lock()
        .await
        .clone()
        .expect("测试必须先记录 nonce");
    let nonce = match state.token_variant {
        令牌变体::错误随机数 => "wrong-nonce".to_owned(),
        _ => expected_nonce,
    };
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(issuer).expect("测试 issuer 有效"),
        vec![Audience::new(audience.to_owned())],
        expiration,
        now,
        StandardClaims::new(SubjectIdentifier::new("subject-42".to_owned())),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(nonce)))
    .set_auth_time(Some(now))
    .set_preferred_username(Some(EndUserUsername::new("测试玩家".to_owned())))
    .set_locale(Some(LanguageTag::new("zh-CN".to_owned())));
    let response_access_token = AccessToken::new("access-token".to_owned());
    let signed_access_token = match state.token_variant {
        令牌变体::错误访问令牌哈希 => {
            AccessToken::new("tampered-access-token".to_owned())
        }
        _ => response_access_token.clone(),
    };
    let id_token = CoreIdToken::new(
        claims,
        &CoreHmacKey::new(CLIENT_SECRET.as_bytes()),
        CoreJwsSigningAlgorithm::HmacSha256,
        Some(&signed_access_token),
        None,
    )
    .expect("测试 ID Token 应可签名");

    CoreTokenResponse::new(
        response_access_token,
        CoreTokenType::Bearer,
        CoreIdTokenFields::new(Some(id_token), EmptyExtraTokenFields {}),
    )
}

async fn 执行兑换(
    token_variant: 令牌变体,
) -> Result<agent_room_application::ports::VerifiedOidcIdentity, OidcFailureKind> {
    let provider = 假提供者::启动(token_variant).await;
    let gateway = provider.网关();
    let authorization = gateway
        .begin_authorization(OidcAuthorizationOptions {
            request_profile: true,
            maximum_authentication_age: DurationMillis::new(300_000).expect("时长有效"),
        })
        .await
        .expect("Discovery 与授权请求应成功");
    provider
        .记录授权上下文(&authorization.authorization_url, &authorization.nonce)
        .await;
    gateway
        .exchange_code(OidcCodeExchange {
            code: "valid-code",
            pkce_verifier: &authorization.pkce_verifier,
            expected_nonce: &authorization.nonce,
        })
        .await
        .map_err(agent_room_application::ports::OidcFailure::kind)
}

#[tokio::test]
async fn 完整授权码流程校验_pkce_签名_声明和访问令牌哈希() {
    let identity = 执行兑换(令牌变体::有效).await.expect("有效协议响应应通过");

    assert_eq!(identity.subject(), "subject-42");
    assert_eq!(identity.display_name(), Some("测试玩家"));
    assert_eq!(identity.locale(), Some("zh-CN"));
    assert!(identity.authenticated_at().is_some());
}

#[tokio::test]
async fn 错误_pkce_verifier_由提供者拒绝() {
    let provider = 假提供者::启动(令牌变体::有效).await;
    let gateway = provider.网关();
    let authorization = gateway
        .begin_authorization(OidcAuthorizationOptions {
            request_profile: false,
            maximum_authentication_age: DurationMillis::new(300_000).expect("时长有效"),
        })
        .await
        .expect("授权请求应成功");
    provider
        .记录授权上下文(&authorization.authorization_url, &authorization.nonce)
        .await;
    let wrong_verifier = SecretValue::new("wrong-verifier").expect("测试值有效");

    let failure = gateway
        .exchange_code(OidcCodeExchange {
            code: "valid-code",
            pkce_verifier: &wrong_verifier,
            expected_nonce: &authorization.nonce,
        })
        .await
        .expect_err("错误 verifier 必须失败");

    assert_eq!(failure.kind(), OidcFailureKind::ProviderRejected);
}

#[tokio::test]
async fn 拒绝错误签发者_受众_过期_nonce_和_at_hash() {
    for variant in [
        令牌变体::错误签发者,
        令牌变体::错误受众,
        令牌变体::已过期,
        令牌变体::错误随机数,
        令牌变体::错误访问令牌哈希,
    ] {
        let failure = 执行兑换(variant).await.expect_err("无效 ID Token 必须失败");
        assert_eq!(
            failure,
            OidcFailureKind::InvalidIdentityToken,
            "{variant:?}"
        );
    }
}
