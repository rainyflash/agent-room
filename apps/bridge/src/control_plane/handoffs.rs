use std::sync::Arc;

use agent_room_bridge_core::{
    handoffs::{
        HandoffAuthorizationDecision, HandoffAuthorizationFailure, HandoffAuthorizationFailureKind,
        HandoffAuthorizationGateway, HandoffAuthorizationRequest, HandoffDeviceAddress,
        HandoffDirectoryFailure, HandoffDirectoryFailureKind, HandoffInstanceDirectory,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::ids::{AgentId, AgentInstanceId};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::{Uuid, Version};

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    signed_request_headers,
};

const AUTHORIZATION_TARGET: &str = "/handoffs/authorization";
const MAX_RESPONSE_BYTES: usize = 8 * 1_024;

pub struct ReqwestControlPlaneHandoffGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneHandoffGateway {
    /// 创建只访问固定控制面交接端点的签名 HTTP 网关。
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
        request: &HandoffAuthorizationRequest,
    ) -> Result<HandoffAuthorizationDecision, HandoffAuthorizationFailure> {
        let body = serde_json::to_string(&AuthorizationBody::from(request))
            .map_err(|_| authorization_failure(HandoffAuthorizationFailureKind::InvalidResponse))?;
        let authorized = self
            .authorizer
            .authorize("POST", AUTHORIZATION_TARGET, &body)
            .await
            .map_err(map_authorization_session_failure)?;
        let response = signed_request_headers(
            self.client
                .post(self.url(AUTHORIZATION_TARGET).map_err(|()| {
                    authorization_failure(HandoffAuthorizationFailureKind::InvalidResponse)
                })?)
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            AUTHORIZATION_TARGET,
        )
        .map_err(|()| authorization_failure(HandoffAuthorizationFailureKind::InvalidResponse))?
        .body(body)
        .send()
        .await
        .map_err(|_| authorization_failure(HandoffAuthorizationFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(authorization_status_failure(response.status()));
        }
        let decoded = decode_json::<AuthorizationResponse>(response)
            .await
            .map_err(map_authorization_decode_failure)?;
        match decoded.decision.as_str() {
            "allowed" => Ok(HandoffAuthorizationDecision::Allowed),
            "denied" => Ok(HandoffAuthorizationDecision::Denied),
            _ => Err(authorization_failure(
                HandoffAuthorizationFailureKind::InvalidResponse,
            )),
        }
    }

    async fn resolve_internal(
        &self,
        instance_id: AgentInstanceId,
    ) -> Result<HandoffDeviceAddress, HandoffDirectoryFailure> {
        let target = format!("/agent-instances/{instance_id}/handoff-address");
        let authorized = self
            .authorizer
            .authorize("GET", &target, "")
            .await
            .map_err(map_directory_session_failure)?;
        let response =
            signed_request_headers(
                self.client.get(self.url(&target).map_err(|()| {
                    directory_failure(HandoffDirectoryFailureKind::InvalidResponse)
                })?),
                &authorized,
                "GET",
                &target,
            )
            .map_err(|()| directory_failure(HandoffDirectoryFailureKind::InvalidResponse))?
            .body(Vec::new())
            .send()
            .await
            .map_err(|_| directory_failure(HandoffDirectoryFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(directory_status_failure(response.status()));
        }
        let decoded = decode_json::<InstanceAddressResponse>(response)
            .await
            .map_err(map_directory_decode_failure)?;
        decoded.into_address(instance_id)
    }

    fn url(&self, target: &str) -> Result<Url, ()> {
        self.base_url
            .join(target.trim_start_matches('/'))
            .map_err(|_| ())
    }
}

impl HandoffAuthorizationGateway for ReqwestControlPlaneHandoffGateway {
    fn authorize<'a>(
        &'a self,
        request: &'a HandoffAuthorizationRequest,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        Result<HandoffAuthorizationDecision, HandoffAuthorizationFailure>,
    > {
        Box::pin(self.authorize_internal(request))
    }
}

impl HandoffInstanceDirectory for ReqwestControlPlaneHandoffGateway {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> agent_room_application::ports::PortFuture<
        '_,
        Result<HandoffDeviceAddress, HandoffDirectoryFailure>,
    > {
        Box::pin(self.resolve_internal(instance_id))
    }
}

#[derive(Serialize)]
struct AuthorizationBody {
    #[serde(rename = "principalId")]
    principal: String,
    #[serde(rename = "requesterAgentId")]
    requester_agent: String,
    #[serde(rename = "requesterInstanceId")]
    requester_instance: String,
    #[serde(rename = "targetAgentId")]
    target_agent: String,
    #[serde(rename = "targetInstanceId")]
    target_instance: String,
}

impl From<&HandoffAuthorizationRequest> for AuthorizationBody {
    fn from(value: &HandoffAuthorizationRequest) -> Self {
        Self {
            principal: value.principal_id.to_string(),
            requester_agent: value.requester_agent_id.to_string(),
            requester_instance: value.requester_instance_id.to_string(),
            target_agent: value.target_agent_id.to_string(),
            target_instance: value.target_instance_id.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationResponse {
    decision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceAddressResponse {
    #[serde(rename = "agentId")]
    agent: String,
    #[serde(rename = "agentInstanceId")]
    instance: String,
    #[serde(rename = "matrixUserId")]
    matrix_user: String,
    #[serde(rename = "matrixDeviceId")]
    matrix_device: String,
}

impl InstanceAddressResponse {
    fn into_address(
        self,
        expected_instance_id: AgentInstanceId,
    ) -> Result<HandoffDeviceAddress, HandoffDirectoryFailure> {
        let agent_id = parse_uuid_v7(&self.agent)
            .map(AgentId::from_uuid)
            .ok_or_else(|| directory_failure(HandoffDirectoryFailureKind::InvalidResponse))?;
        let instance_id = parse_uuid_v7(&self.instance)
            .map(AgentInstanceId::from_uuid)
            .filter(|value| *value == expected_instance_id)
            .ok_or_else(|| directory_failure(HandoffDirectoryFailureKind::InvalidResponse))?;
        let matrix_user = agent_room_application::ports::MatrixUserId::new(self.matrix_user)
            .map_err(|_| directory_failure(HandoffDirectoryFailureKind::InvalidResponse))?;
        let matrix_device = agent_room_application::ports::MatrixDeviceId::new(self.matrix_device)
            .map_err(|_| directory_failure(HandoffDirectoryFailureKind::InvalidResponse))?;
        Ok(HandoffDeviceAddress::new(
            agent_id,
            instance_id,
            matrix_user,
            matrix_device,
        ))
    }
}

#[derive(Debug, Clone, Copy)]
enum ResponseDecodeFailure {
    Unavailable,
    InvalidResponse,
}

async fn decode_json<T: for<'de> Deserialize<'de>>(
    mut response: reqwest::Response,
) -> Result<T, ResponseDecodeFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ResponseDecodeFailure::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ResponseDecodeFailure::Unavailable)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ResponseDecodeFailure::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ResponseDecodeFailure::InvalidResponse)
}

fn parse_uuid_v7(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version() == Some(Version::SortRand))
}

fn map_authorization_session_failure(failure: BridgeSessionFailure) -> HandoffAuthorizationFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            HandoffAuthorizationFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => HandoffAuthorizationFailureKind::InvalidResponse,
    };
    authorization_failure(kind)
}

