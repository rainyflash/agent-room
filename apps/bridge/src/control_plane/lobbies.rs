use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::{
    lobby_session::{
        AgentLobbyEntryIntent, ControlPlaneLobbyEntryFailure, ControlPlaneLobbyEntryFailureKind,
        ControlPlaneLobbyEntryGateway, ControlPlaneLobbyEntryOutcome, ControlPlaneLobbyEntryResult,
        JoinedAgentLobby, LobbyAssignmentKind,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomLanguage, RoomRegion},
    time::UtcMillis,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

pub struct ReqwestControlPlaneLobbyEntryGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneLobbyEntryGateway {
    /// 创建只向固定控制面提交自动大厅分配的 HTTP 网关。
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

    async fn enter_internal(
        &self,
        intent: &AgentLobbyEntryIntent,
    ) -> ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome> {
        let request_target = format!(
            "/agents/{}/instances/{}/lobbies/{}/entry",
            intent.agent_id(),
            intent.agent_instance_id(),
            intent.catalog_id()
        );
        let body = serde_json::to_string(&EnterLobbyBody {
            preferred_language: intent.preferred_language().map(RoomLanguage::as_str),
            preferred_region: intent.preferred_region().map(RoomRegion::as_str),
        })
        .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::Internal))?;
        let authorized = self
            .authorizer
            .authorize("POST", &request_target, &body)
            .await
            .map_err(map_session_failure)?;
        let request_url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::Internal))?;
        let request = signed_request_headers(
            self.client
                .post(request_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            &request_target,
        )
        .map_err(|()| failure(ControlPlaneLobbyEntryFailureKind::Internal))?;
        let response = request
            .body(body)
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        decode_response(response).await
    }
}

impl ControlPlaneLobbyEntryGateway for ReqwestControlPlaneLobbyEntryGateway {
    fn enter<'a>(
        &'a self,
        intent: &'a AgentLobbyEntryIntent,
    ) -> PortFuture<'a, ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>> {
        Box::pin(self.enter_internal(intent))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnterLobbyBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_language: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_region: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum LobbyEntryResponse {
    Joined {
        catalog_id: String,
        room_instance_id: String,
        matrix_room_id: String,
        reservation_id: String,
        assignment: String,
    },
    ProvisioningBusy {
        retry_at_unix_ms: i64,
    },
    CapacityChanged {
        catalog_id: String,
    },
}

#[derive(Deserialize)]
struct ErrorCategoryResponse {
    category: String,
}

async fn decode_response(
    response: reqwest::Response,
) -> ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome> {
    let status = response.status();
    let body = read_limited_response_body(response)
        .await
        .map_err(|()| response_body_failure(status))?;
    if !status.is_success() {
        let category = serde_json::from_slice::<ErrorCategoryResponse>(&body)
            .ok()
            .map(|envelope| envelope.category);
        return Err(status_failure(status, category.as_deref()));
    }
    let response = serde_json::from_slice::<LobbyEntryResponse>(&body)
        .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::UnknownCommit))?;
    response.try_into()
}

impl TryFrom<LobbyEntryResponse> for ControlPlaneLobbyEntryOutcome {
    type Error = ControlPlaneLobbyEntryFailure;

    fn try_from(value: LobbyEntryResponse) -> Result<Self, Self::Error> {
        match value {
            LobbyEntryResponse::Joined {
                catalog_id,
                room_instance_id,
                matrix_room_id,
                reservation_id,
                assignment,
            } => Ok(Self::Joined(JoinedAgentLobby::new(
                parse_v7(&catalog_id).map(RoomCatalogId::from_uuid)?,
                parse_v7(&room_instance_id).map(RoomInstanceId::from_uuid)?,
                MatrixRoomReference::new(matrix_room_id)
                    .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::InvalidResponse))?,
                parse_v7(&reservation_id).map(RoomReservationId::from_uuid)?,
                parse_assignment(&assignment)?,
            ))),
            LobbyEntryResponse::ProvisioningBusy { retry_at_unix_ms } => {
                Ok(Self::ProvisioningBusy {
                    retry_at: UtcMillis::new(retry_at_unix_ms)
                        .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::InvalidResponse))?,
                })
            }
            LobbyEntryResponse::CapacityChanged { catalog_id } => Ok(Self::CapacityChanged {
                catalog_id: parse_v7(&catalog_id).map(RoomCatalogId::from_uuid)?,
            }),
        }
    }
}

