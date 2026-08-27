use std::sync::Arc;

use agent_room_application::{
    agent_lobbies::{AgentLobbyEntryUseCases, EnterAgentLobby},
    devices::DeviceAuthorizationUseCases,
    persistence::RepositoryErrorKind,
    ports::{RoomDirectory, RoomDirectoryQuery, SecretFactory},
    rooms::{EnterLobbyOutcome, LobbyJoinKind},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    rooms::{RoomLanguage, RoomRegion},
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
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

const MAX_LOBBY_ENTRY_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct LobbyHttpState {
    entries: Arc<dyn AgentLobbyEntryUseCases>,
    directory: Arc<dyn RoomDirectory>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
}

pub(crate) struct LobbyHttpDependencies {
    pub(crate) entries: Arc<dyn AgentLobbyEntryUseCases>,
    pub(crate) directory: Arc<dyn RoomDirectory>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl LobbyHttpState {
    pub(crate) fn new(dependencies: LobbyHttpDependencies) -> Self {
        Self {
            entries: dependencies.entries,
            directory: dependencies.directory,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
        }
    }
}

pub(crate) fn router(state: LobbyHttpState) -> Router {
    Router::new()
        .route("/lobbies/public", get(list_public_lobbies))
        .route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            post(enter_lobby),
        )
        .layer(DefaultBodyLimit::max(MAX_LOBBY_ENTRY_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnterLobbyBody {
    #[serde(default)]
    preferred_language: Option<String>,
    #[serde(default)]
    preferred_region: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum LobbyEntryResponse {
    Joined {
        catalog_id: String,
        room_instance_id: String,
        matrix_room_id: String,
        reservation_id: String,
        assignment: &'static str,
    },
    ProvisioningBusy {
        retry_at_unix_ms: i64,
    },
    CapacityChanged {
        catalog_id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicLobbyDirectoryResponse {
    lobbies: Vec<PublicLobbyResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicLobbyResponse {
    catalog_id: String,
    slug: Option<String>,
    name: String,
    description: String,
    language: Option<String>,
    active_instance_count: u16,
    online_agent_count: u32,
}

async fn list_public_lobbies(
    State(state): State<LobbyHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
) -> Response {
    match state
        .directory
        .list_public(&RoomDirectoryQuery::default())
        .await
    {
        Ok(entries) => no_store(
            Json(PublicLobbyDirectoryResponse {
                lobbies: entries
                    .into_iter()
                    .map(|entry| PublicLobbyResponse {
                        catalog_id: entry.catalog.id().to_string(),
                        slug: entry.catalog.slug().map(|slug| slug.as_str().to_owned()),
                        name: entry.catalog.name().to_owned(),
                        description: entry.catalog.description().to_owned(),
                        language: entry
                            .catalog
                            .language()
                            .map(|language| language.as_str().to_owned()),
                        active_instance_count: entry.active_instance_count,
                        online_agent_count: entry.online_agent_count,
                    })
                    .collect(),
            })
            .into_response(),
        ),
        Err(error) => {
            let (status, code, category) = match error.kind() {
                RepositoryErrorKind::Unavailable => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "lobby.directory_unavailable",
                    ErrorCategory::DependencyUnavailable,
                ),
                RepositoryErrorKind::Forbidden
                | RepositoryErrorKind::NotFound
                | RepositoryErrorKind::Conflict
                | RepositoryErrorKind::Constraint
                | RepositoryErrorKind::CorruptData => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "lobby.directory_internal",
                    ErrorCategory::Transient,
                ),
            };
            tracing::warn!(
                correlation.id = %correlation_id.as_uuid(),
                operation = error.operation(),
                failure = ?error.kind(),
                "公开大厅目录读取失败"
            );
            no_store(
                ApiError::new(
                    status,
                    code,
                    category,
                    "公开大厅目录暂时不可用。",
                    correlation_id,
                )
                .into_response(),
            )
        }
    }
}

async fn enter_lobby(
    State(state): State<LobbyHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent_id, instance_id, catalog_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target =
        format!("/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry");
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(invalid_body(correlation_id).into_response());
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
    let Ok(body) = serde_json::from_slice::<EnterLobbyBody>(&body) else {
        return no_store(invalid_body(correlation_id).into_response());
    };
    let Ok(preferred_language) = body.preferred_language.map(RoomLanguage::new).transpose() else {
        return no_store(invalid_body(correlation_id).into_response());
    };
    let Ok(preferred_region) = body.preferred_region.map(RoomRegion::new).transpose() else {
        return no_store(invalid_body(correlation_id).into_response());
    };
    match state
        .entries
        .enter(EnterAgentLobby {
            actor,
            agent_id,
            agent_instance_id: instance_id,
            catalog_id,
            preferred_language,
            preferred_region,
        })
        .await
    {
        Ok(outcome) => lobby_outcome(outcome),
        Err(failure) => no_store(ApiError::agent_lobby(&failure, correlation_id).into_response()),
    }
}

fn lobby_outcome(outcome: EnterLobbyOutcome) -> Response {
    let (status, body) = match outcome {
        EnterLobbyOutcome::Joined {
            reservation,
            room,
            kind,
        } => (
            StatusCode::OK,
            LobbyEntryResponse::Joined {
                catalog_id: room.catalog_id().to_string(),
                room_instance_id: room.id().to_string(),
                matrix_room_id: room.matrix_room_id().as_str().to_owned(),
                reservation_id: reservation.id().to_string(),
                assignment: match kind {
                    LobbyJoinKind::NewAssignment => "new",
                    LobbyJoinKind::RecoveredAssignment => "recovered",
                },
            },
        ),
        EnterLobbyOutcome::ProvisioningBusy { retry_at } => (
            StatusCode::ACCEPTED,
            LobbyEntryResponse::ProvisioningBusy {
                retry_at_unix_ms: retry_at.value(),
            },
        ),
        EnterLobbyOutcome::CapacityChanged { catalog_id } => (
            StatusCode::ACCEPTED,
            LobbyEntryResponse::CapacityChanged {
                catalog_id: catalog_id.to_string(),
            },
        ),
    };
    no_store((status, Json(body)).into_response())
}

fn invalid_resource_id(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("lobby.invalid_resource_id", correlation_id)
}

fn invalid_body(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("lobby.invalid_entry_body", correlation_id)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use agent_room_application::{
        agent_lobbies::{AgentLobbyEntryResult, AgentLobbyEntryUseCases, EnterAgentLobby},
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
            RevokedDevice,
        },
        persistence::RepositoryResult,
        ports::{
            PortFuture, PrincipalAccount, PublicLobbyDirectoryEntry, RoomDirectory,
            RoomDirectoryQuery, SecretFactory,
        },
        rooms::{EnterLobbyOutcome, LobbyJoinKind},
    };
    use agent_room_domain::{
        devices::Device,
        identity::Principal,
        ids::{
            AgentId, AgentInstanceId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId,
            RoomReservationId,
        },
        rooms::{
            MatrixRoomReference, RoomCapacity, RoomCatalog, RoomInstance, RoomInstanceFields,
            RoomInstanceState, RoomReservation,
        },
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

    use super::{LobbyHttpDependencies, LobbyHttpState, router};

    const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
    const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
    const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const ROOM_INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const RESERVATION_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";

    struct FakeEntries {
        request: Mutex<Option<EnterAgentLobby>>,
        outcome: Mutex<Option<AgentLobbyEntryResult<EnterLobbyOutcome>>>,
    }

    struct FakeDirectory;

    impl RoomDirectory for FakeDirectory {
        fn list_public<'a>(
            &'a self,
            _query: &'a RoomDirectoryQuery,
        ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn find_catalog(
            &self,
            _catalog_id: RoomCatalogId,
        ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
            Box::pin(async { Ok(None) })
        }
    }

    impl FakeEntries {
        fn returning(outcome: EnterLobbyOutcome) -> Self {
            Self {
                request: Mutex::new(None),
                outcome: Mutex::new(Some(Ok(outcome))),
            }
        }
    }

    impl AgentLobbyEntryUseCases for FakeEntries {
        fn enter(
            &self,
            request: EnterAgentLobby,
        ) -> PortFuture<'_, AgentLobbyEntryResult<EnterLobbyOutcome>> {
            *self.request.lock().expect("大厅请求记录锁可用") = Some(request);
            let outcome = self
                .outcome
                .lock()
                .expect("大厅结果记录锁可用")
                .take()
                .expect("每个测试只能调用一次大厅用例");
            Box::pin(async move { outcome })
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
            Box::pin(async { unreachable!("大厅路由不会注册设备") })
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
            assert_eq!(request.proof.request_target(), request_target());
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
            Box::pin(async { unreachable!("大厅路由不会刷新设备会话") })
        }

        fn list_devices(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
            Box::pin(async { unreachable!("大厅路由不会列出设备") })
        }

        fn revoke_device(
            &self,
            _principal_id: PrincipalId,
            _device_id: DeviceId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
            Box::pin(async { unreachable!("大厅路由不会撤销设备") })
        }
    }

    #[tokio::test]
    async fn 签名端点转发权威设备与偏好并返回加入投影() {
        let entries = Arc::new(FakeEntries::returning(joined_outcome()));
        let devices = Arc::new(FakeDevices::default());
        let body = json!({
            "preferredLanguage": "zh-CN",
            "preferredRegion": "ap-southeast"
        })
        .to_string();
        *devices
            .expected_body
            .lock()
            .expect("设备请求正文记录锁可用") = Some(body.clone());

        let response = test_router(entries.clone(), devices.clone())
            .oneshot(entry_request(&body, true))
            .await
            .expect("大厅加入路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let payload = response_json(response).await;
        assert_eq!(payload["status"], "joined");
        assert_eq!(payload["catalogId"], CATALOG_UUID);
        assert_eq!(payload["roomInstanceId"], ROOM_INSTANCE_UUID);
        assert_eq!(
            payload["matrixRoomId"],
            "!public-lobby:matrix.agent-room.test"
        );
        assert_eq!(payload["reservationId"], RESERVATION_UUID);
        assert_eq!(payload["assignment"], "new");
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);

        let request = entries
            .request
            .lock()
            .expect("大厅请求记录锁可用")
            .clone()
            .expect("大厅用例已调用");
        assert_eq!(request.actor.device_id, device_id());
        assert_eq!(request.agent_id, agent_id());
        assert_eq!(request.agent_instance_id, agent_instance_id());
        assert_eq!(request.catalog_id, catalog_id());
        assert_eq!(
            request
                .preferred_language
                .as_ref()
                .expect("语言偏好存在")
                .as_str(),
            "zh-CN"
        );
        assert_eq!(
            request
                .preferred_region
                .as_ref()
                .expect("地区偏好存在")
                .as_str(),
            "ap-southeast"
        );
    }

    #[tokio::test]
    async fn 公开大厅目录不伪造缺失数据() {
        let response = test_router(
            Arc::new(FakeEntries::returning(joined_outcome())),
            Arc::new(FakeDevices::default()),
        )
        .oneshot(
            Request::builder()
                .uri("/lobbies/public")
                .body(Body::empty())
                .expect("公开大厅目录请求有效"),
        )
        .await
        .expect("公开大厅目录路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, json!({ "lobbies": [] }));
    }

    #[tokio::test]
    async fn 供给繁忙返回可重试的二零二而不是伪造房间() {
        let entries = Arc::new(FakeEntries::returning(
            EnterLobbyOutcome::ProvisioningBusy {
                retry_at: time(1_700_000_030_000),
            },
        ));
        let devices = Arc::new(FakeDevices::default());
        let body = "{}".to_owned();
        *devices
            .expected_body
            .lock()
            .expect("设备请求正文记录锁可用") = Some(body.clone());

        let response = test_router(entries, devices)
            .oneshot(entry_request(&body, true))
            .await
            .expect("大厅繁忙响应可调用");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            response_json(response).await,
            json!({
                "status": "provisioning_busy",
                "retryAtUnixMs": 1_700_000_030_000_i64
            })
        );
    }

    #[tokio::test]
    async fn 缺失设备证明时不会触碰设备认证或大厅用例() {
        let entries = Arc::new(FakeEntries::returning(joined_outcome()));
        let devices = Arc::new(FakeDevices::default());

        let response = test_router(entries.clone(), devices.clone())
            .oneshot(entry_request("{}", false))
            .await
            .expect("缺失设备证明请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 0);
        assert!(
            entries
                .request
                .lock()
                .expect("大厅请求记录锁可用")
                .is_none()
        );
    }

    fn test_router(entries: Arc<FakeEntries>, devices: Arc<FakeDevices>) -> axum::Router {
        let state = LobbyHttpState::new(LobbyHttpDependencies {
            entries,
            directory: Arc::new(FakeDirectory),
            devices,
            secrets: Arc::new(SecureSecretFactory),
        });
        router(state).layer(middleware::from_fn(crate::correlation::attach))
    }

    fn entry_request(body: &str, include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(request_target())
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
            .expect("大厅加入请求有效")
    }

    fn request_target() -> String {
        format!("/agents/{AGENT_UUID}/instances/{INSTANCE_UUID}/lobbies/{CATALOG_UUID}/entry")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    fn joined_outcome() -> EnterLobbyOutcome {
        let room = RoomInstance::restore(
            RoomInstanceId::from_uuid(uuid(ROOM_INSTANCE_UUID)),
            RoomInstanceFields {
                catalog_id: catalog_id(),
                matrix_room_id: MatrixRoomReference::new(
                    "!public-lobby:matrix.agent-room.test".to_owned(),
                )
                .expect("测试 Matrix 房间有效"),
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: 1,
                allocated_slots: 1,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .expect("测试房间实例有效");
        let reservation = RoomReservation::reserve(
            RoomReservationId::from_uuid(uuid(RESERVATION_UUID)),
            catalog_id(),
            room.id(),
            agent_instance_id(),
            time(1_700_000_000_000),
            time(1_700_000_060_000),
        )
        .expect("测试大厅预约有效");
        EnterLobbyOutcome::Joined {
            reservation,
            room,
            kind: LobbyJoinKind::NewAssignment,
        }
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

    fn agent_instance_id() -> AgentInstanceId {
        AgentInstanceId::from_uuid(uuid(INSTANCE_UUID))
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(uuid(CATALOG_UUID))
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