fn map_directory_session_failure(failure: BridgeSessionFailure) -> HandoffDirectoryFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            HandoffDirectoryFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => HandoffDirectoryFailureKind::InvalidResponse,
    };
    directory_failure(kind)
}

const fn map_authorization_decode_failure(
    failure: ResponseDecodeFailure,
) -> HandoffAuthorizationFailure {
    authorization_failure(match failure {
        ResponseDecodeFailure::Unavailable => HandoffAuthorizationFailureKind::Unavailable,
        ResponseDecodeFailure::InvalidResponse => HandoffAuthorizationFailureKind::InvalidResponse,
    })
}

const fn map_directory_decode_failure(failure: ResponseDecodeFailure) -> HandoffDirectoryFailure {
    directory_failure(match failure {
        ResponseDecodeFailure::Unavailable => HandoffDirectoryFailureKind::Unavailable,
        ResponseDecodeFailure::InvalidResponse => HandoffDirectoryFailureKind::InvalidResponse,
    })
}

fn authorization_status_failure(status: StatusCode) -> HandoffAuthorizationFailure {
    let kind = if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    {
        HandoffAuthorizationFailureKind::Unavailable
    } else {
        HandoffAuthorizationFailureKind::InvalidResponse
    };
    authorization_failure(kind)
}

fn directory_status_failure(status: StatusCode) -> HandoffDirectoryFailure {
    let kind = match status {
        StatusCode::NOT_FOUND => HandoffDirectoryFailureKind::NotFound,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::UNAUTHORIZED
        | StatusCode::FORBIDDEN
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => HandoffDirectoryFailureKind::Unavailable,
        _ if status.is_server_error() => HandoffDirectoryFailureKind::Unavailable,
        _ => HandoffDirectoryFailureKind::InvalidResponse,
    };
    directory_failure(kind)
}

const fn authorization_failure(
    kind: HandoffAuthorizationFailureKind,
) -> HandoffAuthorizationFailure {
    HandoffAuthorizationFailure::new(kind)
}

