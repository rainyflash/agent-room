use std::{sync::Arc, time::Duration};

use agent_room_application::{
    devices::{AuthenticatedDevice, DeviceCredentials},
    ports::{
        AgentInstanceVerificationRecord, MatrixDeviceId, MatrixSession, MatrixSessionMetadata,
        MatrixUserId, PortFuture, PrincipalAccount, SecretValue,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_runtime::{
        AgentRuntimeRegistrationIntent, ControlPlaneAgentRuntimeFailure,
        ControlPlaneAgentRuntimeFailureKind, ControlPlaneAgentRuntimeGateway,
        ControlPlaneAgentRuntimeResult, RegisteredAgentRuntime,
    },
    agent_verification::{
        AgentInstanceVerificationGateway, AgentInstanceVerificationGatewayFailure,
        AgentInstanceVerificationGatewayFailureKind, AgentInstanceVerificationGatewayResult,
    },
    onboarding::{
        BridgeDefaultAgent, BridgePublicLobby, ControlPlaneOnboardingFailure,
        ControlPlaneOnboardingFailureKind, ControlPlaneOnboardingGateway,
        ControlPlaneOnboardingResult,
    },
    ports::{
        ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway,
        ControlPlaneDeviceResult, RefreshBridgeDevice, RegisterBridgeDevice,
    },
    session::{
        AuthorizedControlPlaneRequest, BridgeSessionFailure, BridgeSessionFailureKind,
        ControlPlaneRequestAuthorizer,
    },
};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    identity::Principal,
    ids::{
        AdapterBindingId, AgentId, AgentInstanceId, ContentId, DeviceId, PrincipalId, RoomCatalogId,
    },
    rooms::RoomLanguage,
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::{Uuid, Version};

mod automation;
mod content;
mod handoffs;
mod lobbies;
mod message_content;

pub use automation::ReqwestControlPlaneAutomationAuthorizationGateway;
pub use content::ReqwestControlPlaneContentGateway;
pub use handoffs::ReqwestControlPlaneHandoffGateway;
pub use lobbies::ReqwestControlPlaneLobbyEntryGateway;
pub use message_content::ReqwestControlPlaneMessageContentGateway;

const REGISTER_DEVICE_PATH: &str = "auth/devices/register";
const REFRESH_DEVICE_PATH: &str = "auth/devices/refresh";
const DEVICE_DEFAULT_AGENT_PATH: &str = "/onboarding/device/default-agent";
const PUBLIC_LOBBIES_PATH: &str = "/lobbies/public";
const DEVICE_ID_HEADER: &str = "x-agent-room-device-id";
const PROOF_ISSUED_AT_HEADER: &str = "x-agent-room-proof-issued-at";
const PROOF_NONCE_HEADER: &str = "x-agent-room-proof-nonce";
const PROOF_SIGNATURE_HEADER: &str = "x-agent-room-proof-signature";
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const MAX_RESPONSE_BYTES: usize = 64 * 1_024;

pub struct ControlPlaneHttpConfig {
    pub base_url: String,
    pub request_timeout: Duration,
}

pub struct ReqwestControlPlaneDeviceGateway {
    client: Client,
    register_url: Url,
    refresh_url: Url,
}

pub struct ReqwestAgentInstanceVerificationGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

pub struct ReqwestControlPlaneAgentRuntimeGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

pub struct ReqwestControlPlaneOnboardingGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneDeviceGateway {
    /// 创建不跟随重定向、限制超时且只向固定控制面发送凭据的客户端。
    ///
    /// # Errors
    ///
    /// URL、明文传输边界或 HTTP 客户端配置无效时返回稳定配置错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        let register_url = base_url
            .join(REGISTER_DEVICE_PATH)
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        let refresh_url = base_url
            .join(REFRESH_DEVICE_PATH)
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        Ok(Self {
            client,
            register_url,
            refresh_url,
        })
    }

    async fn register_internal(
        &self,
        request: RegisterBridgeDevice,
    ) -> ControlPlaneDeviceResult<DeviceCredentials> {
        let body = RegisterDeviceBody {
            label: request.label,
            platform: request.platform.as_str(),
            public_signing_key: URL_SAFE_NO_PAD.encode(request.public_signing_key.as_bytes()),
            possession_signature: URL_SAFE_NO_PAD.encode(request.possession_signature.as_bytes()),
            import_display_name: request.import_display_name,
            import_locale: request.import_locale,
        };
        let response = self
            .client
            .post(self.register_url.clone())
            .bearer_auth(request.oidc_assertion.expose())
            .json(&body)
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        decode_credentials_response(response).await
    }

    async fn refresh_internal(
        &self,
        request: RefreshBridgeDevice,
    ) -> ControlPlaneDeviceResult<DeviceCredentials> {
        if !request.proof.nonce().expose().is_ascii() {
            return Err(failure(ControlPlaneDeviceFailureKind::InvalidRequest));
        }
        let response = self
            .client
            .post(self.refresh_url.clone())
            .bearer_auth(request.refresh_token.expose())
            .header(DEVICE_ID_HEADER, request.proof.device_id().to_string())
            .header(
                PROOF_ISSUED_AT_HEADER,
                request.proof.issued_at().value().to_string(),
            )
            .header(PROOF_NONCE_HEADER, request.proof.nonce().expose())
            .header(
                PROOF_SIGNATURE_HEADER,
                URL_SAFE_NO_PAD.encode(request.proof.signature().as_bytes()),
            )
            .body(Vec::new())
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        decode_credentials_response(response).await
    }
}

fn configured_client(
    config: &ControlPlaneHttpConfig,
) -> Result<(Client, Url), ControlPlaneHttpConfigurationError> {
    if config.request_timeout.is_zero() {
        return Err(ControlPlaneHttpConfigurationError::InvalidTimeout);
    }
    let base_url = Url::parse(&config.base_url)
        .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
    validate_base_url(&base_url)?;
    let client = Client::builder()
        .timeout(config.request_timeout)
        .connect_timeout(config.request_timeout)
        .redirect(Policy::none())
        .build()
        .map_err(|_| ControlPlaneHttpConfigurationError::HttpClient)?;
    Ok((client, base_url))
}

