use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{DeviceSignature, MatrixEventId, MatrixRoomId, PortFuture, SecretFactory, SecretValue},
};
use agent_room_bridge::control_plane::{
    ControlPlaneHttpConfig, ReqwestControlPlaneMessageContentGateway,
};
use agent_room_bridge_core::{
    messages::{
        MessageContentBindRequest, MessageContentGateway, MessageContentRedactRequest,
        MessageContentUploadRequest,
    },
    session::{AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    content::{ContentByteLength, ContentEncryptionMode, ContentMediaType, Sha256Digest},
    ids::{AgentId, ContentUploadRequestId, DeviceId},
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, post, put},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::net::TcpListener;
use uuid::Uuid;

const CONTENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const REQUEST_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const ROOM_ID: &str = "!lobby:matrix.test";
const EVENT_ID: &str = "$message:matrix.test";

#[derive(Default)]
struct 测试请求授权器 {
    requests: Mutex<Vec<(String, String, String)>>,
}

struct 测试正文服务状态 {
    digest: String,
}

impl ControlPlaneRequestAuthorizer for 测试请求授权器 {
    fn authorize<'a>(
        &'a self,
        method: &'a str,
        request_target: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>> {
        self.requests.lock().expect("授权记录锁可用").push((
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
async fn 消息正文写入完整执行声明上传绑定与撤回() {
    let payload = "正文".as_bytes().to_vec();
    let digest = hex_digest(&payload);
    let app = 测试正文服务(digest.clone());
    let authorizer = Arc::new(测试请求授权器::default());
    let gateway = gateway(spawn_server(app).await, authorizer.clone());
    let upload = upload_request(payload);

    let record = gateway.upload(&upload).await.expect("正文上传应成功");
    assert_eq!(record.content_id.to_string(), CONTENT_ID);
    gateway
        .bind(&MessageContentBindRequest {
            content_id: record.content_id,
            room_id: MatrixRoomId::new(ROOM_ID).expect("房间标识有效"),
            event_id: MatrixEventId::new(EVENT_ID).expect("事件标识有效"),
        })
        .await
        .expect("事件绑定应成功");
    gateway
        .redact(&MessageContentRedactRequest {
            content_id: record.content_id,
        })
        .await
        .expect("正文撤回应成功");

    let requests = authorizer.requests.lock().expect("授权记录锁可用");
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].0, "POST");
    assert_eq!(requests[0].1, "/content/uploads");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].2).expect("声明正文是 JSON"),
        json!({
            "actorAgentId": AGENT_ID,
            "matrixRoomId": ROOM_ID,
            "accessMode": "room_member",
            "sha256": digest,
            "byteLength": 6,
            "mediaType": "text/plain",
            "encryptionMode": "server_side",
            "expiresAtUnixMs": null
        })
    );
    assert_eq!(
        requests[1],
        (
            "PUT".to_owned(),
            format!("/content/{CONTENT_ID}/bytes"),
            format!("sha256={digest}\nbyte-length=6"),
        )
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[2].2).expect("绑定正文是 JSON"),
        json!({
            "matrixRoomId": ROOM_ID,
            "matrixEventId": EVENT_ID
        })
    );
    assert_eq!(requests[3].0, "DELETE");
    assert_eq!(requests[3].1, format!("/content/{CONTENT_ID}"));
    assert!(requests[3].2.is_empty());
}

fn 测试正文服务(digest: String) -> Router {
    Router::new()
        .route("/content/uploads", post(声明上传))
        .route("/content/{content_id}/bytes", put(完成上传))
        .route("/content/{content_id}/event-binding", put(绑定事件))
        .route("/content/{content_id}", delete(撤回正文))
        .with_state(Arc::new(测试正文服务状态 { digest }))
}

async fn 声明上传(
    State(state): State<Arc<测试正文服务状态>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let valid = signed_headers_are_valid(&headers)
        && header_value(&headers, "idempotency-key") == Some(REQUEST_ID)
        && body
            == json!({
                "actorAgentId": AGENT_ID,
                "matrixRoomId": ROOM_ID,
                "accessMode": "room_member",
                "sha256": state.digest,
                "byteLength": 6,
                "mediaType": "text/plain",
                "encryptionMode": "server_side",
                "expiresAtUnixMs": null
            });
    if !valid {
        return StatusCode::BAD_REQUEST.into_response();
    }
    (
        StatusCode::CREATED,
        Json(content_object_response(
            &state.digest,
            "uploading",
            "pending",
            &json!({
                "matrixRoomId": ROOM_ID,
                "accessMode": "room_member",
                "created": true
            }),
        )),
    )
        .into_response()
}

