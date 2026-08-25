use std::sync::Arc;

use agent_room_bridge_core::{
    messages::{
        AutomationAuthorizationDenial, AutomationAuthorizationFailure,
        AutomationAuthorizationGateway, AutomationAuthorizationRequest,
        AutomationAuthorizationResult,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::policy::AutomationRiskScanOutcome;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

pub struct ReqwestControlPlaneAutomationAuthorizationGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneAutomationAuthorizationGateway {
    /// 创建绑定固定控制面与设备证明的自动发言授权网关。
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

    async fn authorize_internal(
        &self,
        request: &AutomationAuthorizationRequest,
    ) -> AutomationAuthorizationResult<()> {
        let request_target = format!("/automation-grants/{}/authorizations", request.grant_id);
        let body = serde_json::to_string(&AuthorizationBody::from(request))
            .map_err(|_| AutomationAuthorizationFailure::internal())?;
        let authorized = self
            .authorizer
            .authorize("POST", &request_target, &body)
            .await
            .map_err(map_session_failure)?;
        let request_url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| AutomationAuthorizationFailure::internal())?;
        let request = signed_request_headers(
            self.client
                .post(request_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            &request_target,
        )
        .map_err(|()| AutomationAuthorizationFailure::internal())?;
        let response = request
            .body(body)
            .send()
            .await
            .map_err(|_| AutomationAuthorizationFailure::unavailable())?;
        decode_response(response).await
    }
}

impl AutomationAuthorizationGateway for ReqwestControlPlaneAutomationAuthorizationGateway {
    fn authorize<'a>(
        &'a self,
        request: &'a AutomationAuthorizationRequest,
    ) -> agent_room_application::ports::PortFuture<'a, AutomationAuthorizationResult<()>> {
        Box::pin(self.authorize_internal(request))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationBody<'a> {
    submission_id: String,
    agent_id: String,
    agent_instance_id: String,
    room_catalog_id: String,
    matrix_room_id: &'a str,
    message_kind: &'static str,
    risk_scan: &'static str,
}

impl<'a> From<&'a AutomationAuthorizationRequest> for AuthorizationBody<'a> {
    fn from(request: &'a AutomationAuthorizationRequest) -> Self {
        Self {
            submission_id: request.submission_id.to_string(),
            agent_id: request.agent_id.to_string(),
            agent_instance_id: request.agent_instance_id.to_string(),
            room_catalog_id: request.room_catalog_id.to_string(),
            matrix_room_id: request.matrix_room_id.as_str(),
            message_kind: if request.is_reply {
                "reply"
            } else {
                "room_message"
            },
            risk_scan: risk_scan(request.risk_scan),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthorizationResponse {
    decision: String,
    reason: Option<String>,
    reused: bool,
}

async fn decode_response(response: reqwest::Response) -> AutomationAuthorizationResult<()> {
    let status = response.status();
    let body = read_limited_response_body(response)
        .await
        .map_err(|()| AutomationAuthorizationFailure::invalid_response())?;
    if !status.is_success() {
        return Err(status_failure(status));
    }
    let response = serde_json::from_slice::<AuthorizationResponse>(&body)
        .map_err(|_| AutomationAuthorizationFailure::invalid_response())?;
    match (
        response.decision.as_str(),
        response.reason.as_deref(),
        response.reused,
    ) {
        ("authorized", None, _) => Ok(()),
        ("denied", Some(reason), false) => {
            let reason = AutomationAuthorizationDenial::try_from(reason)
                .map_err(|()| AutomationAuthorizationFailure::invalid_response())?;
            Err(AutomationAuthorizationFailure::denied(reason))
        }
        _ => Err(AutomationAuthorizationFailure::invalid_response()),
    }
}

const fn risk_scan(value: AutomationRiskScanOutcome) -> &'static str {
    match value {
        AutomationRiskScanOutcome::Passed => "passed",
        AutomationRiskScanOutcome::Rejected => "rejected",
        AutomationRiskScanOutcome::Unavailable => "unavailable",
        AutomationRiskScanOutcome::NotRequested => "not_requested",
    }
}

fn status_failure(status: StatusCode) -> AutomationAuthorizationFailure {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AutomationAuthorizationFailure::denied(
            AutomationAuthorizationDenial::ControlPlaneRejected,
        ),
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => AutomationAuthorizationFailure::unavailable(),
        _ if status.is_server_error() => AutomationAuthorizationFailure::unavailable(),
        _ => AutomationAuthorizationFailure::invalid_response(),
    }
}

fn map_session_failure(failure: BridgeSessionFailure) -> AutomationAuthorizationFailure {
    match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            AutomationAuthorizationFailure::denied(
                AutomationAuthorizationDenial::ControlPlaneRejected,
            )
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            AutomationAuthorizationFailure::unavailable()
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => AutomationAuthorizationFailure::internal(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_room_application::{
        devices::{DeviceRequestProof, DeviceRequestProofPayload},
        ports::{DeviceSignature, SecretFactory, SecretValue},
    };
    use agent_room_bridge_core::{
        messages::{
            AutomationAuthorizationDenial, AutomationAuthorizationFailureKind,
            AutomationAuthorizationGateway, AutomationAuthorizationRequest,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{
        ids::{
            AgentId, AgentInstanceId, AutomationGrantId, DeviceId, MessageSubmissionId,
            RoomCatalogId,
        },
        policy::AutomationRiskScanOutcome,
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{ControlPlaneHttpConfig, ReqwestControlPlaneAutomationAuthorizationGateway};

    const GRANT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const SUBMISSION_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
    const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const DEVICE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";

    struct RecordingAuthorizer {
        body: Mutex<Option<String>>,
    }

    impl RecordingAuthorizer {
        fn new() -> Self {
            Self {
                body: Mutex::new(None),
            }
        }
    }

    impl ControlPlaneRequestAuthorizer for RecordingAuthorizer {
        fn authorize<'a>(
            &'a self,
            method: &'a str,
            request_target: &'a str,
            body: &'a str,
        ) -> agent_room_application::ports::PortFuture<
            'a,
            BridgeSessionResult<AuthorizedControlPlaneRequest>,
        > {
            assert_eq!(method, "POST");
            assert_eq!(request_target, target());
            *self.body.lock().expect("授权正文记录锁可用") = Some(body.to_owned());
            let proof = proof(method, request_target, body);
            Box::pin(async move {
                Ok(AuthorizedControlPlaneRequest {
                    access_token: SecretValue::new("device-access-token")
                        .expect("测试访问令牌有效"),
                    proof,
                })
            })
        }
    }

    #[derive(Clone)]
    struct ServerState {
        status: StatusCode,
        response: Value,
    }

    #[tokio::test]
    async fn 授权请求签名覆盖精确正文且许可响应放行() {
        let authorizer = Arc::new(RecordingAuthorizer::new());
        let gateway = gateway(
            spawn_server(ServerState {
                status: StatusCode::OK,
                response: json!({
                    "decision": "authorized",
                    "reused": false
                }),
            })
            .await,
            authorizer.clone(),
        );

        gateway
            .authorize(&authorization_request())
            .await
            .expect("授权放行");

        let body = authorizer
            .body
            .lock()
            .expect("授权正文记录锁可用")
            .clone()
            .expect("已记录授权正文");
        assert_eq!(
            serde_json::from_str::<Value>(&body).expect("授权正文是 JSON"),
            json!({
                "submissionId": SUBMISSION_ID,
                "agentId": AGENT_ID,
                "agentInstanceId": INSTANCE_ID,
                "roomCatalogId": CATALOG_ID,
                "matrixRoomId": "!lobby:matrix.agent-room.test",
                "messageKind": "reply",
                "riskScan": "not_requested"
            })
        );
    }

    #[tokio::test]
    async fn 控制面业务拒绝保留类型化原因() {
        let gateway = gateway(
            spawn_server(ServerState {
                status: StatusCode::OK,
                response: json!({
                    "decision": "denied",
                    "reason": "automation.rate_limit_exceeded",
                    "reused": false
                }),
            })
            .await,
            Arc::new(RecordingAuthorizer::new()),
        );

        let failure = gateway
            .authorize(&authorization_request())
            .await
            .expect_err("频率耗尽必须拒绝");

        assert_eq!(failure.kind(), AutomationAuthorizationFailureKind::Denied);
        assert_eq!(
            failure.denial(),
            Some(AutomationAuthorizationDenial::RateLimitExceeded)
        );
    }

    #[tokio::test]
    async fn 服务不可用与未知拒绝原因都失败关闭() {
        let unavailable = gateway(
            spawn_server(ServerState {
                status: StatusCode::SERVICE_UNAVAILABLE,
                response: json!({ "code": "automation.dependency_unavailable" }),
            })
            .await,
            Arc::new(RecordingAuthorizer::new()),
        );
        let malformed = gateway(
            spawn_server(ServerState {
                status: StatusCode::OK,
                response: json!({
                    "decision": "denied",
                    "reason": "automation.future_unknown_reason",
                    "reused": false
                }),
            })
            .await,
            Arc::new(RecordingAuthorizer::new()),
        );

        assert_eq!(
            unavailable
                .authorize(&authorization_request())
                .await
                .expect_err("依赖不可用必须失败")
                .kind(),
            AutomationAuthorizationFailureKind::Unavailable
        );
        assert_eq!(
            malformed
                .authorize(&authorization_request())
                .await
                .expect_err("未知拒绝原因不得被误判为许可")
                .kind(),
            AutomationAuthorizationFailureKind::InvalidResponse
        );
    }

    async fn authorize_handler(
        State(state): State<ServerState>,
        Path(grant_id): Path<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        assert_eq!(grant_id, GRANT_ID);
        assert_eq!(
            header(&headers, "authorization"),
            Some("Bearer device-access-token")
        );
        assert_eq!(header(&headers, "x-agent-room-device-id"), Some(DEVICE_ID));
        assert_eq!(
            header(&headers, "x-agent-room-proof-issued-at"),
            Some("1700000000000")
        );
        assert_eq!(
            header(&headers, "x-agent-room-proof-nonce"),
            Some("nonce-0123456789abcdef")
        );
        assert_eq!(
            header(&headers, "x-agent-room-proof-signature"),
            Some(URL_SAFE_NO_PAD.encode([9_u8; 64]).as_str())
        );
        assert!(!body.is_empty());
        (state.status, Json(state.response)).into_response()
    }

    async fn spawn_server(state: ServerState) -> String {
        let app = Router::new()
            .route(
                "/automation-grants/{grant_id}/authorizations",
                post(authorize_handler),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试监听器可绑定");
        let address = listener.local_addr().expect("测试地址存在");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试服务器正常运行");
        });
        format!("http://{address}/")
    }

    fn gateway(
        base_url: String,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> ReqwestControlPlaneAutomationAuthorizationGateway {
        ReqwestControlPlaneAutomationAuthorizationGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: std::time::Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("测试自动授权网关配置有效")
    }

    fn authorization_request() -> AutomationAuthorizationRequest {
        AutomationAuthorizationRequest {
            grant_id: AutomationGrantId::from_uuid(uuid(GRANT_ID)),
            submission_id: MessageSubmissionId::from_uuid(uuid(SUBMISSION_ID)),
            agent_id: AgentId::from_uuid(uuid(AGENT_ID)),
            agent_instance_id: AgentInstanceId::from_uuid(uuid(INSTANCE_ID)),
            room_catalog_id: RoomCatalogId::from_uuid(uuid(CATALOG_ID)),
            matrix_room_id: agent_room_application::ports::MatrixRoomId::new(
                "!lobby:matrix.agent-room.test",
            )
            .expect("测试 Matrix 房间有效"),
            is_reply: true,
            risk_scan: AutomationRiskScanOutcome::NotRequested,
        }
    }

    fn target() -> String {
        format!("/automation-grants/{GRANT_ID}/authorizations")
    }

    fn proof(method: &str, request_target: &str, body: &str) -> DeviceRequestProof {
        let payload = DeviceRequestProofPayload::new(
            DeviceId::from_uuid(uuid(DEVICE_ID)),
            UtcMillis::new(1_700_000_000_000).expect("测试签发时间有效"),
            SecretValue::new("nonce-0123456789abcdef").expect("测试随机数有效"),
            method.to_owned(),
            request_target.to_owned(),
            SecureSecretFactory.digest(body),
        )
        .expect("测试证明载荷有效");
        DeviceRequestProof::new(
            payload,
            DeviceSignature::new(vec![9_u8; 64]).expect("测试签名有效"),
        )
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }
}