impl ReqwestAgentInstanceVerificationGateway {
    /// 创建只向固定控制面请求实例验签材料的 HTTP 网关。
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

    async fn resolve_internal(
        &self,
        instance_id: AgentInstanceId,
    ) -> AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord> {
        let request_target = format!("/agent-instances/{instance_id}/verification");
        let authorized = self
            .authorizer
            .authorize("GET", &request_target, "")
            .await
            .map_err(map_session_failure)?;
        let request_url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| {
                verification_failure(AgentInstanceVerificationGatewayFailureKind::Internal)
            })?;
        let request = signed_request(
            self.client.get(request_url),
            &authorized,
            "GET",
            &request_target,
        )?;
        let response = request.body(Vec::new()).send().await.map_err(|_| {
            verification_failure(AgentInstanceVerificationGatewayFailureKind::Unavailable)
        })?;
        decode_verification_response(response, instance_id).await
    }
}

impl ReqwestControlPlaneAgentRuntimeGateway {
    /// 创建以 Bridge 设备会话签名的 Agent 实例登记网关。
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

    async fn register_internal(
        &self,
        intent: &AgentRuntimeRegistrationIntent,
    ) -> ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime> {
        let request_target = format!("/agents/{}/instances", intent.agent_id());
        let body = serde_json::to_string(&RegisterAgentRuntimeBody {
            adapter_type: intent.adapter_type(),
            capability_version: intent.capability_version(),
            configuration: serde_json::Map::new(),
            public_signing_key: URL_SAFE_NO_PAD.encode(intent.public_signing_key().as_bytes()),
        })
        .map_err(|_| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::Internal))?;
        let authorized = self
            .authorizer
            .authorize("POST", &request_target, &body)
            .await
            .map_err(map_agent_runtime_session_failure)?;
        let request_url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::Internal))?;
        let request = signed_agent_runtime_request(
            self.client
                .post(request_url)
                .header(IDEMPOTENCY_KEY_HEADER, intent.request_id().to_string())
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            &request_target,
        )?;
        let response = request
            .body(body)
            .send()
            .await
            .map_err(|error| agent_runtime_transport_failure(&error))?;
        decode_agent_runtime_response(response, intent.agent_id()).await
    }
}

impl ReqwestControlPlaneOnboardingGateway {
    /// 创建以设备持有证明恢复默认 Agent、并只读取公开大厅目录的网关。
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

    async fn ensure_default_agent_internal(
        &self,
    ) -> ControlPlaneOnboardingResult<BridgeDefaultAgent> {
        let authorized = self
            .authorizer
            .authorize("PUT", DEVICE_DEFAULT_AGENT_PATH, "")
            .await
            .map_err(map_onboarding_session_failure)?;
        let request_url = self
            .base_url
            .join(DEVICE_DEFAULT_AGENT_PATH.trim_start_matches('/'))
            .map_err(|_| onboarding_failure(ControlPlaneOnboardingFailureKind::Internal))?;
        let request = signed_onboarding_request(
            self.client.put(request_url),
            &authorized,
            "PUT",
            DEVICE_DEFAULT_AGENT_PATH,
        )?;
        let response = request
            .body(Vec::new())
            .send()
            .await
            .map_err(onboarding_transport_failure)?;
        decode_default_agent_response(response).await
    }

    async fn list_public_lobbies_internal(
        &self,
    ) -> ControlPlaneOnboardingResult<Vec<BridgePublicLobby>> {
        let request_url = self
            .base_url
            .join(PUBLIC_LOBBIES_PATH.trim_start_matches('/'))
            .map_err(|_| onboarding_failure(ControlPlaneOnboardingFailureKind::Internal))?;
        let response = self
            .client
            .get(request_url)
            .send()
            .await
            .map_err(onboarding_transport_failure)?;
        decode_public_lobbies_response(response).await
    }
}

impl AgentInstanceVerificationGateway for ReqwestAgentInstanceVerificationGateway {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>>
    {
        Box::pin(self.resolve_internal(instance_id))
    }
}

impl ControlPlaneAgentRuntimeGateway for ReqwestControlPlaneAgentRuntimeGateway {
    fn register<'a>(
        &'a self,
        intent: &'a AgentRuntimeRegistrationIntent,
    ) -> PortFuture<'a, ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime>> {
        Box::pin(self.register_internal(intent))
    }
}

impl ControlPlaneOnboardingGateway for ReqwestControlPlaneOnboardingGateway {
    fn ensure_default_agent(
        &self,
    ) -> PortFuture<'_, ControlPlaneOnboardingResult<BridgeDefaultAgent>> {
        Box::pin(self.ensure_default_agent_internal())
    }

    fn list_public_lobbies(
        &self,
    ) -> PortFuture<'_, ControlPlaneOnboardingResult<Vec<BridgePublicLobby>>> {
        Box::pin(self.list_public_lobbies_internal())
    }
}

impl ControlPlaneDeviceGateway for ReqwestControlPlaneDeviceGateway {
    fn register(
        &self,
        request: RegisterBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        Box::pin(self.register_internal(request))
    }

