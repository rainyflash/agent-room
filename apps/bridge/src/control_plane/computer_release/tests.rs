use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
};
use agent_room_bridge_core::session::{
    AuthorizedControlPlaneRequest, BridgeSessionFailureKind, BridgeSessionResult,
    ControlPlaneRequestAuthorizer,
};
use agent_room_domain::{ids::DeviceId, time::UtcMillis};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

use super::{
    ComputerReleaseOutcome, ControlPlaneHttpConfig, RELEASE_REJECTED, RELEASE_UNAVAILABLE,
    RELEASE_UNSUPPORTED, ReqwestComputerRelease, outcome_for_session_failure,
};

#[derive(Default)]
struct 测试请求授权器 {
    requests: Mutex<Vec<(String, String, String)>>,
}

impl ControlPlaneRequestAuthorizer for 测试请求授权器 {
    fn authorize<'a>(
        &'a self,
        method: &'a str,
        request_target: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>> {
        self.requests.lock().expect("授权请求记录锁可用").push((
            method.to_owned(),
            request_target.to_owned(),
            body.to_owned(),
        ));
        let payload = DeviceRequestProofPayload::new(
            DeviceId::from_uuid(Uuid::from_u128(1)),
            UtcMillis::new(1_000).expect("测试时间有效"),
            secret("0123456789abcdef"),
            method.to_owned(),
            request_target.to_owned(),
            SecureSecretFactory.digest(body),
        )
        .expect("测试设备证明有效");
        Box::pin(async move {
            Ok(AuthorizedControlPlaneRequest {
                access_token: secret("access-token"),
                proof: DeviceRequestProof::new(
                    payload,
                    DeviceSignature::new(vec![5; 64]).expect("测试签名有效"),
                ),
            })
        })
    }
}

#[tokio::test]
async fn 用这台电脑的签名请控制面移出它_204_和_202_都算移出去了() {
    for status in [StatusCode::NO_CONTENT, StatusCode::ACCEPTED] {
        let app = Router::new().route(
            "/auth/devices/current/release",
            post(move |headers: HeaderMap, body: String| async move {
                let signed = header(&headers, "authorization") == Some("Bearer access-token")
                    && header(&headers, "x-agent-room-device-id")
                        == Some("00000000-0000-0000-0000-000000000001")
                    && header(&headers, "x-agent-room-proof-nonce") == Some("0123456789abcdef");
                if signed && body.is_empty() {
                    status.into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
        let authorizer = Arc::new(测试请求授权器::default());
        let outcome = release(spawn_server(app).await, authorizer.clone())
            .release()
            .await
            .expect("移出这台电脑");

        assert_eq!(outcome, ComputerReleaseOutcome::Released, "{status}");
        assert_eq!(
            authorizer
                .requests
                .lock()
                .expect("授权请求记录锁可用")
                .as_slice(),
            [(
                "POST".to_owned(),
                "/auth/devices/current/release".to_owned(),
                String::new(),
            )]
        );
    }
}

#[tokio::test]
async fn 凭据已经作废算早就移出去了_签名不对和暂时出错不算() {
    for (status, code, expected) in [
        (
            StatusCode::UNAUTHORIZED,
            Some("device.invalid_token"),
            Ok(ComputerReleaseOutcome::AlreadyReleased),
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("device.refresh_token_reuse"),
            Ok(ComputerReleaseOutcome::AlreadyReleased),
        ),
        (
            StatusCode::FORBIDDEN,
            Some("device.revoked"),
            Ok(ComputerReleaseOutcome::AlreadyReleased),
        ),
        (
            StatusCode::FORBIDDEN,
            Some("device.principal_suspended"),
            Ok(ComputerReleaseOutcome::AlreadyReleased),
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("device.invalid_proof"),
            Err(RELEASE_REJECTED),
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("device.proof_replay"),
            Err(RELEASE_REJECTED),
        ),
        (StatusCode::UNAUTHORIZED, None, Err(RELEASE_REJECTED)),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("device.dependency_unavailable"),
            Err(RELEASE_UNAVAILABLE),
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Some("rate_limited"),
            Err(RELEASE_UNAVAILABLE),
        ),
    ] {
        let app = Router::new().route(
            "/auth/devices/current/release",
            post(move || async move {
                match code {
                    Some(code) => (status, Json(json!({"code": code}))).into_response(),
                    None => status.into_response(),
                }
            }),
        );
        let outcome = release(spawn_server(app).await, Arc::new(测试请求授权器::default()))
            .release()
            .await
            .map_err(super::ComputerReleaseFailure::code);

        assert_eq!(outcome, expected, "{status} {code:?}");
    }
}

#[tokio::test]
async fn 还没有这个接口的控制面报不支持_连不上报暂时不可用() {
    let old = release(
        spawn_server(Router::new()).await,
        Arc::new(测试请求授权器::default()),
    );
    assert_eq!(
        old.release()
            .await
            .map_err(super::ComputerReleaseFailure::code),
        Err(RELEASE_UNSUPPORTED)
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试监听器可绑定");
    let address = listener.local_addr().expect("测试地址存在");
    drop(listener);
    let unreachable = release(
        format!("http://{address}/"),
        Arc::new(测试请求授权器::default()),
    );
    assert_eq!(
        unreachable
            .release()
            .await
            .map_err(super::ComputerReleaseFailure::code),
        Err(RELEASE_UNAVAILABLE)
    );
}

#[test]
fn 设备会话已经作废时不发请求_算早就移出去了() {
    assert_eq!(
        outcome_for_session_failure(BridgeSessionFailureKind::NotAuthorized),
        Ok(ComputerReleaseOutcome::AlreadyReleased)
    );
    for kind in [
        BridgeSessionFailureKind::RefreshOutcomeUnknown,
        BridgeSessionFailureKind::ControlPlaneUnavailable,
        BridgeSessionFailureKind::SecureStorageUnavailable,
    ] {
        assert_eq!(
            outcome_for_session_failure(kind).map_err(super::ComputerReleaseFailure::code),
            Err(RELEASE_UNAVAILABLE),
            "{kind:?}"
        );
    }
    for kind in [
        BridgeSessionFailureKind::CorruptSecureStorage,
        BridgeSessionFailureKind::InvalidControlPlaneResponse,
        BridgeSessionFailureKind::Internal,
    ] {
        assert_eq!(
            outcome_for_session_failure(kind).map_err(super::ComputerReleaseFailure::code),
            Err(RELEASE_REJECTED),
            "{kind:?}"
        );
    }
}

fn release(base_url: String, authorizer: Arc<测试请求授权器>) -> ReqwestComputerRelease {
    ReqwestComputerRelease::new(
        &ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        },
        authorizer,
    )
    .expect("本地控制面地址有效")
}

async fn spawn_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试监听器可绑定");
    let address = listener.local_addr().expect("测试地址存在");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试服务器应正常运行");
    });
    format!("http://{address}/")
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn secret(value: &str) -> SecretValue {
    SecretValue::new(value).expect("测试秘密有效")
}