async fn 完成上传(
    State(state): State<Arc<测试正文服务状态>>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let valid = content_id == CONTENT_ID
        && signed_headers_are_valid(&headers)
        && header_value(&headers, "content-type") == Some("text/plain")
        && header_value(&headers, "content-length") == Some("6")
        && header_value(&headers, "x-agent-room-content-sha256") == Some(state.digest.as_str())
        && header_value(&headers, "x-agent-room-content-byte-length") == Some("6")
        && body.as_ref() == "正文".as_bytes();
    if !valid {
        return StatusCode::BAD_REQUEST.into_response();
    }
    Json(content_object_response(
        &state.digest,
        "active",
        "clean",
        &json!({ "alreadyActive": false }),
    ))
    .into_response()
}

async fn 绑定事件(
    Path(content_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let valid = content_id == CONTENT_ID
        && signed_headers_are_valid(&headers)
        && body
            == json!({
                "matrixRoomId": ROOM_ID,
                "matrixEventId": EVENT_ID
            });
    if !valid {
        return StatusCode::BAD_REQUEST.into_response();
    }
    Json(json!({
        "contentId": CONTENT_ID,
        "matrixRoomId": ROOM_ID,
        "matrixEventId": EVENT_ID,
        "accessMode": "room_member",
        "alreadyBound": false
    }))
    .into_response()
}

async fn 撤回正文(
    Path(content_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if content_id != CONTENT_ID || !signed_headers_are_valid(&headers) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    Json(json!({
        "contentId": CONTENT_ID,
        "lifecycleState": "redacted",
        "alreadyRedacted": false
    }))
    .into_response()
}

fn content_object_response(
    digest: &str,
    lifecycle_state: &str,
    scan_state: &str,
    extension: &Value,
) -> Value {
    let mut response = json!({
        "contentId": CONTENT_ID,
        "sha256": digest,
        "byteLength": 6,
        "mediaType": "text/plain",
        "encryptionMode": "server_side",
        "scanState": scan_state,
        "lifecycleState": lifecycle_state,
        "expiresAtUnixMs": null,
        "createdAtUnixMs": 1_000
    });
    let response_fields = response.as_object_mut().expect("测试响应是对象");
    response_fields.extend(
        extension
            .as_object()
            .expect("测试扩展是对象")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    response
}

fn upload_request(body: Vec<u8>) -> MessageContentUploadRequest {
    let digest = Sha256Digest::from_bytes(Sha256::digest(&body).into());
    MessageContentUploadRequest {
        request_id: ContentUploadRequestId::from_uuid(
            Uuid::parse_str(REQUEST_ID).expect("上传请求标识有效"),
        ),
        room_id: MatrixRoomId::new(ROOM_ID).expect("房间标识有效"),
        digest,
        byte_length: ContentByteLength::new(6).expect("正文长度有效"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        expires_at: None,
        body: Arc::from(body),
    }
}

fn gateway(
    base_url: String,
    authorizer: Arc<测试请求授权器>,
) -> ReqwestControlPlaneMessageContentGateway {
    ReqwestControlPlaneMessageContentGateway::new(
        &ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        },
        authorizer,
        AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("Agent 标识有效")),
    )
    .expect("本地消息正文网关地址有效")
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

fn signed_headers_are_valid(headers: &HeaderMap) -> bool {
    header_value(headers, "authorization") == Some("Bearer access-token")
        && header_value(headers, "x-agent-room-device-id")
            == Some("00000000-0000-0000-0000-000000000001")
        && header_value(headers, "x-agent-room-proof-issued-at") == Some("1000")
        && header_value(headers, "x-agent-room-proof-nonce") == Some("0123456789abcdef")
        && header_value(headers, "x-agent-room-proof-signature")
            == Some(
                "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQ",
            )
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn hex_digest(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn secret(value: &str) -> SecretValue {
    SecretValue::new(value.to_owned()).expect("测试密文有效")
}