    fn refresh(
        &self,
        request: RefreshBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        Box::pin(self.refresh_internal(request))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterDeviceBody {
    label: String,
    platform: &'static str,
    public_signing_key: String,
    possession_signature: String,
    import_display_name: bool,
    import_locale: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterAgentRuntimeBody<'a> {
    adapter_type: &'a str,
    capability_version: &'a str,
    configuration: serde_json::Map<String, serde_json::Value>,
    public_signing_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceCredentialsResponse {
    device_id: String,
    principal_id: String,
    matrix_user_id: String,
    display_name: String,
    avatar_content_id: Option<String>,
    locale: String,
    access_token: String,
    access_token_expires_at_unix_ms: i64,
    refresh_token: String,
    refresh_token_expires_at_unix_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentInstanceVerificationResponse {
    agent_instance_id: String,
    agent_id: String,
    public_signing_key: String,
    registered_at_unix_ms: i64,
    invalidated_at_unix_ms: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentRuntimeResponse {
    agent_id: String,
    display_name: String,
    avatar_content_id: Option<String>,
    adapter_binding_id: String,
    agent_instance_id: String,
    matrix_user_id: String,
    matrix_device_id: String,
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefaultAgentResponse {
    agent_id: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicLobbyDirectoryResponse {
    lobbies: Vec<PublicLobbyResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicLobbyResponse {
    catalog_id: String,
    language: Option<String>,
}

fn signed_request(
    request: reqwest::RequestBuilder,
    authorized: &AuthorizedControlPlaneRequest,
    expected_method: &str,
    expected_target: &str,
) -> Result<reqwest::RequestBuilder, AgentInstanceVerificationGatewayFailure> {
    signed_request_headers(request, authorized, expected_method, expected_target)
        .map_err(|()| verification_failure(AgentInstanceVerificationGatewayFailureKind::Internal))
}

fn signed_agent_runtime_request(
    request: reqwest::RequestBuilder,
    authorized: &AuthorizedControlPlaneRequest,
    expected_method: &str,
    expected_target: &str,
) -> Result<reqwest::RequestBuilder, ControlPlaneAgentRuntimeFailure> {
    signed_request_headers(request, authorized, expected_method, expected_target)
        .map_err(|()| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::Internal))
}

fn signed_onboarding_request(
    request: reqwest::RequestBuilder,
    authorized: &AuthorizedControlPlaneRequest,
    expected_method: &str,
    expected_target: &str,
) -> Result<reqwest::RequestBuilder, ControlPlaneOnboardingFailure> {
    signed_request_headers(request, authorized, expected_method, expected_target)
        .map_err(|()| onboarding_failure(ControlPlaneOnboardingFailureKind::Internal))
}

fn signed_request_headers(
    request: reqwest::RequestBuilder,
    authorized: &AuthorizedControlPlaneRequest,
    expected_method: &str,
    expected_target: &str,
) -> Result<reqwest::RequestBuilder, ()> {
    if !authorized.proof.nonce().expose().is_ascii()
        || authorized.proof.method() != expected_method
        || authorized.proof.request_target() != expected_target
    {
        return Err(());
    }
    Ok(request
        .bearer_auth(authorized.access_token.expose())
        .header(DEVICE_ID_HEADER, authorized.proof.device_id().to_string())
        .header(
            PROOF_ISSUED_AT_HEADER,
            authorized.proof.issued_at().value().to_string(),
        )
        .header(PROOF_NONCE_HEADER, authorized.proof.nonce().expose())
        .header(
            PROOF_SIGNATURE_HEADER,
            URL_SAFE_NO_PAD.encode(authorized.proof.signature().as_bytes()),
        ))
}

async fn decode_credentials_response(
    response: reqwest::Response,
) -> ControlPlaneDeviceResult<DeviceCredentials> {
    let status = response.status();
    if !status.is_success() {
        return Err(status_failure(status));
    }
    let body = read_limited_body(response).await?;
    let response = serde_json::from_slice::<DeviceCredentialsResponse>(&body)
        .map_err(|_| failure(ControlPlaneDeviceFailureKind::UnknownCommit))?;
    response
        .try_into()
        .map_err(|()| failure(ControlPlaneDeviceFailureKind::UnknownCommit))
}

async fn decode_verification_response(
    response: reqwest::Response,
    requested_instance_id: AgentInstanceId,
) -> AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord> {
    let status = response.status();
    if !status.is_success() {
        return Err(verification_status_failure(status));
    }
    let body = read_limited_verification_body(response).await?;
    let response =
        serde_json::from_slice::<AgentInstanceVerificationResponse>(&body).map_err(|_| {
            verification_failure(AgentInstanceVerificationGatewayFailureKind::InvalidResponse)
        })?;
    let record = AgentInstanceVerificationRecord::try_from(response).map_err(|()| {
        verification_failure(AgentInstanceVerificationGatewayFailureKind::InvalidResponse)
    })?;
    if record.instance_id != requested_instance_id {
        return Err(verification_failure(
            AgentInstanceVerificationGatewayFailureKind::InvalidResponse,
        ));
    }
    Ok(record)
}

async fn decode_agent_runtime_response(
    response: reqwest::Response,
    requested_agent_id: AgentId,
) -> ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime> {
    let status = response.status();
    if !status.is_success() {
        return Err(agent_runtime_status_failure(status));
    }
    let body = read_limited_response_body(response).await.map_err(|()| {
        agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
    })?;
    let response = serde_json::from_slice::<AgentRuntimeResponse>(&body)
        .map_err(|_| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse))?;
    response.try_into_runtime(requested_agent_id)
}

async fn decode_default_agent_response(
    response: reqwest::Response,
) -> ControlPlaneOnboardingResult<BridgeDefaultAgent> {
    let status = response.status();
    if !status.is_success() {
        return Err(onboarding_status_failure(status));
    }
    let body = read_limited_response_body(response)
        .await
        .map_err(|()| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    let response = serde_json::from_slice::<DefaultAgentResponse>(&body)
        .map_err(|_| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    let agent_id = parse_v7_id(&response.agent_id)
        .map(AgentId::from_uuid)
        .map_err(|()| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    validate_account_text(&response.display_name, 128)
        .map_err(|()| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    Ok(BridgeDefaultAgent {
        agent_id,
        display_name: response.display_name,
    })
}

async fn decode_public_lobbies_response(
    response: reqwest::Response,
) -> ControlPlaneOnboardingResult<Vec<BridgePublicLobby>> {
    let status = response.status();
    if !status.is_success() {
        return Err(onboarding_status_failure(status));
    }
    let body = read_limited_response_body(response)
        .await
        .map_err(|()| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    let response = serde_json::from_slice::<PublicLobbyDirectoryResponse>(&body)
        .map_err(|_| onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse))?;
    response
        .lobbies
        .into_iter()
        .map(|lobby| {
            let catalog_id = parse_v7_id(&lobby.catalog_id)
                .map(RoomCatalogId::from_uuid)
                .map_err(|()| {
                    onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse)
                })?;
            let language = lobby
                .language
                .map(RoomLanguage::new)
                .transpose()
                .map_err(|_| {
                    onboarding_failure(ControlPlaneOnboardingFailureKind::InvalidResponse)
                })?;
            Ok(BridgePublicLobby {
                catalog_id,
                language,
            })
        })
        .collect()
}

async fn read_limited_body(response: reqwest::Response) -> ControlPlaneDeviceResult<Vec<u8>> {
    read_limited_response_body(response)
        .await
        .map_err(|()| failure(ControlPlaneDeviceFailureKind::UnknownCommit))
}

async fn read_limited_verification_body(
    response: reqwest::Response,
) -> AgentInstanceVerificationGatewayResult<Vec<u8>> {
    read_limited_response_body(response).await.map_err(|()| {
        verification_failure(AgentInstanceVerificationGatewayFailureKind::InvalidResponse)
    })
}

async fn read_limited_response_body(mut response: reqwest::Response) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl TryFrom<DeviceCredentialsResponse> for DeviceCredentials {
    type Error = ();

    fn try_from(value: DeviceCredentialsResponse) -> Result<Self, Self::Error> {
        validate_account_text(&value.matrix_user_id, 512)?;
        if !value.matrix_user_id.starts_with('@') || !value.matrix_user_id.contains(':') {
            return Err(());
        }
        validate_account_text(&value.display_name, 128)?;
        validate_locale(&value.locale)?;
        let device_id = parse_id(&value.device_id).map(DeviceId::from_uuid)?;
        let principal_id = parse_id(&value.principal_id).map(PrincipalId::from_uuid)?;
        let avatar_content_id = value
            .avatar_content_id
            .map(|content_id| parse_id(&content_id).map(ContentId::from_uuid))
            .transpose()?;
        let access_token = SecretValue::new(value.access_token).map_err(|_| ())?;
        let refresh_token = SecretValue::new(value.refresh_token).map_err(|_| ())?;
        let access_token_expires_at =
            UtcMillis::new(value.access_token_expires_at_unix_ms).map_err(|_| ())?;
        let refresh_token_expires_at =
            UtcMillis::new(value.refresh_token_expires_at_unix_ms).map_err(|_| ())?;
        if access_token_expires_at >= refresh_token_expires_at {
            return Err(());
        }

        Ok(Self {
            device: AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(principal_id),
                    matrix_user_id: value.matrix_user_id,
                    display_name: value.display_name,
                    avatar_content_id,
                    locale: value.locale,
                },
                device_id,
                access_token_expires_at,
            },
            access_token,
            refresh_token,
            refresh_token_expires_at,
        })
    }
}

impl TryFrom<AgentInstanceVerificationResponse> for AgentInstanceVerificationRecord {
    type Error = ();

    fn try_from(value: AgentInstanceVerificationResponse) -> Result<Self, Self::Error> {
        let instance_id = parse_v7_id(&value.agent_instance_id).map(AgentInstanceId::from_uuid)?;
        let agent_id = parse_v7_id(&value.agent_id).map(AgentId::from_uuid)?;
        let public_signing_key = URL_SAFE_NO_PAD
            .decode(value.public_signing_key)
            .map_err(|_| ())
            .and_then(|bytes| AgentInstancePublicSigningKey::new(bytes).map_err(|_| ()))?;
        let registered_at = UtcMillis::new(value.registered_at_unix_ms).map_err(|_| ())?;
        let invalidated_at = value
            .invalidated_at_unix_ms
            .map(|timestamp| UtcMillis::new(timestamp).map_err(|_| ()))
            .transpose()?;
        if invalidated_at.is_some_and(|timestamp| timestamp < registered_at) {
            return Err(());
        }
        Ok(Self {
            instance_id,
            agent_id,
            public_signing_key,
            registered_at,
            invalidated_at,
        })
    }
}

impl AgentRuntimeResponse {
    fn try_into_runtime(
        self,
        requested_agent_id: AgentId,
    ) -> ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime> {
        let agent_id = parse_v7_id(&self.agent_id)
            .map(AgentId::from_uuid)
            .map_err(|()| {
                agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
            })?;
        if agent_id != requested_agent_id {
            return Err(agent_runtime_failure(
                ControlPlaneAgentRuntimeFailureKind::InvalidResponse,
            ));
        }
        if let Some(content_id) = self.avatar_content_id {
            parse_v7_id(&content_id).map_err(|()| {
                agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
            })?;
        }
        let binding_id = parse_v7_id(&self.adapter_binding_id)
            .map(AdapterBindingId::from_uuid)
            .map_err(|()| {
                agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
            })?;
        let instance_id = parse_v7_id(&self.agent_instance_id)
            .map(AgentInstanceId::from_uuid)
            .map_err(|()| {
                agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
            })?;
        let matrix_user_id = MatrixUserId::new(self.matrix_user_id).map_err(|_| {
            agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
        })?;
        let matrix_device_id = MatrixDeviceId::new(self.matrix_device_id).map_err(|_| {
            agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
        })?;
        let access_token = SecretValue::new(self.access_token).map_err(|_| {
            agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
        })?;
        let refresh_token = self
            .refresh_token
            .map(SecretValue::new)
            .transpose()
            .map_err(|_| {
                agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse)
            })?;
        let identity = BridgeAgentIdentity::new(
            agent_id,
            self.display_name,
            matrix_user_id.as_str(),
            instance_id,
        )
        .map_err(|_| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse))?;
        RegisteredAgentRuntime::new(
            identity,
            binding_id,
            MatrixSession::new(
                MatrixSessionMetadata::new(matrix_user_id, matrix_device_id),
                access_token,
                refresh_token,
            ),
        )
        .map_err(|_| agent_runtime_failure(ControlPlaneAgentRuntimeFailureKind::InvalidResponse))
    }
}

fn validate_base_url(url: &Url) -> Result<(), ControlPlaneHttpConfigurationError> {
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.scheme() == "http" && !is_loopback
    {
        return Err(ControlPlaneHttpConfigurationError::InvalidBaseUrl);
    }
    Ok(())
}

fn parse_id(value: &str) -> Result<Uuid, ()> {
    Uuid::parse_str(value).map_err(|_| ())
}

fn parse_v7_id(value: &str) -> Result<Uuid, ()> {
    let id = parse_id(value)?;
    if id.get_version() == Some(Version::SortRand) {
        Ok(id)
    } else {
        Err(())
    }
}

fn validate_account_text(value: &str, maximum_length: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

fn validate_locale(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(());
    }
    Ok(())
}

fn transport_failure(error: &reqwest::Error) -> ControlPlaneDeviceFailure {
    if error.is_connect() {
        failure(ControlPlaneDeviceFailureKind::DependencyUnavailable)
    } else {
        failure(ControlPlaneDeviceFailureKind::UnknownCommit)
    }
}

fn status_failure(status: StatusCode) -> ControlPlaneDeviceFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            ControlPlaneDeviceFailureKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ControlPlaneDeviceFailureKind::AuthenticationRejected
        }
        StatusCode::CONFLICT => ControlPlaneDeviceFailureKind::Conflict,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => ControlPlaneDeviceFailureKind::DependencyUnavailable,
        _ if status.is_server_error() => ControlPlaneDeviceFailureKind::DependencyUnavailable,
        _ => ControlPlaneDeviceFailureKind::Internal,
    };
    failure(kind)
}

fn verification_status_failure(status: StatusCode) -> AgentInstanceVerificationGatewayFailure {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            AgentInstanceVerificationGatewayFailureKind::AuthenticationRejected
        }
        StatusCode::NOT_FOUND => AgentInstanceVerificationGatewayFailureKind::NotFound,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => AgentInstanceVerificationGatewayFailureKind::Unavailable,
        _ if status.is_server_error() => AgentInstanceVerificationGatewayFailureKind::Unavailable,
        _ => AgentInstanceVerificationGatewayFailureKind::InvalidResponse,
    };
    verification_failure(kind)
}

