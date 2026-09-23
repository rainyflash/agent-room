use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
};
use agent_room_bridge_core::{
    join_codes::{ControlPlaneJoinCodeGateway, JoinCodeFailureKind},
    session::{AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{ids::AgentId, ids::DeviceId, time::UtcMillis};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::{ControlPlaneHttpConfig, ReqwestControlPlaneJoinCodeGateway};

const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

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
async fn 查看与兑换各走各的路径_用同一正文签名并解析房间() {
    let signed = |headers: &HeaderMap, body: &str| {
        header(headers, "authorization") == Some("Bearer access-token")
            && header(headers, "x-agent-room-device-id")
                == Some("00000000-0000-0000-0000-000000000001")
            && serde_json::from_str::<Value>(body).ok() == Some(json!({"code": "K7P3-Q9XW-2DMA"}))
    };
    let app = Router::new()
        .route(
            "/join-codes/resolve",
            post(move |headers: HeaderMap, body: String| async move {
                if signed(&headers, &body) {
                    (StatusCode::OK, Json(room_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        )
        .route(
            "/agents/{agent_id}/join-codes/redeem",
            post(
                move |Path(agent_id): Path<String>, headers: HeaderMap, body: String| async move {
                    if agent_id == AGENT_ID && signed(&headers, &body) {
                        (StatusCode::OK, Json(room_response())).into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                },
            ),
        );
    let authorizer = Arc::new(测试请求授权器::default());
    let gateway = gateway(spawn_server(app).await, authorizer.clone());

    let resolved = gateway.resolve("K7P3-Q9XW-2DMA").await.expect("查看房间");
    assert_eq!(resolved.catalog_id.to_string(), CATALOG_ID);
    assert_eq!(resolved.matrix_room_id.as_str(), "!project:matrix.test");
    assert_eq!(resolved.name, "项目室");
    let redeemed = gateway
        .redeem(agent_id(), "K7P3-Q9XW-2DMA")
        .await
        .expect("兑换口令");
    assert_eq!(redeemed, resolved);
    let requests = authorizer.requests.lock().expect("授权请求记录锁可用");
    let targets: Vec<_> = requests
        .iter()
        .map(|(method, target, _)| (method.as_str(), target.as_str()))
        .collect();
    assert_eq!(
        targets,
        [
            ("POST", "/join-codes/resolve"),
            (
                "POST",
                format!("/agents/{AGENT_ID}/join-codes/redeem").as_str()
            ),
        ]
    );
}

#[tokio::test]
async fn 口令错误按错误码区分_没有口令接口的控制面报不支持() {
    for (status, code, expected) in [
        (
            StatusCode::BAD_REQUEST,
            Some("join_code.invalid"),
            JoinCodeFailureKind::InvalidCode,
        ),
        (
            StatusCode::NOT_FOUND,
            Some("join_code.not_found"),
            JoinCodeFailureKind::NotFound,
        ),
        (
            StatusCode::FORBIDDEN,
            Some("join_code.forbidden"),
            JoinCodeFailureKind::Forbidden,
        ),
        (
            StatusCode::CONFLICT,
            Some("join_code.conflict"),
            JoinCodeFailureKind::RoomUnavailable,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Some("join_code.rate_limited"),
            JoinCodeFailureKind::RateLimited,
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("device.authentication_required"),
            JoinCodeFailureKind::NotAuthorized,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("join_code.dependency_unavailable"),
            JoinCodeFailureKind::ControlPlaneUnavailable,
        ),
    ] {
        let app = Router::new().route(
            "/join-codes/resolve",
            post(move || async move {
                (
                    status,
                    Json(json!({"code": code, "category": "validation"})),
                )
            }),
        );
        let failure = gateway(spawn_server(app).await, Arc::new(测试请求授权器::default()))
            .resolve("K7P3-Q9XW-2DMA")
            .await
            .expect_err("控制面拒绝");
        assert_eq!(failure.kind(), expected, "{status} {code:?}");
    }
    let old = gateway(
        spawn_server(Router::new()).await,
        Arc::new(测试请求授权器::default()),
    );
    assert_eq!(
        old.redeem(agent_id(), "K7P3-Q9XW-2DMA")
            .await
            .expect_err("旧控制面没有口令接口")
            .kind(),
        JoinCodeFailureKind::Unsupported
    );
}

#[tokio::test]
async fn 房间字段校验后才交给调用方() {
    for invalid in [
        json!({"catalogId": "not-a-uuid", "matrixRoomId": "!project:matrix.test", "name": "项目室"}),
        json!({"catalogId": "0198b601-77a1-4bb8-83eb-a8fe68c97e46", "matrixRoomId": "!project:matrix.test", "name": "项目室"}),
        json!({"catalogId": CATALOG_ID, "matrixRoomId": "project", "name": "项目室"}),
        json!({"catalogId": CATALOG_ID, "matrixRoomId": "!project:matrix.test", "name": "  "}),
        json!({"catalogId": CATALOG_ID}),
    ] {
        let app = Router::new().route(
            "/join-codes/resolve",
            post(move || {
                let invalid = invalid.clone();
                async move { (StatusCode::OK, Json(invalid)) }
            }),
        );
        let failure = gateway(spawn_server(app).await, Arc::new(测试请求授权器::default()))
            .resolve("K7P3-Q9XW-2DMA")
            .await
            .expect_err("回答不合规");
        assert_eq!(
            failure.kind(),
            JoinCodeFailureKind::InvalidControlPlaneResponse
        );
    }
}

fn gateway(
    base_url: String,
    authorizer: Arc<测试请求授权器>,
) -> ReqwestControlPlaneJoinCodeGateway {
    ReqwestControlPlaneJoinCodeGateway::new(
        &ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        },
        authorizer,
    )
    .expect("本地口令网关地址有效")
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

fn room_response() -> Value {
    json!({
        "catalogId": CATALOG_ID,
        "matrixRoomId": "!project:matrix.test",
        "name": "项目室"
    })
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("测试 UUID 有效"))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn secret(value: &str) -> SecretValue {
    SecretValue::new(value).expect("测试秘密有效")
}
