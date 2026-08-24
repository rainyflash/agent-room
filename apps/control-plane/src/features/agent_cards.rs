use std::sync::Arc;

use agent_room_application::{
    agent_cards::{AgentCardChange, AgentCardRefresh, AgentCardUseCases, RefreshAgentCard},
    devices::DeviceAuthorizationUseCases,
    ports::SecretFactory,
};
use agent_room_domain::{
    agent_cards::{
        AgentCardCapabilities, AgentCardEndpoint, AgentCardProvider, AgentCardSkill,
        NormalizedAgentCard,
    },
    ids::AgentId,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::no_store, devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const MAX_AGENT_CARD_REFRESH_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct AgentCardHttpState {
    cards: Arc<dyn AgentCardUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
}

pub(crate) struct AgentCardHttpDependencies {
    pub(crate) cards: Arc<dyn AgentCardUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl AgentCardHttpState {
    pub(crate) fn new(dependencies: AgentCardHttpDependencies) -> Self {
        Self {
            cards: dependencies.cards,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
        }
    }
}

pub(crate) fn router(state: AgentCardHttpState) -> Router {
    Router::new()
        .route(
            "/agents/{agent_id}/agent-card/refresh",
            post(refresh_agent_card),
        )
        .layer(DefaultBodyLimit::max(MAX_AGENT_CARD_REFRESH_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshAgentCardBody {
    source_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardRefreshResponse {
    agent_id: String,
    source_url: String,
    verification: &'static str,
    change: &'static str,
    fetched_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    card: AgentCardResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardResponse {
    name: String,
    description: String,
    provider: Option<AgentCardProviderResponse>,
    version: String,
    endpoints: Vec<AgentCardEndpointResponse>,
    capabilities: AgentCardCapabilitiesResponse,
    security_schemes: Vec<AgentCardSecuritySchemeResponse>,
    default_input_modes: Vec<String>,
    default_output_modes: Vec<String>,
    skills: Vec<AgentCardSkillResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardProviderResponse {
    organization: String,
    url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardEndpointResponse {
    url: String,
    protocol_binding: &'static str,
    protocol_version: String,
    tenant: Option<String>,
    verification: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardCapabilitiesResponse {
    streaming: bool,
    push_notifications: bool,
    extended_agent_card: bool,
    extensions: Vec<AgentCardExtensionResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardExtensionResponse {
    uri: String,
    description: String,
    required: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardSecuritySchemeResponse {
    name: String,
    kind: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCardSkillResponse {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    input_modes: Vec<String>,
    output_modes: Vec<String>,
}

async fn refresh_agent_card(
    State(state): State<AgentCardHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agents/{agent_id}/agent-card/refresh");
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(
            ApiError::invalid_request("agent_card.invalid_agent_id", correlation_id)
                .into_response(),
        );
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(
            ApiError::invalid_request("agent_card.invalid_refresh_body", correlation_id)
                .into_response(),
        );
    };
    let actor = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "POST",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<RefreshAgentCardBody>(&body) else {
        return no_store(
            ApiError::invalid_request("agent_card.invalid_refresh_body", correlation_id)
                .into_response(),
        );
    };
    let Ok(source_url) = agent_room_domain::agent_cards::AgentCardSourceUrl::new(body.source_url)
    else {
        return no_store(
            ApiError::invalid_request("agent_card.invalid_source_url", correlation_id)
                .into_response(),
        );
    };

    match state
        .cards
        .refresh(RefreshAgentCard {
            actor,
            agent_id,
            source_url,
        })
        .await
    {
        Ok(refresh) => no_store(Json(AgentCardRefreshResponse::from(refresh)).into_response()),
        Err(failure) => no_store(ApiError::agent_card(failure, correlation_id).into_response()),
    }
}

impl From<AgentCardRefresh> for AgentCardRefreshResponse {
    fn from(value: AgentCardRefresh) -> Self {
        let snapshot = value.snapshot;
        let card = AgentCardResponse::from(snapshot.card());
        Self {
            agent_id: snapshot.agent_id().to_string(),
            source_url: snapshot.source_url().to_owned(),
            verification: snapshot.stored_verification().as_str(),
            change: change_name(value.change),
            fetched_at_unix_ms: snapshot.fetched_at().value(),
            expires_at_unix_ms: snapshot.expires_at().value(),
            card,
        }
    }
}

impl From<&NormalizedAgentCard> for AgentCardResponse {
    fn from(value: &NormalizedAgentCard) -> Self {
        Self {
            name: value.name().to_owned(),
            description: value.description().to_owned(),
            provider: value.provider().map(AgentCardProviderResponse::from),
            version: value.version().to_owned(),
            endpoints: value
                .endpoints()
                .iter()
                .map(AgentCardEndpointResponse::from)
                .collect(),
            capabilities: AgentCardCapabilitiesResponse::from(value.capabilities()),
            security_schemes: value
                .security_schemes()
                .iter()
                .map(|scheme| AgentCardSecuritySchemeResponse {
                    name: scheme.name().to_owned(),
                    kind: scheme.kind().as_str(),
                })
                .collect(),
            default_input_modes: value.default_input_modes().to_vec(),
            default_output_modes: value.default_output_modes().to_vec(),
            skills: value
                .skills()
                .iter()
                .map(AgentCardSkillResponse::from)
                .collect(),
        }
    }
}

impl From<&AgentCardProvider> for AgentCardProviderResponse {
    fn from(value: &AgentCardProvider) -> Self {
        Self {
            organization: value.organization().to_owned(),
            url: value.url().to_owned(),
        }
    }
}

impl From<&AgentCardEndpoint> for AgentCardEndpointResponse {
    fn from(value: &AgentCardEndpoint) -> Self {
        let version = value.protocol_version();
        Self {
            url: value.url().to_owned(),
            protocol_binding: value.transport().as_str(),
            protocol_version: format!("{}.{}", version.major(), version.minor()),
            tenant: value.tenant().map(str::to_owned),
            verification: value.verification().as_str(),
        }
    }
}

impl From<&AgentCardCapabilities> for AgentCardCapabilitiesResponse {
    fn from(value: &AgentCardCapabilities) -> Self {
        Self {
            streaming: value.streaming(),
            push_notifications: value.push_notifications(),
            extended_agent_card: value.extended_agent_card(),
            extensions: value
                .extensions()
                .iter()
                .map(|extension| AgentCardExtensionResponse {
                    uri: extension.uri().to_owned(),
                    description: extension.description().to_owned(),
                    required: extension.required(),
                })
                .collect(),
        }
    }
}

impl From<&AgentCardSkill> for AgentCardSkillResponse {
    fn from(value: &AgentCardSkill) -> Self {
        Self {
            id: value.id().to_owned(),
            name: value.name().to_owned(),
            description: value.description().to_owned(),
            tags: value.tags().to_vec(),
            input_modes: value.input_modes().to_vec(),
            output_modes: value.output_modes().to_vec(),
        }
    }
}

const fn change_name(value: AgentCardChange) -> &'static str {
    match value {
        AgentCardChange::Initial => "initial",
        AgentCardChange::Unchanged => "unchanged",
        AgentCardChange::ProfileChanged => "profile_changed",
        AgentCardChange::CapabilitySurfaceChanged => "capability_surface_changed",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use agent_room_application::{
        agent_cards::{
            AgentCardManagementResult, AgentCardRefresh, AgentCardUseCases, RefreshAgentCard,
        },
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        },
        ports::{PortFuture, PrincipalAccount, SecretFactory},
    };
    use agent_room_domain::{
        agent_cards::{
            AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardExtension,
            AgentCardProtocolVersion, AgentCardProvider, AgentCardSecurityScheme,
            AgentCardSecuritySchemeKind, AgentCardSkill, AgentCardSnapshot,
            AgentCardSnapshotFields, AgentCardSourceUrl, AgentCardTransport,
            AgentCardVerificationState, AgentEndpointVerificationState, NormalizedAgentCard,
            NormalizedAgentCardFields,
        },
        devices::Device,
        identity::Principal,
        ids::{AgentCardSnapshotId, AgentId, DeviceId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::{AgentCardHttpDependencies, AgentCardHttpState, router};

    const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
    const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
    const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const SNAPSHOT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const SOURCE_URL: &str = "https://agent.example/.well-known/agent-card.json";

    struct FakeCards {
        request: Mutex<Option<RefreshAgentCard>>,
        refresh: AgentCardRefresh,
    }

    impl FakeCards {
        fn new() -> Self {
            Self {
                request: Mutex::new(None),
                refresh: refresh_result(),
            }
        }
    }

    impl AgentCardUseCases for FakeCards {
        fn refresh(
            &self,
            request: RefreshAgentCard,
        ) -> PortFuture<'_, AgentCardManagementResult<AgentCardRefresh>> {
            *self.request.lock().expect("Agent Card 请求记录锁可用") = Some(request);
            let refresh = self.refresh.clone();
            Box::pin(async move { Ok(refresh) })
        }
    }

    #[derive(Default)]
    struct FakeDevices {
        authentications: AtomicUsize,
        expected_body: Mutex<Option<String>>,
    }

    impl DeviceAuthorizationUseCases for FakeDevices {
        fn register_device(
            &self,
            _request: RegisterDevice,
        ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("Agent Card 路由不会注册设备") })
        }

        fn authenticate_device<'a>(
            &'a self,
            request: AuthenticateDeviceRequest<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
            self.authentications.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.access_token.expose(), "device-access-token");
            assert_eq!(request.proof.device_id(), device_id());
            assert_eq!(request.proof.issued_at(), time(1_700_000_000_000));
            assert_eq!(request.proof.nonce().expose(), "nonce-0123456789abcdef");
            assert_eq!(request.proof.method(), "POST");
            assert_eq!(
                request.proof.request_target(),
                format!("/agents/{AGENT_UUID}/agent-card/refresh")
            );
            let expected_body = self
                .expected_body
                .lock()
                .expect("设备请求正文记录锁可用")
                .take()
                .expect("测试必须登记原始请求正文");
            assert_eq!(
                request.proof.body_digest(),
                &SecureSecretFactory.digest(&expected_body)
            );
            Box::pin(async { Ok(authenticated_device()) })
        }

        fn refresh_device_session<'a>(
            &'a self,
            _request: RefreshDeviceSession<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("Agent Card 路由不会刷新设备会话") })
        }

        fn list_devices(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
            Box::pin(async { unreachable!("Agent Card 路由不会列出设备") })
        }

        fn revoke_device(
            &self,
            _principal_id: PrincipalId,
            _device_id: DeviceId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<()>> {
            Box::pin(async { unreachable!("Agent Card 路由不会撤销设备") })
        }
    }

    #[tokio::test]
    async fn 刷新端点按原始正文认证并只返回安全投影() {
        let cards = Arc::new(FakeCards::new());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({ "sourceUrl": SOURCE_URL }).to_string();
        *devices
            .expected_body
            .lock()
            .expect("设备请求正文记录锁可用") = Some(body.clone());

        let response = test_router(cards.clone(), devices.clone())
            .oneshot(refresh_request(&body, true))
            .await
            .expect("Agent Card 刷新路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let payload = response_json(response).await;
        assert_eq!(payload["agentId"], AGENT_UUID);
        assert_eq!(payload["sourceUrl"], SOURCE_URL);
        assert_eq!(payload["verification"], "verified");
        assert_eq!(payload["change"], "initial");
        assert_eq!(payload["card"]["endpoints"][0]["protocolVersion"], "1.0");
        assert_eq!(
            payload["card"]["securitySchemes"][0],
            json!({ "name": "oauth", "kind": "oauth2" })
        );
        let serialized = payload.to_string();
        assert!(!serialized.contains("signatures"));
        assert!(!serialized.contains("examples"));
        assert!(!serialized.contains("clientSecret"));
        assert!(!serialized.contains("digest"));
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);

        let request = cards
            .request
            .lock()
            .expect("Agent Card 请求记录锁可用")
            .clone()
            .expect("Agent Card 刷新用例已调用");
        assert_eq!(request.agent_id, agent_id());
        assert_eq!(request.source_url.as_str(), SOURCE_URL);
        assert_eq!(request.actor.device_id, device_id());
    }

    #[tokio::test]
    async fn 非_https_来源在设备认证后且抓取用例前失败() {
        let cards = Arc::new(FakeCards::new());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({ "sourceUrl": "http://127.0.0.1/agent-card.json" }).to_string();
        *devices
            .expected_body
            .lock()
            .expect("设备请求正文记录锁可用") = Some(body.clone());

        let response = test_router(cards.clone(), devices.clone())
            .oneshot(refresh_request(&body, true))
            .await
            .expect("非法来源请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
        assert!(
            cards
                .request
                .lock()
                .expect("Agent Card 请求记录锁可用")
                .is_none()
        );
    }

    #[tokio::test]
    async fn 缺失设备证明时不会触碰认证或刷新用例() {
        let cards = Arc::new(FakeCards::new());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({ "sourceUrl": SOURCE_URL }).to_string();

        let response = test_router(cards.clone(), devices.clone())
            .oneshot(refresh_request(&body, false))
            .await
            .expect("缺失设备证明请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 0);
        assert!(
            cards
                .request
                .lock()
                .expect("Agent Card 请求记录锁可用")
                .is_none()
        );
    }

    fn test_router(cards: Arc<FakeCards>, devices: Arc<FakeDevices>) -> axum::Router {
        let state = AgentCardHttpState::new(AgentCardHttpDependencies {
            cards,
            devices,
            secrets: Arc::new(SecureSecretFactory),
        });
        router(state).layer(middleware::from_fn(crate::correlation::attach))
    }

    fn refresh_request(body: &str, include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/agents/{AGENT_UUID}/agent-card/refresh"))
            .header(header::AUTHORIZATION, "Bearer device-access-token")
            .header(header::CONTENT_TYPE, "application/json");
        if include_proof {
            request = request
                .header("x-agent-room-device-id", DEVICE_UUID)
                .header("x-agent-room-proof-issued-at", "1700000000000")
                .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
                .header(
                    "x-agent-room-proof-signature",
                    URL_SAFE_NO_PAD.encode([9_u8; 64]),
                );
        }
        request
            .body(Body::from(body.to_owned()))
            .expect("Agent Card 刷新请求有效")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    fn refresh_result() -> AgentCardRefresh {
        AgentCardRefresh {
            snapshot: AgentCardSnapshot::new(AgentCardSnapshotFields {
                id: AgentCardSnapshotId::from_uuid(uuid(SNAPSHOT_UUID)),
                agent_id: agent_id(),
                source_url: AgentCardSourceUrl::new(SOURCE_URL.to_owned()).expect("测试来源有效"),
                digest: AgentCardDigest::from_array([7_u8; 32]),
                card: normalized_card(),
                verification: AgentCardVerificationState::Verified,
                fetched_at: time(1_700_000_000_000),
                expires_at: time(1_700_000_300_000),
            })
            .expect("测试 Agent Card 快照有效"),
            change: agent_room_application::agent_cards::AgentCardChange::Initial,
        }
    }

    fn normalized_card() -> NormalizedAgentCard {
        let extension_uri = "urn:agent-room:public-messaging".to_owned();
        let supported_extensions = BTreeSet::from([extension_uri.clone()]);
        NormalizedAgentCard::new(NormalizedAgentCardFields {
            name: "研究助手".to_owned(),
            description: "公开能力资料".to_owned(),
            provider: Some(
                AgentCardProvider::new("Agent Room".to_owned(), "https://agent.example".to_owned())
                    .expect("测试提供方有效"),
            ),
            version: "1.2.0".to_owned(),
            endpoints: vec![
                AgentCardEndpoint::new(
                    "https://agent.example/a2a".to_owned(),
                    AgentCardTransport::HttpJson,
                    AgentCardProtocolVersion::V1_0,
                    Some("public".to_owned()),
                    AgentEndpointVerificationState::Verified,
                )
                .expect("测试端点有效"),
            ],
            capabilities: AgentCardCapabilities::new(
                true,
                false,
                false,
                vec![
                    AgentCardExtension::new(extension_uri, "公开消息".to_owned(), true)
                        .expect("测试扩展有效"),
                ],
                &supported_extensions,
            )
            .expect("测试能力有效"),
            security_schemes: vec![
                AgentCardSecurityScheme::new(
                    "oauth".to_owned(),
                    AgentCardSecuritySchemeKind::OAuth2,
                )
                .expect("测试认证摘要有效"),
            ],
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills: vec![
                AgentCardSkill::new(
                    "research".to_owned(),
                    "研究".to_owned(),
                    "整理公开信息".to_owned(),
                    vec!["research".to_owned()],
                    vec!["text/plain".to_owned()],
                    vec!["text/plain".to_owned()],
                )
                .expect("测试技能有效"),
            ],
        })
        .expect("测试规范化 Agent Card 有效")
    }

    fn authenticated_device() -> AuthenticatedDevice {
        AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id()),
                matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                display_name: "Agent Room User".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: device_id(),
            access_token_expires_at: time(1_700_000_900_000),
        }
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(uuid(DEVICE_UUID))
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(uuid(AGENT_UUID))
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
