use std::sync::Arc;

use agent_room_bridge_core::{
    handoffs::{
        TargetedHandoffQueueFailure, TargetedHandoffQueueFailureKind, TargetedHandoffQueueGateway,
        TargetedHandoffReceipt, TargetedHandoffTarget,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        HandoffContentReference, HandoffFailureCode, HandoffPermission, HandoffPermissions,
        HandoffPurpose, HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields,
        TargetedHandoffStatus,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::{Uuid, Version};

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    signed_request_headers,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1_024;

pub struct ReqwestTargetedHandoffQueueGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestTargetedHandoffQueueGateway {
    /// 创建只访问固定云端交接队列端点的签名网关。
    ///
    /// # Errors
    ///
    /// 控制面地址、明文传输边界、超时或 HTTP 客户端配置无效时返回错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        Ok(Self {
            client,
            base_url,
            authorizer,
        })
    }

    async fn claim_internal(
        &self,
        target: TargetedHandoffTarget,
    ) -> Result<Option<TargetedHandoff>, TargetedHandoffQueueFailure> {
        let request_target = format!("/agent-instances/{}/handoffs/claim", target.instance_id);
        let response = self
            .signed_request("POST", &request_target, "")
            .await?
            .body(Vec::new())
            .send()
            .await
            .map_err(|_| failure(TargetedHandoffQueueFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(status_failure(response.status()));
        }
        let decoded = decode_json::<ClaimResponse>(response).await?;
        let Some(handoff) = decoded.handoff else {
            return Ok(None);
        };
        let handoff = handoff.into_domain()?;
        ensure_target(&handoff, target)?;
        if handoff.status() != TargetedHandoffStatus::Delivered {
            return Err(failure(TargetedHandoffQueueFailureKind::InvalidResponse));
        }
        Ok(Some(handoff))
    }

    async fn receipt_internal(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
        receipt: &TargetedHandoffReceipt,
    ) -> Result<TargetedHandoff, TargetedHandoffQueueFailure> {
        let request_target = format!(
            "/agent-instances/{}/handoffs/{handoff_id}/receipt",
            target.instance_id
        );
        let body = serde_json::to_string(&ReceiptBody::from(receipt))
            .map_err(|_| failure(TargetedHandoffQueueFailureKind::InvalidResponse))?;
        let response = self
            .signed_request("PUT", &request_target, &body)
            .await?
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| failure(TargetedHandoffQueueFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(status_failure(response.status()));
        }
        let handoff = decode_json::<HandoffResponse>(response)
            .await?
            .into_domain()?;
        ensure_target(&handoff, target)?;
        if handoff.fields().id != handoff_id || handoff.status().as_str() != receipt.status() {
            return Err(failure(TargetedHandoffQueueFailureKind::InvalidResponse));
        }
        Ok(handoff)
    }

    async fn signed_request(
        &self,
        method: &str,
        request_target: &str,
        body: &str,
    ) -> Result<reqwest::RequestBuilder, TargetedHandoffQueueFailure> {
        let authorized = self
            .authorizer
            .authorize(method, request_target, body)
            .await
            .map_err(map_session_failure)?;
        let url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| failure(TargetedHandoffQueueFailureKind::InvalidResponse))?;
        let builder = match method {
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            _ => return Err(failure(TargetedHandoffQueueFailureKind::InvalidResponse)),
        }
        .header(header::ACCEPT, "application/json");
        signed_request_headers(builder, &authorized, method, request_target)
            .map_err(|()| failure(TargetedHandoffQueueFailureKind::InvalidResponse))
    }
}

impl TargetedHandoffQueueGateway for ReqwestTargetedHandoffQueueGateway {
    fn claim_next(
        &self,
        target: TargetedHandoffTarget,
    ) -> agent_room_application::ports::PortFuture<
        '_,
        Result<Option<TargetedHandoff>, TargetedHandoffQueueFailure>,
    > {
        Box::pin(self.claim_internal(target))
    }

