use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
};
use agent_room_bridge_core::{
    room_management::{
        ControlPlaneRoomManagementGateway, RoomCodeState, RoomManagementFailureKind,
    },
    session::{AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method as HttpMethod, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::{ControlPlaneHttpConfig, ReqwestControlPlaneRoomManagementGateway};

const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const ACCOUNT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

#[derive(Default)]
struct 测试请求授权器 {
    requests: Mutex<Vec<(String, String, String)>>,
}

impl 测试请求授权器 {
    fn signed(&self) -> Vec<(String, String, String)> {
        self.requests.lock().expect("授权请求记录锁可用").clone()
    }
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

/// 收到的请求：方法、路径、幂等键和正文。
type 收到的请求 = (String, String, Option<String>, String);

/// 记下收到的请求；回答按顺序给，给完了回答 500。
#[derive(Default)]
struct 假控制面 {
    received: Mutex<Vec<收到的请求>>,
    answers: Mutex<VecDeque<(StatusCode, Value)>>,
}

impl 假控制面 {
    fn answering(answers: impl IntoIterator<Item = (StatusCode, Value)>) -> Arc<Self> {
        Arc::new(Self {
            received: Mutex::default(),
            answers: Mutex::new(answers.into_iter().collect()),
        })
    }

    fn received(&self) -> Vec<收到的请求> {
        self.received.lock().expect("请求记录锁可用").clone()
    }
}

async fn answer(
    State(control_plane): State<Arc<假控制面>>,
    method: HttpMethod,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signed = header(&headers, "authorization") == Some("Bearer access-token")
        && header(&headers, "x-agent-room-device-id")
            == Some("00000000-0000-0000-0000-000000000001");
    control_plane
        .received
        .lock()
        .expect("请求记录锁可用")
        .push((
            method.to_string(),
            uri.path().to_owned(),
            header(&headers, "idempotency-key").map(str::to_owned),
            String::from_utf8(body.to_vec()).expect("正文是 UTF-8"),
        ));
    if !signed {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (status, value) = control_plane
        .answers
        .lock()
        .expect("回答锁可用")
        .pop_front()
        .unwrap_or((StatusCode::INTERNAL_SERVER_ERROR, Value::Null));
    if value.is_null() {
        status.into_response()
    } else {
        (status, Json(value)).into_response()
    }
}

#[tokio::test]
async fn 建房拿房间_id_当幂等键_签名覆盖方法路径和正文() {
    let control_plane = 假控制面::answering([(
        StatusCode::CREATED,
        json!({
            "catalogId": CATALOG_ID,
            "matrixRoomId": "!weekly:matrix.test",
            "name": "周会",
        }),
    )]);
    let authorizer = Arc::new(测试请求授权器::default());
    let gateway = gateway(spawn(control_plane.clone()).await, authorizer.clone());

    let created = gateway
        .create(catalog_id(), "周会", agent_id())
        .await
        .expect("建房");

    assert_eq!(created.catalog_id, catalog_id());
    assert_eq!(created.matrix_room_id.as_str(), "!weekly:matrix.test");
    assert_eq!(created.name, "周会");
    let received = control_plane.received();
    assert_eq!(received.len(), 1);
    let (method, path, key, body) = &received[0];
    assert_eq!(
        (method.as_str(), path.as_str(), key.as_deref()),
        ("POST", "/devices/current/private-rooms", Some(CATALOG_ID))
    );
    assert_eq!(
        serde_json::from_str::<Value>(body).expect("正文是 JSON"),
        json!({"name": "周会", "agentId": AGENT_ID})
    );
    assert_eq!(
        authorizer.signed(),
        [(
            "POST".to_owned(),
            "/devices/current/private-rooms".to_owned(),
            body.clone()
        )]
    );
}

#[tokio::test]
async fn 口令能看就给_没有的和看不到的分开说_生成和停用各走各的方法() {
    let code = || json!({"code": "K7P3-Q9XW-2DMA", "createdAtUnixMs": 1_700_000_000_000_i64});
    let control_plane = 假控制面::answering([
        (StatusCode::OK, code()),
        (
            StatusCode::NOT_FOUND,
            json!({"code": "join_code.not_found"}),
        ),
        (
            StatusCode::NOT_FOUND,
            json!({"code": "join_code.not_revealable"}),
        ),
        (StatusCode::OK, code()),
        (StatusCode::NO_CONTENT, Value::Null),
    ]);
    let authorizer = Arc::new(测试请求授权器::default());
    let gateway = gateway(spawn(control_plane.clone()).await, authorizer.clone());

    let RoomCodeState::Visible(visible) = gateway.code(catalog_id()).await.expect("看口令")
    else {
        panic!("能看到口令");
    };
    assert_eq!(visible.code, "K7P3-Q9XW-2DMA");
    assert_eq!(visible.created_at.value(), 1_700_000_000_000);
    assert!(
        !format!("{visible:?}").contains("K7P3"),
        "调试输出里不出现口令"
    );
    assert_eq!(
        gateway.code(catalog_id()).await.expect("没有口令"),
        RoomCodeState::Missing
    );
    assert_eq!(
        gateway.code(catalog_id()).await.expect("看不到口令"),
        RoomCodeState::Hidden
    );
    let generated = gateway.generate_code(catalog_id()).await.expect("生成口令");
    assert_eq!(generated, visible);
    gateway.disable_code(catalog_id()).await.expect("停用口令");

    let code_path = format!("/devices/current/private-rooms/{CATALOG_ID}/code");
    let requests: Vec<_> = control_plane
        .received()
        .into_iter()
        .map(|(method, path, _, body)| (method, path, body))
        .collect();
    let expected: Vec<_> = ["GET", "GET", "GET", "PUT", "DELETE"]
        .into_iter()
        .map(|method| (method.to_owned(), code_path.clone(), String::new()))
        .collect();
    assert_eq!(requests, expected);
    assert_eq!(
        authorizer.signed(),
        expected,
        "签名覆盖原样的方法、路径和空正文"
    );
}

#[tokio::test]
async fn 敲门列出来_放行用_put_不让进用_delete_邀请和移出各走各的路径() {
    let control_plane = 假控制面::answering([
        (
            StatusCode::OK,
            json!({"knocks": [{
                "agentId": AGENT_ID,
                "displayName": "Sol",
                "knockedAtUnixMs": 1_700_000_300_000_i64,
                "expiresAtUnixMs": 1_700_003_900_000_i64,
            }]}),
        ),
        (StatusCode::OK, json!({"agent": {"agentId": AGENT_ID}})),
        (StatusCode::NO_CONTENT, Value::Null),
        (StatusCode::NO_CONTENT, Value::Null),
        (StatusCode::NO_CONTENT, Value::Null),
    ]);
    let gateway = gateway(
        spawn(control_plane.clone()).await,
        Arc::new(测试请求授权器::default()),
    );

    let knocks = gateway.knocks(catalog_id()).await.expect("看敲门");
    assert_eq!(knocks.len(), 1);
    assert_eq!(knocks[0].agent_id, agent_id());
    assert_eq!(knocks[0].display_name, "Sol");
    assert_eq!(knocks[0].expires_at.value(), 1_700_003_900_000);
    gateway
        .answer_knock(catalog_id(), agent_id(), true)
        .await
        .expect("放行");
    gateway
        .answer_knock(catalog_id(), agent_id(), false)
        .await
        .expect("不让进");
    gateway
        .invite(catalog_id(), account_id())
        .await
        .expect("邀请人");
    gateway
        .remove_agent(catalog_id(), agent_id())
        .await
        .expect("移出 Agent");

    let room = format!("/devices/current/private-rooms/{CATALOG_ID}");
    let requests: Vec<_> = control_plane
        .received()
        .into_iter()
        .map(|(method, path, _, body)| (method, path, body))
        .collect();
    assert_eq!(
        requests,
        [
            ("GET".to_owned(), format!("{room}/knocks"), String::new()),
            (
                "PUT".to_owned(),
                format!("{room}/knocks/{AGENT_ID}"),
                String::new()
            ),
            (
                "DELETE".to_owned(),
                format!("{room}/knocks/{AGENT_ID}"),
                String::new()
            ),
            (
                "POST".to_owned(),
                format!("{room}/invitations"),
                json!({"accountId": ACCOUNT_ID}).to_string()
            ),
            (
                "DELETE".to_owned(),
                format!("{room}/agents/{AGENT_ID}"),
                String::new()
            ),
        ]
    );
}

#[tokio::test]
async fn 错误码按点号后面那段认_没有这些接口的旧控制面报不支持() {
    for (status, code, expected) in [
        (
            StatusCode::BAD_REQUEST,
            "private_room.invalid_creation_body",
            RoomManagementFailureKind::InvalidRequest,
        ),
        (
            StatusCode::BAD_REQUEST,
            "join_code.invalid",
            RoomManagementFailureKind::InvalidRequest,
        ),
        (
            StatusCode::FORBIDDEN,
            "private_room.forbidden",
            RoomManagementFailureKind::Forbidden,
        ),
        (
            StatusCode::FORBIDDEN,
            "agent_knock.forbidden",
            RoomManagementFailureKind::Forbidden,
        ),
        (
            StatusCode::NOT_FOUND,
            "agent_knock.not_found",
            RoomManagementFailureKind::NotFound,
        ),
        (
            StatusCode::CONFLICT,
            "private_room.conflict",
            RoomManagementFailureKind::Conflict,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "join_code.rate_limited",
            RoomManagementFailureKind::RateLimited,
        ),
        (
            StatusCode::UNAUTHORIZED,
            "device.invalid_bearer",
            RoomManagementFailureKind::NotAuthorized,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "private_room.agent_entry_unavailable",
            RoomManagementFailureKind::ControlPlaneUnavailable,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "private_room.internal",
            RoomManagementFailureKind::ControlPlaneUnavailable,
        ),
    ] {
        let control_plane = 假控制面::answering([(status, json!({"code": code}))]);
        let failure = gateway(
            spawn(control_plane).await,
            Arc::new(测试请求授权器::default()),
        )
        .knocks(catalog_id())
        .await
        .expect_err("控制面拒绝");
        assert_eq!(failure.kind(), expected, "{status} {code}");
    }

    let old = gateway(
        spawn_router(Router::new()).await,
        Arc::new(测试请求授权器::default()),
    );
    assert_eq!(
        old.create(catalog_id(), "周会", agent_id())
            .await
            .expect_err("旧控制面没有这些接口")
            .kind(),
        RoomManagementFailureKind::Unsupported
    );
}

#[tokio::test]
async fn 回答不合规就不交给调用方() {
    let created = |catalog: &str, room: &str, name: &str| json!({"catalogId": catalog, "matrixRoomId": room, "name": name});
    for invalid in [
        created("not-a-uuid", "!weekly:matrix.test", "周会"),
        created(
            "0198b601-77a1-4bb8-83eb-a8fe68c97e46",
            "!weekly:matrix.test",
            "周会",
        ),
        created(CATALOG_ID, "weekly", "周会"),
        created(CATALOG_ID, "!weekly:matrix.test", "  "),
    ] {
        let control_plane = 假控制面::answering([(StatusCode::CREATED, invalid)]);
        let failure = gateway(
            spawn(control_plane).await,
            Arc::new(测试请求授权器::default()),
        )
        .create(catalog_id(), "周会", agent_id())
        .await
        .expect_err("建房回答不合规");
        assert_eq!(
            failure.kind(),
            RoomManagementFailureKind::InvalidControlPlaneResponse
        );
    }

    for invalid in [
        json!({"code": "", "createdAtUnixMs": 1}),
        json!({"code": "rm -rf /", "createdAtUnixMs": 1}),
        json!({"code": "K7P3-Q9XW-2DMA"}),
    ] {
        let control_plane = 假控制面::answering([(StatusCode::OK, invalid)]);
        let failure = gateway(
            spawn(control_plane).await,
            Arc::new(测试请求授权器::default()),
        )
        .generate_code(catalog_id())
        .await
        .expect_err("口令回答不合规");
        assert_eq!(
            failure.kind(),
            RoomManagementFailureKind::InvalidControlPlaneResponse
        );
    }

    let knock = |agent: &str, name: &str| {
        json!({"knocks": [{
            "agentId": agent,
            "displayName": name,
            "knockedAtUnixMs": 1,
            "expiresAtUnixMs": 2,
        }]})
    };
    for invalid in [knock("not-a-uuid", "Sol"), knock(AGENT_ID, " ")] {
        let control_plane = 假控制面::answering([(StatusCode::OK, invalid)]);
        let failure = gateway(
            spawn(control_plane).await,
            Arc::new(测试请求授权器::default()),
        )
        .knocks(catalog_id())
        .await
        .expect_err("敲门回答不合规");
        assert_eq!(
            failure.kind(),
            RoomManagementFailureKind::InvalidControlPlaneResponse
        );
    }
}

fn gateway(
    base_url: String,
    authorizer: Arc<测试请求授权器>,
) -> ReqwestControlPlaneRoomManagementGateway {
    ReqwestControlPlaneRoomManagementGateway::new(
        &ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        },
        authorizer,
    )
    .expect("本地管房间网关地址有效")
}

async fn spawn(control_plane: Arc<假控制面>) -> String {
    spawn_router(
        Router::new()
            .route("/devices/current/private-rooms", any(answer))
            .route("/devices/current/private-rooms/{*rest}", any(answer))
            .with_state(control_plane),
    )
    .await
}

async fn spawn_router(app: Router) -> String {
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

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_ID).expect("测试 UUID 有效"))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("测试 UUID 有效"))
}

fn account_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(ACCOUNT_ID).expect("测试 UUID 有效"))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn secret(value: &str) -> SecretValue {
    SecretValue::new(value).expect("测试秘密有效")
}
