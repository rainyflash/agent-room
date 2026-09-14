use std::{collections::HashMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    OidcDeviceAssertionVerifier, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDeviceGrantGateway, OidcDevicePromptFailure, OidcFailureKind,
};
use agent_room_identity_adapter::{DiscoveredOidcDeviceGrant, OidcDeviceGrantConfig};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{TimeDelta, Utc};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, EmptyExtraTokenFields, EndUserUsername,
    IssuerUrl, JsonWebKeyId, LanguageTag, PrivateSigningKey, StandardClaims, SubjectIdentifier,
    core::{
        CoreIdToken, CoreIdTokenClaims, CoreIdTokenFields, CoreJsonWebKeySet,
        CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey, CoreTokenResponse, CoreTokenType,
    },
};
use rand::rngs::OsRng;
use rsa::{
    RsaPrivateKey,
    pkcs1::{EncodeRsaPrivateKey, LineEnding},
};
use serde_json::json;
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

const CLIENT_ID: &str = "agent-room-bridge-test";
const DEVICE_CODE: &str = "一次性设备授权码-测试专用";
const USER_CODE: &str = "ABCD-EFGH";
const TEST_KEY_ID: &str = "agent-room-device-grant-test-key";

#[derive(Debug, Clone, Copy)]
enum 断言变体 {
    有效,
    错误受众,
    复用旧登录,
    省略登录时间,
    已过期,
}

#[derive(Clone)]
struct 提供者状态 {
    issuer: String,
    assertion_variant: 断言变体,
    poll_count: Arc<Mutex<usize>>,
    signing_key_pem: Arc<str>,
}

struct 假设备授权提供者 {
    issuer: String,
    task: JoinHandle<()>,
}

impl 假设备授权提供者 {
    async fn 启动(assertion_variant: 断言变体) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("应能绑定本地测试端口");
        let issuer = format!("http://{}", listener.local_addr().expect("测试地址可读"));
        let state = 提供者状态 {
            issuer: issuer.clone(),
            assertion_variant,
            poll_count: Arc::new(Mutex::new(0)),
            signing_key_pem: Arc::from(生成测试签名密钥()),
        };
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(发现文档))
            .route("/jwks", get(公钥集))
            .route("/device", post(创建设备码))
            .route("/token", post(轮询令牌))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("假 OIDC 提供者不应异常退出");
        });
        Self { issuer, task }
    }

    fn 网关(&self) -> DiscoveredOidcDeviceGrant {
        DiscoveredOidcDeviceGrant::new(OidcDeviceGrantConfig {
            issuer_url: self.issuer.clone(),
            client_id: CLIENT_ID.to_owned(),
            request_timeout: Duration::from_secs(2),
            maximum_polling_duration: Duration::from_secs(10),
        })
        .expect("测试网关配置有效")
    }
}

impl Drop for 假设备授权提供者 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct 记录提示(std::sync::Mutex<Option<OidcDeviceAuthorizationPrompt>>);