fn parse_v7(value: &str) -> ControlPlaneLobbyEntryResult<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value)
        .map_err(|_| failure(ControlPlaneLobbyEntryFailureKind::InvalidResponse))?;
    if id.get_version() != Some(uuid::Version::SortRand) {
        return Err(failure(ControlPlaneLobbyEntryFailureKind::InvalidResponse));
    }
    Ok(id)
}

fn parse_assignment(value: &str) -> ControlPlaneLobbyEntryResult<LobbyAssignmentKind> {
    match value {
        "new" => Ok(LobbyAssignmentKind::New),
        "recovered" => Ok(LobbyAssignmentKind::Recovered),
        _ => Err(failure(ControlPlaneLobbyEntryFailureKind::InvalidResponse)),
    }
}

fn transport_failure(error: &reqwest::Error) -> ControlPlaneLobbyEntryFailure {
    if error.is_connect() {
        failure(ControlPlaneLobbyEntryFailureKind::Unavailable)
    } else {
        failure(ControlPlaneLobbyEntryFailureKind::UnknownCommit)
    }
}

fn response_body_failure(status: StatusCode) -> ControlPlaneLobbyEntryFailure {
    if status.is_success() {
        failure(ControlPlaneLobbyEntryFailureKind::UnknownCommit)
    } else {
        status_failure(status, None)
    }
}

fn status_failure(status: StatusCode, category: Option<&str>) -> ControlPlaneLobbyEntryFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            ControlPlaneLobbyEntryFailureKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED => ControlPlaneLobbyEntryFailureKind::AuthenticationRejected,
        StatusCode::FORBIDDEN => ControlPlaneLobbyEntryFailureKind::Forbidden,
        StatusCode::NOT_FOUND => ControlPlaneLobbyEntryFailureKind::NotFound,
        StatusCode::CONFLICT => ControlPlaneLobbyEntryFailureKind::Conflict,
        StatusCode::SERVICE_UNAVAILABLE if category == Some("unknown_commit") => {
            ControlPlaneLobbyEntryFailureKind::UnknownCommit
        }
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => ControlPlaneLobbyEntryFailureKind::Unavailable,
        _ if status.is_server_error() => ControlPlaneLobbyEntryFailureKind::Unavailable,
        _ => ControlPlaneLobbyEntryFailureKind::InvalidResponse,
    };
    failure(kind)
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> ControlPlaneLobbyEntryFailure {
    let kind = match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => {
            ControlPlaneLobbyEntryFailureKind::AuthenticationRejected
        }
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            ControlPlaneLobbyEntryFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => ControlPlaneLobbyEntryFailureKind::Internal,
    };
    failure(kind)
}