fn agent_runtime_transport_failure(error: &reqwest::Error) -> ControlPlaneAgentRuntimeFailure {
    let kind = if error.is_connect() {
        ControlPlaneAgentRuntimeFailureKind::Unavailable
    } else {
        ControlPlaneAgentRuntimeFailureKind::UnknownCommit
    };
    agent_runtime_failure(kind)
}

fn agent_runtime_status_failure(status: StatusCode) -> ControlPlaneAgentRuntimeFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            ControlPlaneAgentRuntimeFailureKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED => ControlPlaneAgentRuntimeFailureKind::AuthenticationRejected,
        StatusCode::FORBIDDEN => ControlPlaneAgentRuntimeFailureKind::Forbidden,
        StatusCode::NOT_FOUND => ControlPlaneAgentRuntimeFailureKind::NotFound,
        StatusCode::CONFLICT => ControlPlaneAgentRuntimeFailureKind::Conflict,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => ControlPlaneAgentRuntimeFailureKind::Unavailable,
        _ if status.is_server_error() => ControlPlaneAgentRuntimeFailureKind::Unavailable,
        _ => ControlPlaneAgentRuntimeFailureKind::InvalidResponse,
    };
    agent_runtime_failure(kind)
}

fn onboarding_transport_failure(_error: reqwest::Error) -> ControlPlaneOnboardingFailure {
    onboarding_failure(ControlPlaneOnboardingFailureKind::Unavailable)
}