impl OidcDeviceAuthorizationPromptSink for 记录提示 {
    fn present(
        &self,
        prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure> {
        *self.0.lock().expect("提示锁未中毒") = Some(prompt.clone());
        Ok(())
    }
}

async fn 发现文档(State(state): State<提供者状态>) -> Json<serde_json::Value> {
    Json(json!({
        "issuer": state.issuer,
        "authorization_endpoint": format!("{}/authorize", state.issuer),
        "device_authorization_endpoint": format!("{}/device", state.issuer),
        "token_endpoint": format!("{}/token", state.issuer),
        "jwks_uri": format!("{}/jwks", state.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "grant_types_supported": ["urn:ietf:params:oauth:grant-type:device_code"],
        "scopes_supported": ["openid", "profile"]
    }))
}

async fn 公钥集(State(state): State<提供者状态>) -> Json<CoreJsonWebKeySet> {
    let signing_key = 测试签名密钥(&state.signing_key_pem);
    Json(CoreJsonWebKeySet::new(vec![
        signing_key.as_verification_key(),
    ]))
}

async fn 创建设备码(body: Bytes) -> Response {
    let form = 表单(&body);
    let scopes = form.get("scope").map(String::as_str).unwrap_or_default();
    if form.get("client_id").map(String::as_str) != Some(CLIENT_ID)
        || !scopes
            .split_ascii_whitespace()
            .any(|scope| scope == "openid")
    {
        return oauth_错误("invalid_request", "缺少公开客户端或 openid scope");
    }
    Json(json!({
        "device_code": DEVICE_CODE,
        "user_code": USER_CODE,
        "verification_uri": "https://login.agent-room.test/device",
        "verification_uri_complete": format!("https://login.agent-room.test/device?user_code={USER_CODE}"),
        "expires_in": 60,
        "interval": 1
    }))
    .into_response()
}

async fn 轮询令牌(State(state): State<提供者状态>, body: Bytes) -> Response {
    let form = 表单(&body);
    if form.get("client_id").map(String::as_str) != Some(CLIENT_ID)
        || form.get("device_code").map(String::as_str) != Some(DEVICE_CODE)
        || form.get("grant_type").map(String::as_str)
            != Some("urn:ietf:params:oauth:grant-type:device_code")
    {
        return oauth_错误("invalid_grant", "设备码无效");
    }
    let mut poll_count = state.poll_count.lock().await;
    *poll_count += 1;
    if *poll_count == 1 {
        return oauth_错误("authorization_pending", "用户尚未完成授权");
    }
    Json(创建令牌响应(&state)).into_response()
}

fn oauth_错误(code: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": code, "error_description": description })),
    )
        .into_response()
}

fn 表单(body: &[u8]) -> HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

fn 创建令牌响应(state: &提供者状态) -> CoreTokenResponse {
    let now = Utc::now();
    let audience = match state.assertion_variant {
        断言变体::错误受众 => "another-device-client",
        _ => CLIENT_ID,
    };
    let authenticated_at = match state.assertion_variant {
        断言变体::复用旧登录 => Some(now - TimeDelta::days(1)),
        断言变体::省略登录时间 => None,
        _ => Some(now),
    };
    let expires_at = if matches!(state.assertion_variant, 断言变体::已过期) {
        now - TimeDelta::minutes(1)
    } else {
        now + TimeDelta::minutes(5)
    };
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(state.issuer.clone()).expect("测试 issuer 有效"),
        vec![Audience::new(audience.to_owned())],
        expires_at,
        now,
        StandardClaims::new(SubjectIdentifier::new("device-subject-42".to_owned())),
        EmptyAdditionalClaims {},
    )
    .set_auth_time(authenticated_at)
    .set_preferred_username(Some(EndUserUsername::new("设备测试用户".to_owned())))
    .set_locale(Some(LanguageTag::new("zh-CN".to_owned())));
    let id_token = CoreIdToken::new(
        claims,
        &测试签名密钥(&state.signing_key_pem),
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .expect("测试 ID Token 可签名");

    CoreTokenResponse::new(
        AccessToken::new("只在假提供者内部使用的访问令牌".to_owned()),
        CoreTokenType::Bearer,
        CoreIdTokenFields::new(Some(id_token), EmptyExtraTokenFields {}),
    )
}

fn 生成测试签名密钥() -> String {
    RsaPrivateKey::new(&mut OsRng, 2_048)
        .expect("测试 RSA 密钥可生成")
        .to_pkcs1_pem(LineEnding::LF)
        .expect("测试 RSA 密钥可编码")
        .to_string()
}

fn 测试签名密钥(pem: &str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(pem, Some(JsonWebKeyId::new(TEST_KEY_ID.to_owned())))
        .expect("测试 RSA 私钥有效")
}

#[tokio::test]
async fn 设备授权遵守轮询间隔并返回可验证的一次性身份断言() {
    let provider = 假设备授权提供者::启动(断言变体::有效).await;
    let gateway = provider.网关();
    let prompt_sink = 记录提示::default();

    let assertion = gateway
        .authorize(&prompt_sink)
        .await
        .expect("设备授权应在用户确认后成功");
    let prompt = prompt_sink
        .0
        .lock()
        .expect("提示锁未中毒")
        .clone()
        .expect("授权提示必须展示");
    let verified = gateway
        .verify_assertion(&assertion)
        .await
        .expect("公开客户端 ID Token 应通过签名与声明校验");
    let identity = verified.identity();

    assert_eq!(prompt.user_code.expose(), USER_CODE);
    assert_eq!(
        prompt.verification_uri_complete.as_deref(),
        Some("https://login.agent-room.test/device?user_code=ABCD-EFGH")
    );
    assert_eq!(identity.subject(), "device-subject-42");
    assert_eq!(identity.display_name(), Some("设备测试用户"));
    assert_eq!(identity.locale(), Some("zh-CN"));
    assert!(identity.authenticated_at().is_some());
    assert!(verified.issued_at().value() <= Utc::now().timestamp_millis());
}

#[tokio::test]
async fn 设备断言保留新签发时间并区分旧登录或可选登录时间() {
    for variant in [断言变体::复用旧登录, 断言变体::省略登录时间] {
        let provider = 假设备授权提供者::启动(variant).await;
        let gateway = provider.网关();
        let before = Utc::now().timestamp_millis() - 1_000;
        let assertion = gateway
            .authorize(&记录提示::default())
            .await
            .expect("授权成功");
        let verified = gateway
            .verify_assertion(&assertion)
            .await
            .expect("断言可验证");

        assert!(verified.issued_at().value() >= before);
        match variant {
            断言变体::复用旧登录 => assert!(
                verified
                    .identity()
                    .authenticated_at()
                    .expect("保留实际登录时间")
                    .value()
                    < verified.issued_at().value() - 23 * 60 * 60 * 1_000
            ),
            断言变体::省略登录时间 => {
                assert_eq!(verified.identity().authenticated_at(), None);
            }
            _ => unreachable!("只验证已有会话和省略登录时间"),
        }
    }
}

#[tokio::test]
async fn 过期设备断言仍被拒绝() {
    let provider = 假设备授权提供者::启动(断言变体::已过期).await;
    let gateway = provider.网关();
    let assertion = gateway
        .authorize(&记录提示::default())
        .await
        .expect("提供者签发测试断言");
    let failure = gateway
        .verify_assertion(&assertion)
        .await
        .expect_err("过期断言必须失败");
    assert_eq!(failure.kind(), OidcFailureKind::InvalidIdentityToken);
}

#[tokio::test]
async fn 设备断言的受众不是_bridge_客户端时必须拒绝() {
    let provider = 假设备授权提供者::启动(断言变体::错误受众).await;
    let gateway = provider.网关();
    let assertion = gateway
        .authorize(&记录提示::default())
        .await
        .expect("假提供者仍会签发断言");

    let failure = gateway
        .verify_assertion(&assertion)
        .await
        .expect_err("错误受众必须失败");

    assert_eq!(failure.kind(), OidcFailureKind::InvalidIdentityToken);
}