const fn failure(kind: ControlPlaneLobbyEntryFailureKind) -> ControlPlaneLobbyEntryFailure {
    ControlPlaneLobbyEntryFailure::new(kind)
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
        agent_identity::BridgeAgentIdentity,
        lobby_session::{
            AgentLobbySessionConfig, AgentLobbySessionFailureKind, AgentLobbySessionService,
            ControlPlaneLobbyEntryOutcome,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{
        ids::{AgentId, AgentInstanceId, DeviceId, RoomCatalogId},
        rooms::{RoomLanguage, RoomRegion},
        time::UtcMillis,
    };
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

    use super::{ControlPlaneHttpConfig, ReqwestControlPlaneLobbyEntryGateway};

    const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const ROOM_INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const RESERVATION_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";

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
    async fn 大厅网关以同一正文签名并解析权威房间() {
        let app = Router::new().route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            post(
                |Path((agent_id, instance_id, catalog_id)): Path<(String, String, String)>,
                 headers: HeaderMap,
                 body: String| async move {
                    let valid = agent_id == AGENT_ID
                        && instance_id == INSTANCE_ID
                        && catalog_id == CATALOG_ID
                        && header(&headers, "authorization") == Some("Bearer access-token")
                        && header(&headers, "x-agent-room-device-id")
                            == Some("00000000-0000-0000-0000-000000000001")
                        && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                        && serde_json::from_str::<Value>(&body).ok()
                            == Some(json!({
                                "preferredLanguage": "zh-CN",
                                "preferredRegion": "ap-southeast"
                            }));
                    if valid {
                        (StatusCode::OK, Json(joined_response())).into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                },
            ),
        );
        let authorizer = Arc::new(测试请求授权器::default());
        let service = service(spawn_server(app).await, authorizer.clone());

        let outcome = service
            .enter(&identity(), &config())
            .await
            .expect("规范大厅响应可解析");

        let ControlPlaneLobbyEntryOutcome::Joined(room) = outcome else {
            panic!("应返回已加入大厅");
        };
        assert_eq!(room.catalog_id(), catalog_id());
        assert_eq!(room.matrix_room_id().as_str(), "!public:example.org");
        let requests = authorizer.requests.lock().expect("授权请求记录锁可用");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "POST");
        assert_eq!(
            requests[0].1,
            format!("/agents/{AGENT_ID}/instances/{INSTANCE_ID}/lobbies/{CATALOG_ID}/entry")
        );
    }

    #[tokio::test]
    async fn 大厅网关保留供给重试时间与未知提交类别() {
        let busy = Router::new().route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            post(|| async {
                (
                    StatusCode::ACCEPTED,
                    Json(json!({
                        "status": "provisioning_busy",
                        "retryAtUnixMs": 1_700_000_030_000_i64
                    })),
                )
            }),
        );
        let unknown = Router::new().route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "category": "unknown_commit",
                        "code": "lobby.unknown_commit"
                    })),
                )
            }),
        );

        let busy_outcome = service(
            spawn_server(busy).await,
            Arc::new(测试请求授权器::default()),
        )
        .enter(&identity(), &config())
        .await
        .expect("供给繁忙是可重试结果");
        assert_eq!(
            busy_outcome,
            ControlPlaneLobbyEntryOutcome::ProvisioningBusy {
                retry_at: UtcMillis::new(1_700_000_030_000).expect("测试时间有效")
            }
        );

        let failure = service(
            spawn_server(unknown).await,
            Arc::new(测试请求授权器::default()),
        )
        .enter(&identity(), &config())
        .await
        .expect_err("未知提交不能降级为普通断线");
        assert_eq!(
            failure.kind(),
            AgentLobbySessionFailureKind::EntryOutcomeUnknown
        );
    }

    fn service(
        base_url: String, authorizer: Arc<测试请求授权器>
    ) -> AgentLobbySessionService {
        let gateway = ReqwestControlPlaneLobbyEntryGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("本地大厅网关地址有效");
        AgentLobbySessionService::new(Arc::new(gateway))
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

    fn config() -> AgentLobbySessionConfig {
        AgentLobbySessionConfig::new(
            catalog_id(),
            Some(RoomLanguage::new("zh-CN".to_owned()).expect("测试语言有效")),
            Some(RoomRegion::new("ap-southeast".to_owned()).expect("测试地区有效")),
        )
    }

    fn identity() -> BridgeAgentIdentity {
        BridgeAgentIdentity::new(
            AgentId::from_uuid(uuid(AGENT_ID)),
            "Codex Builder",
            "@agent:example.org",
            AgentInstanceId::from_uuid(uuid(INSTANCE_ID)),
        )
        .expect("测试 Agent 身份有效")
    }

    fn joined_response() -> Value {
        json!({
            "status": "joined",
            "catalogId": CATALOG_ID,
            "roomInstanceId": ROOM_INSTANCE_ID,
            "matrixRoomId": "!public:example.org",
            "reservationId": RESERVATION_ID,
            "assignment": "new"
        })
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(uuid(CATALOG_ID))
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("测试设备标识有效"),
        )
    }

    fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    fn secret(value: &str) -> SecretValue {
        SecretValue::new(value).expect("测试秘密有效")
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }
}