const fn directory_failure(kind: HandoffDirectoryFailureKind) -> HandoffDirectoryFailure {
    HandoffDirectoryFailure::new(kind)
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
            HandoffAuthorizationFailureKind, HandoffAuthorizationGateway,
            HandoffAuthorizationRequest, HandoffDirectoryFailureKind, HandoffInstanceDirectory,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{
        ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{ControlPlaneHttpConfig, ReqwestControlPlaneHandoffGateway, authorization_failure};

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
    async fn 交接网关按原始正文签名授权并解析精确_matrix_设备() {
        let fixture = Fixture::new();
        let response_fixture = fixture.clone();
        let app = Router::new()
            .route(
                "/handoffs/authorization",
                post(|headers: HeaderMap, body: String| async move {
                    let decoded: Value = serde_json::from_str(&body).expect("授权正文是 JSON");
                    if has_signed_headers(&headers)
                        && decoded.get("principalId").and_then(Value::as_str).is_some()
                    {
                        Json(json!({ "decision": "allowed" })).into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                }),
            )
            .route(
                "/agent-instances/{instance_id}/handoff-address",
                get(move |Path(instance_id): Path<String>, headers: HeaderMap| {
                    let fixture = response_fixture.clone();
                    async move {
                        if !has_signed_headers(&headers)
                            || instance_id != fixture.target_instance.to_string()
                        {
                            return StatusCode::BAD_REQUEST.into_response();
                        }
                        Json(json!({
                            "agentId": fixture.target_agent.to_string(),
                            "agentInstanceId": fixture.target_instance.to_string(),
                            "matrixUserId": "@target:matrix.test",
                            "matrixDeviceId": "TARGET_DEVICE"
                        }))
                        .into_response()
                    }
                }),
            );
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = gateway(spawn_server(app).await, authorizer.clone());

        let decision = gateway
            .authorize(&fixture.authorization_request())
            .await
            .expect("授权响应有效");
        assert_eq!(
            decision,
            agent_room_bridge_core::handoffs::HandoffAuthorizationDecision::Allowed
        );
        let address = gateway
            .resolve(fixture.target_instance)
            .await
            .expect("目标地址有效");
        assert_eq!(address.agent_id(), fixture.target_agent);
        assert_eq!(address.instance_id(), fixture.target_instance);
        assert_eq!(address.matrix_user_id().as_str(), "@target:matrix.test");
        assert_eq!(address.matrix_device_id().as_str(), "TARGET_DEVICE");

        let calls = authorizer.requests.lock().expect("授权记录锁可用");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "POST");
        assert_eq!(calls[0].1, "/handoffs/authorization");
        assert_eq!(calls[1].0, "GET");
        assert_eq!(
            calls[1].1,
            format!(
                "/agent-instances/{}/handoff-address",
                fixture.target_instance
            )
        );
    }

    #[tokio::test]
    async fn 目录明确区分不存在与畸形响应且授权拒绝保持业务结果() {
        let app = Router::new()
            .route(
                "/handoffs/authorization",
                post(|| async { Json(json!({ "decision": "denied" })) }),
            )
            .route(
                "/agent-instances/{instance_id}/handoff-address",
                get(|Path(instance_id): Path<String>| async move {
                    if instance_id.ends_with('0') {
                        Json(json!({ "unexpected": true })).into_response()
                    } else {
                        StatusCode::NOT_FOUND.into_response()
                    }
                }),
            );
        let gateway = gateway(spawn_server(app).await, Arc::new(测试请求授权器::default()));
        let fixture = Fixture::new();
        assert_eq!(
            gateway
                .authorize(&fixture.authorization_request())
                .await
                .expect("拒绝是有效业务响应"),
            agent_room_bridge_core::handoffs::HandoffAuthorizationDecision::Denied
        );
        let missing = gateway
            .resolve(fixture.target_instance)
            .await
            .expect_err("不存在必须明确返回");
        assert_eq!(missing.kind(), HandoffDirectoryFailureKind::NotFound);

        let malformed_id = AgentInstanceId::from_uuid(
            Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e40").expect("测试 UUID 有效"),
        );
        let malformed = gateway
            .resolve(malformed_id)
            .await
            .expect_err("畸形响应必须拒绝");
        assert_eq!(
            malformed.kind(),
            HandoffDirectoryFailureKind::InvalidResponse
        );
    }

    #[test]
    fn 授权失败值保持稳定分类() {
        assert_eq!(
            authorization_failure(HandoffAuthorizationFailureKind::Unavailable).kind(),
            HandoffAuthorizationFailureKind::Unavailable
        );
    }

    #[derive(Clone)]
    struct Fixture {
        principal: PrincipalId,
        requester_agent: AgentId,
        requester_instance: AgentInstanceId,
        target_agent: AgentId,
        target_instance: AgentInstanceId,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                principal: PrincipalId::from_uuid(Uuid::now_v7()),
                requester_agent: AgentId::from_uuid(Uuid::now_v7()),
                requester_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
                target_agent: AgentId::from_uuid(Uuid::now_v7()),
                target_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
            }
        }

        fn authorization_request(&self) -> HandoffAuthorizationRequest {
            HandoffAuthorizationRequest {
                principal_id: self.principal,
                requester_agent_id: self.requester_agent,
                requester_instance_id: self.requester_instance,
                target_agent_id: self.target_agent,
                target_instance_id: self.target_instance,
            }
        }
    }

    fn gateway(
        base_url: String,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> ReqwestControlPlaneHandoffGateway {
        ReqwestControlPlaneHandoffGateway::new(
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
}