fn onboarding_status_failure(status: StatusCode) -> ControlPlaneOnboardingFailure {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ControlPlaneOnboardingFailureKind::AuthenticationRejected
        }
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => ControlPlaneOnboardingFailureKind::Unavailable,
        _ if status.is_server_error() => ControlPlaneOnboardingFailureKind::Unavailable,
        _ => ControlPlaneOnboardingFailureKind::InvalidResponse,
    };
    onboarding_failure(kind)
}

fn map_session_failure(failure: BridgeSessionFailure) -> AgentInstanceVerificationGatewayFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            AgentInstanceVerificationGatewayFailureKind::AuthenticationRejected
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            AgentInstanceVerificationGatewayFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => {
            AgentInstanceVerificationGatewayFailureKind::Internal
        }
    };
    verification_failure(kind)
}

fn map_agent_runtime_session_failure(
    failure: BridgeSessionFailure,
) -> ControlPlaneAgentRuntimeFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            ControlPlaneAgentRuntimeFailureKind::AuthenticationRejected
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            ControlPlaneAgentRuntimeFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => ControlPlaneAgentRuntimeFailureKind::Internal,
    };
    agent_runtime_failure(kind)
}

fn map_onboarding_session_failure(failure: BridgeSessionFailure) -> ControlPlaneOnboardingFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            ControlPlaneOnboardingFailureKind::AuthenticationRejected
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            ControlPlaneOnboardingFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => ControlPlaneOnboardingFailureKind::Internal,
    };
    onboarding_failure(kind)
}

const fn failure(kind: ControlPlaneDeviceFailureKind) -> ControlPlaneDeviceFailure {
    ControlPlaneDeviceFailure::new(kind)
}

const fn verification_failure(
    kind: AgentInstanceVerificationGatewayFailureKind,
) -> AgentInstanceVerificationGatewayFailure {
    AgentInstanceVerificationGatewayFailure::new(kind)
}