    fn record_receipt<'a>(
        &'a self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
        receipt: &'a TargetedHandoffReceipt,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        Result<TargetedHandoff, TargetedHandoffQueueFailure>,
    > {
        Box::pin(self.receipt_internal(target, handoff_id, receipt))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimResponse {
    handoff: Option<HandoffResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffResponse {
    handoff_id: String,
    principal_id: String,
    status: String,
    source: HandoffSourceResponse,
    target: HandoffTargetResponse,
    content: HandoffContentResponse,
    permissions: Vec<String>,
    purpose: String,
    created_at_unix_ms: i64,
    queued_at_unix_ms: i64,
    delivered_at_unix_ms: Option<i64>,
    consumed_at_unix_ms: Option<i64>,
    resolved_at_unix_ms: Option<i64>,
    expires_at_unix_ms: i64,
    failure_code: Option<String>,
    version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffSourceResponse {
    #[serde(rename = "matrixRoomId")]
    room: String,
    #[serde(rename = "matrixEventId")]
    event: String,
    #[serde(rename = "messageId")]
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffTargetResponse {
    agent_id: String,
    agent_instance_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffContentResponse {
    content_id: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
}

impl HandoffResponse {
    fn into_domain(self) -> Result<TargetedHandoff, TargetedHandoffQueueFailure> {
        let permissions = self
            .permissions
            .iter()
            .map(|value| HandoffPermission::try_from(value.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .and_then(HandoffPermissions::new)
            .map_err(|_| invalid_response())?;
        let fields = TargetedHandoffFields {
            id: parse_id(&self.handoff_id, HandoffId::from_uuid)?,
            principal_id: parse_id(&self.principal_id, PrincipalId::from_uuid)?,
            source_room_id: MatrixRoomReference::new(self.source.room)
                .map_err(|_| invalid_response())?,
            source_event_id: HandoffSourceEventId::new(self.source.event)
                .map_err(|_| invalid_response())?,
            source_message_id: parse_id(&self.source.message, MessageId::from_uuid)?,
            target_agent_id: parse_id(&self.target.agent_id, AgentId::from_uuid)?,
            target_instance_id: parse_id(
                &self.target.agent_instance_id,
                AgentInstanceId::from_uuid,
            )?,
            content: HandoffContentReference::new(
                parse_id(&self.content.content_id, ContentId::from_uuid)?,
                decode_digest(&self.content.sha256).ok_or_else(invalid_response)?,
                ContentByteLength::new(self.content.byte_length).map_err(|_| invalid_response())?,
                ContentMediaType::new(self.content.media_type).map_err(|_| invalid_response())?,
            ),
            permissions,
            purpose: HandoffPurpose::try_from(self.purpose.as_str())
                .map_err(|_| invalid_response())?,
            created_at: parse_time(self.created_at_unix_ms)?,
            expires_at: parse_time(self.expires_at_unix_ms)?,
        };
        TargetedHandoff::restore(
            fields,
            parse_status(&self.status)?,
            parse_time(self.queued_at_unix_ms)?,
            self.delivered_at_unix_ms.map(parse_time).transpose()?,
            self.consumed_at_unix_ms.map(parse_time).transpose()?,
            self.resolved_at_unix_ms.map(parse_time).transpose()?,
            self.failure_code
                .map(HandoffFailureCode::new)
                .transpose()
                .map_err(|_| invalid_response())?,
            self.version,
        )
        .map_err(|_| invalid_response())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptBody<'a> {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<&'a str>,
}

impl<'a> From<&'a TargetedHandoffReceipt> for ReceiptBody<'a> {
    fn from(value: &'a TargetedHandoffReceipt) -> Self {
        Self {
            status: value.status(),
            failure_code: value.failure_code().map(HandoffFailureCode::as_str),
        }
    }
}

async fn decode_json<T: for<'de> Deserialize<'de>>(
    mut response: reqwest::Response,
) -> Result<T, TargetedHandoffQueueFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(invalid_response());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| failure(TargetedHandoffQueueFailureKind::Unavailable))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(invalid_response());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| invalid_response())
}

fn ensure_target(
    handoff: &TargetedHandoff,
    expected: TargetedHandoffTarget,
) -> Result<(), TargetedHandoffQueueFailure> {
    let fields = handoff.fields();
    if fields.target_agent_id == expected.agent_id
        && fields.target_instance_id == expected.instance_id
    {
        Ok(())
    } else {
        Err(invalid_response())
    }
}

fn parse_id<T>(
    value: &str,
    constructor: impl FnOnce(Uuid) -> T,
) -> Result<T, TargetedHandoffQueueFailure> {
    parse_uuid_v7(value)
        .map(constructor)
        .ok_or_else(invalid_response)
}

fn parse_uuid_v7(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version() == Some(Version::SortRand))
}

fn parse_time(value: i64) -> Result<UtcMillis, TargetedHandoffQueueFailure> {
    UtcMillis::new(value).map_err(|_| invalid_response())
}

fn parse_status(value: &str) -> Result<TargetedHandoffStatus, TargetedHandoffQueueFailure> {
    match value {
        "queued" => Ok(TargetedHandoffStatus::Queued),
        "delivered" => Ok(TargetedHandoffStatus::Delivered),
        "consumed" => Ok(TargetedHandoffStatus::Consumed),
        "declined" => Ok(TargetedHandoffStatus::Declined),
        "revoked" => Ok(TargetedHandoffStatus::Revoked),
        "expired" => Ok(TargetedHandoffStatus::Expired),
        "failed" => Ok(TargetedHandoffStatus::Failed),
        _ => Err(invalid_response()),
    }
}

fn decode_digest(value: &str) -> Option<Sha256Digest> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (decode_hex_nibble(pair[0])? << 4) | decode_hex_nibble(pair[1])?;
    }
    Some(Sha256Digest::from_bytes(output))
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> TargetedHandoffQueueFailure {
    failure(match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            TargetedHandoffQueueFailureKind::Denied
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            TargetedHandoffQueueFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => TargetedHandoffQueueFailureKind::InvalidResponse,
    })
}

fn status_failure(status: StatusCode) -> TargetedHandoffQueueFailure {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => TargetedHandoffQueueFailureKind::Denied,
        StatusCode::NOT_FOUND => TargetedHandoffQueueFailureKind::NotFound,
        StatusCode::CONFLICT => TargetedHandoffQueueFailureKind::Conflict,
        StatusCode::GONE => TargetedHandoffQueueFailureKind::Expired,
        StatusCode::TOO_MANY_REQUESTS => TargetedHandoffQueueFailureKind::RateLimited,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => TargetedHandoffQueueFailureKind::Unavailable,
        _ if status.is_server_error() => TargetedHandoffQueueFailureKind::Unavailable,
        _ => TargetedHandoffQueueFailureKind::InvalidResponse,
    };
    failure(kind)
}

const fn invalid_response() -> TargetedHandoffQueueFailure {
    failure(TargetedHandoffQueueFailureKind::InvalidResponse)
}

const fn failure(kind: TargetedHandoffQueueFailureKind) -> TargetedHandoffQueueFailure {
    TargetedHandoffQueueFailure::new(kind)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use agent_room_application::{
        devices::{DeviceRequestProof, DeviceRequestProofPayload},
        ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
    };
    use agent_room_bridge_core::{
        handoffs::{
            TargetedHandoffQueueFailureKind, TargetedHandoffQueueGateway, TargetedHandoffReceipt,
            TargetedHandoffTarget,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{
        handoff::{HandoffFailureCode, TargetedHandoffStatus},
        ids::{AgentId, AgentInstanceId, DeviceId, HandoffId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{post, put},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{ControlPlaneHttpConfig, ReqwestTargetedHandoffQueueGateway, parse_uuid_v7};

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
            self.requests.lock().expect("授权记录锁可用").push((
                method.to_owned(),
                request_target.to_owned(),
                body.to_owned(),
            ));
            let payload = DeviceRequestProofPayload::new(
                DeviceId::from_uuid(Uuid::now_v7()),
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
    async fn 领取与消费回执都使用签名端点并恢复完整领域对象() {
        let fixture = Arc::new(Fixture::new());
        let app = Router::new()
            .route(
                "/agent-instances/{instance_id}/handoffs/claim",
                post(claim_handler),
            )
            .route(
                "/agent-instances/{instance_id}/handoffs/{handoff_id}/receipt",
                put(receipt_handler),
            )
            .with_state(fixture.clone());
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = gateway(spawn_server(app).await, authorizer.clone());

        let claimed = gateway
            .claim_next(fixture.target())
            .await
            .expect("领取响应有效")
            .expect("存在待处理交接");
        assert_eq!(claimed.status(), TargetedHandoffStatus::Delivered);
        assert_eq!(claimed.fields().id, fixture.handoff_id);

        let consumed = gateway
            .record_receipt(
                fixture.target(),
                fixture.handoff_id,
                &TargetedHandoffReceipt::Consumed,
            )
            .await
            .expect("消费回执有效");
        assert_eq!(consumed.status(), TargetedHandoffStatus::Consumed);

        let calls = authorizer.requests.lock().expect("授权记录锁可用");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "POST");
        assert!(calls[0].1.ends_with("/handoffs/claim"));
        assert!(calls[0].2.is_empty());
        assert_eq!(calls[1].0, "PUT");
        assert!(calls[1].1.ends_with("/receipt"));
        assert_eq!(
            serde_json::from_str::<Value>(&calls[1].2).expect("回执正文有效"),
            json!({ "status": "consumed" })
        );
    }

    #[tokio::test]
    async fn 目标漂移和矛盾时间线均失败关闭() {
        let fixture = Arc::new(Fixture::new());
        let drift_fixture = fixture.clone();
        let drift_app = Router::new().route(
            "/agent-instances/{instance_id}/handoffs/claim",
            post(move || {
                let fixture = drift_fixture.clone();
                async move {
                    Json(json!({
                        "handoff": fixture.handoff_json(
                            "delivered",
                            Some(Uuid::now_v7().to_string()),
                        )
                    }))
                }
            }),
        );
        let drift_gateway = gateway(
            spawn_server(drift_app).await,
            Arc::new(测试请求授权器::default()),
        );
        let drift = drift_gateway
            .claim_next(fixture.target())
            .await
            .expect_err("目标漂移必须拒绝");
        assert_eq!(
            drift.kind(),
            TargetedHandoffQueueFailureKind::InvalidResponse
        );

        let timeline_fixture = fixture.clone();
        let timeline_app = Router::new().route(
            "/agent-instances/{instance_id}/handoffs/claim",
            post(move || {
                let fixture = timeline_fixture.clone();
                async move {
                    let mut handoff = fixture.handoff_json("delivered", None);
                    handoff["deliveredAtUnixMs"] = json!(999);
                    Json(json!({ "handoff": handoff }))
                }
            }),
        );
        let timeline_gateway = gateway(
            spawn_server(timeline_app).await,
            Arc::new(测试请求授权器::default()),
        );
        let timeline = timeline_gateway
            .claim_next(fixture.target())
            .await
            .expect_err("倒退时间线必须拒绝");
        assert_eq!(
            timeline.kind(),
            TargetedHandoffQueueFailureKind::InvalidResponse
        );
    }

    #[tokio::test]
    async fn 回执状态不一致与未知字段不能伪装成功() {
        let fixture = Arc::new(Fixture::new());
        let mismatch_fixture = fixture.clone();
        let mismatch_app = Router::new().route(
            "/agent-instances/{instance_id}/handoffs/{handoff_id}/receipt",
            put(move || {
                let fixture = mismatch_fixture.clone();
                async move { Json(fixture.handoff_json("declined", None)) }
            }),
        );
        let mismatch_gateway = gateway(
            spawn_server(mismatch_app).await,
            Arc::new(测试请求授权器::default()),
        );
        let mismatch = mismatch_gateway
            .record_receipt(
                fixture.target(),
                fixture.handoff_id,
                &TargetedHandoffReceipt::Failed(
                    HandoffFailureCode::new("bridge.persist_failed").expect("失败码有效"),
                ),
            )
            .await
            .expect_err("回执状态不一致必须拒绝");
        assert_eq!(
            mismatch.kind(),
            TargetedHandoffQueueFailureKind::InvalidResponse
        );

        let unknown_fixture = fixture.clone();
        let unknown_app = Router::new().route(
            "/agent-instances/{instance_id}/handoffs/claim",
            post(move || {
                let fixture = unknown_fixture.clone();
                async move {
                    let mut handoff = fixture.handoff_json("delivered", None);
                    handoff["unexpected"] = json!(true);
                    Json(json!({ "handoff": handoff }))
                }
            }),
        );
        let unknown_gateway = gateway(
            spawn_server(unknown_app).await,
            Arc::new(测试请求授权器::default()),
        );
        let unknown = unknown_gateway
            .claim_next(fixture.target())
            .await
            .expect_err("未知字段必须拒绝");
        assert_eq!(
            unknown.kind(),
            TargetedHandoffQueueFailureKind::InvalidResponse
        );
    }

    async fn claim_handler(
        State(fixture): State<Arc<Fixture>>,
        Path(instance_id): Path<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        if instance_id != fixture.target_instance.to_string()
            || !body.is_empty()
            || !has_signed_headers(&headers)
        {
            return StatusCode::BAD_REQUEST.into_response();
        }
        Json(json!({ "handoff": fixture.handoff_json("delivered", None) })).into_response()
    }

    async fn receipt_handler(
        State(fixture): State<Arc<Fixture>>,
        Path((instance_id, handoff_id)): Path<(String, String)>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        let decoded = serde_json::from_slice::<Value>(&body).ok();
        if instance_id != fixture.target_instance.to_string()
            || handoff_id != fixture.handoff_id.to_string()
            || !has_signed_headers(&headers)
            || decoded
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                != Some("consumed")
        {
            return StatusCode::BAD_REQUEST.into_response();
        }
        Json(fixture.handoff_json("consumed", None)).into_response()
    }

    #[derive(Debug)]
    struct Fixture {
        handoff_id: HandoffId,
        principal_id: Uuid,
        source_message_id: Uuid,
        content_id: Uuid,
        target_agent: AgentId,
        target_instance: AgentInstanceId,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                handoff_id: HandoffId::from_uuid(Uuid::now_v7()),
                principal_id: Uuid::now_v7(),
                source_message_id: Uuid::now_v7(),
                content_id: Uuid::now_v7(),
                target_agent: AgentId::from_uuid(Uuid::now_v7()),
                target_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
            }
        }

        const fn target(&self) -> TargetedHandoffTarget {
            TargetedHandoffTarget {
                agent_id: self.target_agent,
                instance_id: self.target_instance,
            }
        }

        fn handoff_json(&self, status: &str, target_instance: Option<String>) -> Value {
            let (consumed_at, resolved_at, failure_code, version) = match status {
                "consumed" => (json!(1_200), json!(1_200), Value::Null, 2),
                "declined" => (Value::Null, json!(1_200), json!("agent.declined"), 2),
                "failed" => (Value::Null, json!(1_200), json!("bridge.persist_failed"), 2),
                _ => (Value::Null, Value::Null, Value::Null, 1),
            };
            json!({
                "handoffId": self.handoff_id.to_string(),
                "principalId": self.principal_id.to_string(),
                "status": status,
                "source": {
                    "matrixRoomId": "!lobby:matrix.test",
                    "matrixEventId": "$event-123",
                    "messageId": self.source_message_id.to_string()
                },
                "target": {
                    "agentId": self.target_agent.to_string(),
                    "agentInstanceId": target_instance
                        .unwrap_or_else(|| self.target_instance.to_string())
                },
                "content": {
                    "contentId": self.content_id.to_string(),
                    "sha256": "0707070707070707070707070707070707070707070707070707070707070707",
                    "byteLength": 16,
                    "mediaType": "text/plain"
                },
                "permissions": ["read_text", "include_metadata"],
                "purpose": "inspect",
                "createdAtUnixMs": 1_000,
                "queuedAtUnixMs": 1_000,
                "deliveredAtUnixMs": 1_100,
                "consumedAtUnixMs": consumed_at,
                "resolvedAtUnixMs": resolved_at,
                "expiresAtUnixMs": 5_000,
                "failureCode": failure_code,
                "version": version
            })
        }
    }

    fn gateway(
        base_url: String,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> ReqwestTargetedHandoffQueueGateway {
        ReqwestTargetedHandoffQueueGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("测试网关配置有效")
    }

    async fn spawn_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试监听器可创建");
        let address = listener.local_addr().expect("测试地址有效");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务正常结束");
        });
        format!("http://{address}/")
    }

    fn has_signed_headers(headers: &HeaderMap) -> bool {
        headers.contains_key("authorization")
            && headers.contains_key("x-agent-room-device-id")
            && headers.contains_key("x-agent-room-proof-issued-at")
            && headers.contains_key("x-agent-room-proof-nonce")
            && headers.contains_key("x-agent-room-proof-signature")
    }

    fn secret(value: &str) -> SecretValue {
        SecretValue::new(value.to_owned()).expect("测试秘密有效")
    }

    #[test]
    fn 测试固定标识均保持_uuid_v7() {
        let fixture = Fixture::new();
        assert!(parse_uuid_v7(&fixture.handoff_id.to_string()).is_some());
        assert!(parse_uuid_v7(&fixture.target_instance.to_string()).is_some());
    }
}
