use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    account_encryption::{AccountEncryptionDependencies, AccountEncryptionService},
    authentication::{
        AuthenticatedPrincipal, AuthenticationResult, BeginLogin, CompleteLogin, LoginCompletion,
        LoginRedirect,
    },
    persistence::RepositoryResult,
    ports::{
        AccountEncryptionKeyRepository, Clock, MatrixCrossSigningResetGateway, MatrixResult,
        MatrixUserId, PortFuture, SecretValue, StoredEncryptionKey,
    },
};
use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use agent_room_identity_adapter::{AesGcmAccountEncryptionKeySealer, NetworkAgentSealKey};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware,
    response::Response,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";
const ORIGIN: &str = "https://app.agent-room.test";
const KEY_BASE64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";

#[derive(Default)]
struct Keys(Mutex<HashMap<PrincipalId, StoredEncryptionKey>>);

impl AccountEncryptionKeyRepository for Keys {
    fn find_encryption_key(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<StoredEncryptionKey>>> {
        let found = self.0.lock().unwrap().get(&principal_id).cloned();
        Box::pin(async move { Ok(found) })
    }

    fn put_encryption_key<'a>(
        &'a self,
        principal_id: PrincipalId,
        key: &'a StoredEncryptionKey,
        _updated_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.0.lock().unwrap().insert(principal_id, key.clone());
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct Synapse(Mutex<Vec<String>>);

impl MatrixCrossSigningResetGateway for Synapse {
    fn allow_cross_signing_replacement<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.0.lock().unwrap().push(user_id.as_str().to_owned());
        Box::pin(async { Ok(()) })
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000).unwrap()
    }
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("保管钥匙不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("保管钥匙不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(
            requirement,
            AuthenticationRequirement::ActiveSession,
            "桌面端重启后自动签名，不能要求刚登录过"
        );
        Box::pin(async {
            Ok(AuthenticatedPrincipal {
                principal_id: PrincipalId::from_uuid(
                    Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e42").unwrap(),
                ),
                matrix_user_id: "@rainy:matrix.agent-room.test".to_owned(),
                display_name: "Rainy".to_owned(),
                locale: "zh-CN".to_owned(),
                authenticated_at: UtcMillis::new(1_790_000_000_000).unwrap(),
                expires_at: UtcMillis::new(1_790_000_060_000).unwrap(),
                recently_authenticated: false,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("保管钥匙不会退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("保管钥匙不会暂停主体") })
    }
}

fn app(synapse: Arc<Synapse>) -> Router {
    let service = Arc::new(AccountEncryptionService::new(
        AccountEncryptionDependencies {
            repository: Arc::new(Keys::default()),
            sealer: Arc::new(AesGcmAccountEncryptionKeySealer::new(Some(
                &NetworkAgentSealKey::from_bytes([3; 32]),
            ))),
            matrix: synapse,
            clock: Arc::new(FixedClock),
        },
    ));
    router(AccountEncryptionHttpState::new(
        service,
        Arc::new(FakeAuthentication),
        &url::Url::parse(ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(method: Method, path: &str, origin: bool, body: Option<Value>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, SESSION_COOKIE);
    if origin {
        request = request.header(header::ORIGIN, ORIGIN);
    }
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 没登录不给_还没有钥匙时说清楚是没有() {
    let app = app(Arc::default());
    let anonymous = Request::builder()
        .uri("/account/encryption-key")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(anonymous).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    let missing = app
        .oneshot(request(Method::GET, "/account/encryption-key", false, None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(missing).await["code"],
        "account.encryption_key_missing"
    );
}

#[tokio::test]
async fn 存下的钥匙原样取回_不缓存() {
    let app = app(Arc::default());
    let stored = app
        .clone()
        .oneshot(request(
            Method::PUT,
            "/account/encryption-key",
            true,
            Some(json!({ "keyId": "KEY1", "key": KEY_BASE64 })),
        ))
        .await
        .unwrap();
    assert_eq!(stored.status(), StatusCode::NO_CONTENT);

    let found = app
        .oneshot(request(Method::GET, "/account/encryption-key", false, None))
        .await
        .unwrap();
    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(found.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        body_json(found).await,
        json!({ "schemaVersion": 1, "keyId": "KEY1", "key": KEY_BASE64 })
    );
}

#[tokio::test]
async fn 改动要校验来源_钥匙不对就不收() {
    let app = app(Arc::default());
    for path in ["/account/encryption-key", "/account/encryption-reset"] {
        let method = if path.ends_with("key") {
            Method::PUT
        } else {
            Method::POST
        };
        let body = path
            .ends_with("key")
            .then(|| json!({ "keyId": "KEY1", "key": KEY_BASE64 }));
        let response = app
            .clone()
            .oneshot(request(method, path, false, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_eq!(body_json(response).await["code"], "account.invalid_origin");
    }

    for body in [
        json!({ "keyId": "KEY1", "key": "not base64!" }),
        json!({ "keyId": "KEY1", "key": "AAAA" }),
        json!({ "keyId": "", "key": KEY_BASE64 }),
        json!({ "keyId": "KEY1", "key": KEY_BASE64, "extra": true }),
    ] {
        let response = app
            .clone()
            .oneshot(request(
                Method::PUT,
                "/account/encryption-key",
                true,
                Some(body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(response).await["code"],
            "account.encryption_key_invalid"
        );
    }
}

#[tokio::test]
async fn 重建签名身份前开豁免_一小时第四次挡下并告诉什么时候再来() {
    let synapse = Arc::new(Synapse::default());
    let app = app(synapse.clone());
    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/account/encryption-reset",
                true,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(
        *synapse.0.lock().unwrap(),
        vec!["@rainy:matrix.agent-room.test".to_owned(); 3]
    );

    let limited = app
        .oneshot(request(
            Method::POST,
            "/account/encryption-reset",
            true,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(
        body_json(limited).await["code"],
        "account.encryption_reset_rate_limited"
    );
    assert_eq!(synapse.0.lock().unwrap().len(), 3);
}