const fn agent_runtime_failure(
    kind: ControlPlaneAgentRuntimeFailureKind,
) -> ControlPlaneAgentRuntimeFailure {
    ControlPlaneAgentRuntimeFailure::new(kind)
}

const fn onboarding_failure(
    kind: ControlPlaneOnboardingFailureKind,
) -> ControlPlaneOnboardingFailure {
    ControlPlaneOnboardingFailure::new(kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneHttpConfigurationError {
    InvalidBaseUrl,
    InvalidTimeout,
    HttpClient,
}

impl std::fmt::Display for ControlPlaneHttpConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidBaseUrl => "控制面地址无效或使用了不安全的明文传输",
            Self::InvalidTimeout => "控制面请求超时配置无效",
            Self::HttpClient => "控制面 HTTP 客户端初始化失败",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ControlPlaneHttpConfigurationError {}

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
        agent_runtime::{
            AgentRuntimeRegistrationIntent, ControlPlaneAgentRuntimeFailureKind,
            ControlPlaneAgentRuntimeGateway,
        },
        agent_verification::{
            AgentInstanceVerificationGateway, AgentInstanceVerificationGatewayFailureKind,
        },
        onboarding::ControlPlaneOnboardingGateway,
        ports::{
            ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway, RefreshBridgeDevice,
            RegisterBridgeDevice,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{
        agents::AgentInstancePublicSigningKey,
        devices::{DevicePlatform, DevicePublicSigningKey},
        ids::{AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId, DeviceId},
        rooms::RoomLanguage,
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        body::HttpBody,
        extract::{Path, Request},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post, put},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{
        ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError,
        ReqwestAgentInstanceVerificationGateway, ReqwestControlPlaneAgentRuntimeGateway,
        ReqwestControlPlaneDeviceGateway, ReqwestControlPlaneOnboardingGateway,
    };

    const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const LOBBY_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";

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
                device_id(),
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
    async fn 注册请求只向固定地址发送规范载荷() {
        let app = Router::new().route(
            "/auth/devices/register",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                let valid = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer oidc-assertion")
                    && body
                        == json!({
                            "label": "Windows 设备",
                            "platform": "windows",
                            "publicSigningKey": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
                            "possessionSignature": "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQ",
                            "importDisplayName": true,
                            "importLocale": true
                        });
                if valid {
                    (StatusCode::OK, Json(valid_credentials_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
        let gateway = gateway(spawn_server(app).await);

        let credentials = gateway
            .register(RegisterBridgeDevice {
                oidc_assertion: secret("oidc-assertion"),
                label: "Windows 设备".to_owned(),
                platform: DevicePlatform::Windows,
                public_signing_key: DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
                possession_signature: DeviceSignature::new(vec![9; 64]).expect("测试签名有效"),
                import_display_name: true,
                import_locale: true,
            })
            .await
            .expect("规范注册请求应成功");

        assert_eq!(credentials.device.device_id, device_id());
        assert_eq!(credentials.device.account.display_name, "测试用户");
        assert_eq!(credentials.access_token.expose(), "access-token");
    }

    #[tokio::test]
    async fn 刷新请求携带发送方约束证明且不在正文复制令牌() {
        let app = Router::new().route(
            "/auth/devices/refresh",
            post(|headers: HeaderMap, request: Request| async move {
                let valid = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer refresh-token")
                    && header(&headers, "x-agent-room-device-id")
                        == Some("00000000-0000-0000-0000-000000000001")
                    && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                    && header(&headers, "x-agent-room-proof-nonce")
                        == Some("0123456789abcdef")
                    && header(&headers, "x-agent-room-proof-signature")
                        == Some("BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQ")
                    && request.body().size_hint().exact() == Some(0);
                if valid {
                    (StatusCode::OK, Json(valid_credentials_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
        let gateway = gateway(spawn_server(app).await);
        let secrets = SecureSecretFactory;
        let payload = DeviceRequestProofPayload::new(
            device_id(),
            UtcMillis::new(1_000).expect("测试时间有效"),
            secret("0123456789abcdef"),
            "POST".to_owned(),
            "/auth/devices/refresh".to_owned(),
            secrets.digest(""),
        )
        .expect("测试证明有效");

        let credentials = gateway
            .refresh(RefreshBridgeDevice {
                refresh_token: secret("refresh-token"),
                proof: DeviceRequestProof::new(
                    payload,
                    DeviceSignature::new(vec![5; 64]).expect("测试签名有效"),
                ),
            })
            .await
            .expect("规范刷新请求应成功");

        assert_eq!(credentials.refresh_token.expose(), "next-refresh-token");
    }

    #[tokio::test]
    async fn 实例验签网关发送精确持有证明并严格解析时间窗() {
        let app = Router::new().route(
            "/agent-instances/{instance_id}/verification",
            get(
                |Path(instance_id): Path<String>, headers: HeaderMap, request: Request| async move {
                    let valid = instance_id == INSTANCE_ID
                        && header(&headers, "authorization") == Some("Bearer access-token")
                        && header(&headers, "x-agent-room-device-id")
                            == Some("00000000-0000-0000-0000-000000000001")
                        && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                        && header(&headers, "x-agent-room-proof-nonce")
                            == Some("0123456789abcdef")
                        && header(&headers, "x-agent-room-proof-signature")
                            == Some("BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQ")
                        && request.body().size_hint().exact() == Some(0);
                    if valid {
                        (StatusCode::OK, Json(valid_verification_response())).into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                },
            ),
        );
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = verification_gateway(spawn_server(app).await, authorizer.clone());

        let record = gateway
            .resolve(instance_id())
            .await
            .expect("规范验签材料响应可解析");

        assert_eq!(record.instance_id, instance_id());
        assert_eq!(record.agent_id.to_string(), AGENT_ID);
        assert_eq!(record.public_signing_key.as_bytes(), &[7_u8; 32]);
        assert_eq!(record.registered_at.value(), 1_000);
        assert_eq!(record.invalidated_at.map(UtcMillis::value), Some(2_000));
        assert_eq!(
            authorizer
                .requests
                .lock()
                .expect("授权请求记录锁可用")
                .as_slice(),
            [(
                "GET".to_owned(),
                format!("/agent-instances/{INSTANCE_ID}/verification"),
                String::new(),
            )]
        );
    }

    #[tokio::test]
    async fn agent_运行时网关以同一正文完成设备证明和幂等登记() {
        let app = Router::new().route(
            "/agents/{agent_id}/instances",
            post(
                |Path(agent_id): Path<String>, headers: HeaderMap, body: String| async move {
                    let parsed = serde_json::from_str::<Value>(&body).ok();
                    let valid = agent_id == AGENT_ID
                        && header(&headers, "authorization") == Some("Bearer access-token")
                        && header(&headers, "idempotency-key")
                            == Some("0198b601-77a1-7bb8-83eb-a8fe68c97e49")
                        && header(&headers, "content-type") == Some("application/json")
                        && header(&headers, "x-agent-room-device-id")
                            == Some("00000000-0000-0000-0000-000000000001")
                        && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                        && header(&headers, "x-agent-room-proof-nonce") == Some("0123456789abcdef")
                        && parsed
                            == Some(json!({
                                "adapterType": "codex-desktop",
                                "capabilityVersion": "2026-08-24",
                                "configuration": {},
                                "publicSigningKey": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc"
                            }));
                    if valid {
                        (StatusCode::OK, Json(valid_agent_runtime_response())).into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                },
            ),
        );
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = agent_runtime_gateway(spawn_server(app).await, authorizer.clone());

        let runtime = gateway
            .register(&agent_runtime_intent())
            .await
            .expect("规范实例登记响应可解析");

        assert_eq!(runtime.identity().agent_id(), agent_id());
        assert_eq!(runtime.identity().display_name(), "Codex Builder");
        assert_eq!(runtime.identity().agent_instance_id(), instance_id());
        assert_eq!(
            runtime.matrix_session().access_token().expose(),
            "agent-device-access-token"
        );
        let requests = authorizer.requests.lock().expect("授权请求记录锁可用");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "POST");
        assert_eq!(requests[0].1, format!("/agents/{AGENT_ID}/instances"));
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].2).expect("签名正文是 JSON"),
            json!({
                "adapterType": "codex-desktop",
                "capabilityVersion": "2026-08-24",
                "configuration": {},
                "publicSigningKey": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc"
            })
        );
    }

    #[tokio::test]
    async fn 桌面首次引导只给默认_agent_请求签名并解析公开大厅() {
        let app = Router::new()
            .route(
                "/onboarding/device/default-agent",
                put(|headers: HeaderMap, request: Request| async move {
                    let valid = header(&headers, "authorization") == Some("Bearer access-token")
                        && header(&headers, "x-agent-room-device-id")
                            == Some("00000000-0000-0000-0000-000000000001")
                        && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                        && header(&headers, "x-agent-room-proof-nonce") == Some("0123456789abcdef")
                        && request.body().size_hint().exact() == Some(0);
                    if valid {
                        Json(json!({
                            "agentId": AGENT_ID,
                            "displayName": "Codex Builder",
                            "matrixUserId": "@agent:example.org",
                            "slug": "codex-builder"
                        }))
                        .into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                }),
            )
            .route(
                "/lobbies/public",
                get(|| async {
                    Json(json!({
                        "lobbies": [{
                            "catalogId": LOBBY_ID,
                            "language": "zh-CN",
                            "name": "公共大厅"
                        }]
                    }))
                }),
            );
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = onboarding_gateway(spawn_server(app).await, authorizer.clone());

        let agent = gateway
            .ensure_default_agent()
            .await
            .expect("默认 Agent 响应可解析");
        let lobbies = gateway
            .list_public_lobbies()
            .await
            .expect("公开大厅目录可解析");

        assert_eq!(agent.agent_id.to_string(), AGENT_ID);
        assert_eq!(agent.display_name, "Codex Builder");
        assert_eq!(lobbies.len(), 1);
        assert_eq!(lobbies[0].catalog_id.to_string(), LOBBY_ID);
        assert_eq!(
            lobbies[0].language.as_ref().map(RoomLanguage::as_str),
            Some("zh-CN")
        );
        assert_eq!(
            authorizer
                .requests
                .lock()
                .expect("授权请求记录锁可用")
                .as_slice(),
            [(
                "PUT".to_owned(),
                "/onboarding/device/default-agent".to_owned(),
                String::new(),
            )]
        );
    }

    #[tokio::test]
    async fn agent_运行时网关拒绝错配身份和未知响应字段() {
        let wrong_identity = Router::new().route(
            "/agents/{agent_id}/instances",
            post(|| async {
                let mut response = valid_agent_runtime_response();
                response["agentId"] = json!("0198b601-77a1-7bb8-83eb-a8fe68c97e45");
                Json(response)
            }),
        );
        let unknown_field = Router::new().route(
            "/agents/{agent_id}/instances",
            post(|| async {
                let mut response = valid_agent_runtime_response();
                response["unexpected"] = json!(true);
                Json(response)
            }),
        );

        for app in [wrong_identity, unknown_field] {
            let failure =
                agent_runtime_gateway(spawn_server(app).await, Arc::new(测试请求授权器::default()))
                    .register(&agent_runtime_intent())
                    .await
                    .expect_err("不可信响应必须失败");
            assert_eq!(
                failure.kind(),
                ControlPlaneAgentRuntimeFailureKind::InvalidResponse
            );
        }
    }

    #[tokio::test]
    async fn 实例验签网关区分不存在与畸形成功响应() {
        let not_found = Router::new().route(
            "/agent-instances/{instance_id}/verification",
            get(|| async { StatusCode::NOT_FOUND }),
        );
        let malformed = Router::new().route(
            "/agent-instances/{instance_id}/verification",
            get(|| async {
                Json(json!({
                    "agentInstanceId": INSTANCE_ID,
                    "agentId": AGENT_ID,
                    "publicSigningKey": "invalid",
                    "registeredAtUnixMs": 1_000,
                    "invalidatedAtUnixMs": null
                }))
            }),
        );

        let missing = verification_gateway(
            spawn_server(not_found).await,
            Arc::new(测试请求授权器::default()),
        )
        .resolve(instance_id())
        .await
        .expect_err("不存在的实例必须失败");
        let invalid = verification_gateway(
            spawn_server(malformed).await,
            Arc::new(测试请求授权器::default()),
        )
        .resolve(instance_id())
        .await
        .expect_err("畸形成功响应必须失败");

        assert_eq!(
            missing.kind(),
            AgentInstanceVerificationGatewayFailureKind::NotFound
        );
        assert_eq!(
            invalid.kind(),
            AgentInstanceVerificationGatewayFailureKind::InvalidResponse
        );
    }

    #[tokio::test]
    async fn 已提交语义不明的响应绝不被当作可安全重试() {
        for app in [
            Router::new().route(
                "/auth/devices/register",
                post(|| async { (StatusCode::OK, "not-json") }),
            ),
            Router::new().route(
                "/auth/devices/register",
                post(|| async { "x".repeat(65 * 1_024) }),
            ),
        ] {
            let failure = gateway(spawn_server(app).await)
                .register(register_request())
                .await
                .expect_err("畸形成功响应必须进入未知提交状态");

            assert_eq!(failure.kind(), ControlPlaneDeviceFailureKind::UnknownCommit);
        }
    }

    #[tokio::test]
    async fn 服务端冲突映射为稳定业务错误() {
        let app = Router::new().route(
            "/auth/devices/register",
            post(|| async { StatusCode::CONFLICT }),
        );

        let failure = gateway(spawn_server(app).await)
            .register(register_request())
            .await
            .expect_err("冲突响应必须失败");

        assert_eq!(failure.kind(), ControlPlaneDeviceFailureKind::Conflict);
    }

    #[test]
    fn 非回环地址禁止使用明文_http() {
        let result = ReqwestControlPlaneDeviceGateway::new(&ControlPlaneHttpConfig {
            base_url: "http://example.com/".to_owned(),
            request_timeout: Duration::from_secs(1),
        });

        assert!(matches!(
            result,
            Err(ControlPlaneHttpConfigurationError::InvalidBaseUrl)
        ));
    }

    fn gateway(base_url: String) -> ReqwestControlPlaneDeviceGateway {
        ReqwestControlPlaneDeviceGateway::new(&ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        })
        .expect("本地测试地址有效")
    }

    fn verification_gateway(
        base_url: String,
        authorizer: Arc<测试请求授权器>,
    ) -> ReqwestAgentInstanceVerificationGateway {
        ReqwestAgentInstanceVerificationGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("本地验签材料网关地址有效")
    }

    fn agent_runtime_gateway(
        base_url: String,
        authorizer: Arc<测试请求授权器>,
    ) -> ReqwestControlPlaneAgentRuntimeGateway {
        ReqwestControlPlaneAgentRuntimeGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("本地 Agent 运行时网关地址有效")
    }

    fn onboarding_gateway(
        base_url: String,
        authorizer: Arc<测试请求授权器>,
    ) -> ReqwestControlPlaneOnboardingGateway {
        ReqwestControlPlaneOnboardingGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("本地首次引导网关地址有效")
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

    fn register_request() -> RegisterBridgeDevice {
        RegisterBridgeDevice {
            oidc_assertion: secret("oidc-assertion"),
            label: "Windows 设备".to_owned(),
            platform: DevicePlatform::Windows,
            public_signing_key: DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
            possession_signature: DeviceSignature::new(vec![9; 64]).expect("测试签名有效"),
            import_display_name: true,
            import_locale: true,
        }
    }

    fn valid_credentials_response() -> Value {
        json!({
            "deviceId": "00000000-0000-0000-0000-000000000001",
            "principalId": "00000000-0000-0000-0000-000000000002",
            "matrixUserId": "@test:matrix.agent-room.localhost",
            "displayName": "测试用户",
            "avatarContentId": null,
            "locale": "zh-CN",
            "accessToken": "access-token",
            "accessTokenExpiresAtUnixMs": 2_000,
            "refreshToken": "next-refresh-token",
            "refreshTokenExpiresAtUnixMs": 3_000
        })
    }

    fn valid_verification_response() -> Value {
        json!({
            "agentInstanceId": INSTANCE_ID,
            "agentId": AGENT_ID,
            "publicSigningKey": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
            "registeredAtUnixMs": 1_000,
            "invalidatedAtUnixMs": 2_000
        })
    }

    fn valid_agent_runtime_response() -> Value {
        json!({
            "agentId": AGENT_ID,
            "displayName": "Codex Builder",
            "avatarContentId": null,
            "adapterBindingId": "0198b601-77a1-7bb8-83eb-a8fe68c97e48",
            "agentInstanceId": INSTANCE_ID,
            "matrixUserId": "@agent:example.org",
            "matrixDeviceId": "AR_TEST",
            "accessToken": "agent-device-access-token",
            "refreshToken": null
        })
    }

    fn agent_runtime_intent() -> AgentRuntimeRegistrationIntent {
        AgentRuntimeRegistrationIntent::new(
            AgentInstanceRegistrationRequestId::from_uuid(
                Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e49").expect("测试请求标识有效"),
            ),
            agent_id(),
            "codex-desktop",
            "2026-08-24",
            AgentInstancePublicSigningKey::new(vec![7; 32]).expect("测试实例公钥有效"),
        )
        .expect("测试登记意图有效")
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("测试 Agent 标识有效"))
    }

    fn secret(value: &str) -> SecretValue {
        SecretValue::new(value).expect("测试密钥有效")
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(Uuid::from_u128(1))
    }

    fn instance_id() -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::parse_str(INSTANCE_ID).expect("测试实例标识有效"))
    }

    fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }
}
