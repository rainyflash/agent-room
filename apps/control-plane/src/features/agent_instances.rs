use std::sync::Arc;

use agent_room_application::{
    agent_instance_management::{
        AgentInstanceCleanupFailureKind, AgentInstanceManagementUseCases,
        AgentInstanceMatrixCleanup, ListAgentInstances, RevokeAgentInstance, RevokedAgentInstance,
    },
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::AgentInstanceManagementRecord,
};
use agent_room_domain::{ids::AgentInstanceId, time::UtcMillis};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

#[derive(Clone)]
pub(crate) struct AgentInstanceHttpState {
    instances: Arc<dyn AgentInstanceManagementUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    frontend_origin: String,
}

pub(crate) struct AgentInstanceHttpStateDependencies {
    pub(crate) instances: Arc<dyn AgentInstanceManagementUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
}

impl AgentInstanceHttpState {
    pub(crate) fn new(
        dependencies: AgentInstanceHttpStateDependencies,
        frontend_origin: &url::Url,
    ) -> Self {
        Self {
            instances: dependencies.instances,
            authentication: dependencies.authentication,
            frontend_origin: frontend_origin.origin().ascii_serialization(),
        }
    }
}

pub(crate) fn router(state: AgentInstanceHttpState) -> Router {
    Router::new()
        .route("/agent-instances", get(list_instances))
        .route(
            "/agent-instances/{instance_id}",
            axum::routing::delete(revoke_instance),
        )
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceListResponse {
    instances: Vec<AgentInstanceSummaryResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceSummaryResponse {
    agent_instance_id: String,
    agent_id: String,
    agent_display_name: String,
    agent_avatar_content_id: Option<String>,
    status: &'static str,
    adapter_type: String,
    capability_version: String,
    matrix_device_id: String,
    device: AgentInstanceDeviceResponse,
    created_at_unix_ms: i64,
    last_seen_at_unix_ms: Option<i64>,
    revoked_at_unix_ms: Option<i64>,
    matrix_device_revoked_at_unix_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceDeviceResponse {
    device_id: String,
    label: String,
    platform: &'static str,
    trust_state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceRevocationResponse {
    instance: AgentInstanceSummaryResponse,
    matrix_cleanup: &'static str,
    matrix_cleanup_pending_reason: Option<&'static str>,
}

async fn list_instances(
    State(state): State<AgentInstanceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .instances
        .list_instances(ListAgentInstances { actor })
        .await
    {
        Ok(instances) => no_store(
            Json(AgentInstanceListResponse {
                instances: instances
                    .into_iter()
                    .map(AgentInstanceSummaryResponse::from)
                    .collect(),
            })
            .into_response(),
        ),
        Err(failure) => {
            no_store(ApiError::agent_instance_management(failure, correlation_id).into_response())
        }
    }
}

async fn revoke_instance(
    State(state): State<AgentInstanceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(instance_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.frontend_origin) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(
            ApiError::invalid_request("agent_instance.invalid_id", correlation_id).into_response(),
        );
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .instances
        .revoke_instance(RevokeAgentInstance { actor, instance_id })
        .await
    {
        Ok(revoked) => revocation_response(revoked),
        Err(failure) => {
            no_store(ApiError::agent_instance_management(failure, correlation_id).into_response())
        }
    }
}

fn revocation_response(revoked: RevokedAgentInstance) -> Response {
    let status = match revoked.matrix_cleanup {
        AgentInstanceMatrixCleanup::Complete => StatusCode::OK,
        AgentInstanceMatrixCleanup::Pending { .. } => StatusCode::ACCEPTED,
    };
    no_store((status, Json(AgentInstanceRevocationResponse::from(revoked))).into_response())
}

fn invalid_origin(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "agent_instance.invalid_origin",
        ErrorCategory::Authorization,
        "Agent 实例撤销请求来源无效。",
        correlation_id,
    )
}

impl From<AgentInstanceManagementRecord> for AgentInstanceSummaryResponse {
    fn from(value: AgentInstanceManagementRecord) -> Self {
        Self {
            agent_instance_id: value.instance.id().to_string(),
            agent_id: value.instance.agent_id().to_string(),
            agent_display_name: value.agent_display_name,
            agent_avatar_content_id: value.agent_avatar_content_id.map(|id| id.to_string()),
            status: value.instance.status().as_str(),
            adapter_type: value.adapter_type,
            capability_version: value.capability_version,
            matrix_device_id: value.instance.matrix_device_id().as_str().to_owned(),
            device: AgentInstanceDeviceResponse {
                device_id: value.instance.device_id().to_string(),
                label: value.device_label,
                platform: value.device_platform.as_str(),
                trust_state: value.device_trust_state.as_str(),
            },
            created_at_unix_ms: value.created_at.value(),
            last_seen_at_unix_ms: value.last_seen_at.map(UtcMillis::value),
            revoked_at_unix_ms: value.revoked_at.map(UtcMillis::value),
            matrix_device_revoked_at_unix_ms: value.matrix_device_revoked_at.map(UtcMillis::value),
        }
    }
}

impl From<RevokedAgentInstance> for AgentInstanceRevocationResponse {
    fn from(value: RevokedAgentInstance) -> Self {
        let (matrix_cleanup, matrix_cleanup_pending_reason) = match value.matrix_cleanup {
            AgentInstanceMatrixCleanup::Complete => ("complete", None),
            AgentInstanceMatrixCleanup::Pending { reason } => {
                ("pending", Some(cleanup_reason(reason)))
            }
        };
        Self {
            instance: AgentInstanceSummaryResponse::from(value.instance),
            matrix_cleanup,
            matrix_cleanup_pending_reason,
        }
    }
}

const fn cleanup_reason(reason: AgentInstanceCleanupFailureKind) -> &'static str {
    match reason {
        AgentInstanceCleanupFailureKind::DependencyUnavailable => "dependencyUnavailable",
        AgentInstanceCleanupFailureKind::Rejected => "rejected",
        AgentInstanceCleanupFailureKind::Unsupported => "unsupported",
        AgentInstanceCleanupFailureKind::InvalidStoredIdentity => "invalidStoredIdentity",
        AgentInstanceCleanupFailureKind::StatePersistenceUnavailable => {
            "statePersistenceUnavailable"
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_room_application::{
        agent_instance_management::{
            AgentInstanceManagementResult, AgentInstanceManagementUseCases,
            AgentInstanceMatrixCleanup, ListAgentInstances, RevokeAgentInstance,
            RevokedAgentInstance,
        },
        authentication::{
            AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
            AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
        },
        ports::{AgentInstanceManagementRecord, PortFuture, SecretValue},
    };
    use agent_room_domain::{
        agents::{
            AgentInstance, AgentInstancePublicSigningKey, AgentInstanceStatus, AgentMatrixDeviceId,
        },
        devices::{DevicePlatform, DeviceTrustState},
        ids::{AdapterBindingId, AgentId, AgentInstanceId, DeviceId, PrincipalId},
        time::UtcMillis,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::{AgentInstanceHttpState, AgentInstanceHttpStateDependencies, router};

    const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
    const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";
    const NOW: i64 = 1_700_000_000_000;

    struct FakeInstances {
        list_requests: Mutex<Vec<ListAgentInstances>>,
        revoke_requests: Mutex<Vec<RevokeAgentInstance>>,
        matrix_cleanup: AgentInstanceMatrixCleanup,
    }

    impl AgentInstanceManagementUseCases for FakeInstances {
        fn list_instances(
            &self,
            request: ListAgentInstances,
        ) -> PortFuture<'_, AgentInstanceManagementResult<Vec<AgentInstanceManagementRecord>>>
        {
            self.list_requests
                .lock()
                .expect("实例列表请求锁可用")
                .push(request);
            Box::pin(async { Ok(vec![management_record(AgentInstanceStatus::Online)]) })
        }

        fn revoke_instance(
            &self,
            request: RevokeAgentInstance,
        ) -> PortFuture<'_, AgentInstanceManagementResult<RevokedAgentInstance>> {
            self.revoke_requests
                .lock()
                .expect("实例撤销请求锁可用")
                .push(request);
            let matrix_cleanup = self.matrix_cleanup;
            Box::pin(async move {
                Ok(RevokedAgentInstance {
                    instance: management_record(AgentInstanceStatus::Revoked),
                    matrix_cleanup,
                })
            })
        }
    }

    #[derive(Default)]
    struct FakeAuthentication {
        requirements: Mutex<Vec<AuthenticationRequirement>>,
    }

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            _request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            Box::pin(async { unreachable!("实例管理路由不会开始登录") })
        }

        fn complete_login<'a>(
            &'a self,
            _request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            Box::pin(async { unreachable!("实例管理路由不会完成登录") })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            assert_eq!(session_secret.expose(), "session-secret");
            self.requirements
                .lock()
                .expect("认证要求锁可用")
                .push(requirement);
            Box::pin(async { Ok(authenticated_principal()) })
        }

        fn logout<'a>(
            &'a self,
            _session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("实例管理路由不会退出登录") })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("实例管理路由不会暂停主体") })
        }
    }

    struct Fixture {
        app: axum::Router,
        instances: Arc<FakeInstances>,
        authentication: Arc<FakeAuthentication>,
    }

    impl Fixture {
        fn new(matrix_cleanup: AgentInstanceMatrixCleanup) -> Self {
            let instances = Arc::new(FakeInstances {
                list_requests: Mutex::new(Vec::new()),
                revoke_requests: Mutex::new(Vec::new()),
                matrix_cleanup,
            });
            let authentication = Arc::new(FakeAuthentication::default());
            let state = AgentInstanceHttpState::new(
                AgentInstanceHttpStateDependencies {
                    instances: instances.clone(),
                    authentication: authentication.clone(),
                },
                &Url::parse(FRONTEND_ORIGIN).expect("前端 Origin 有效"),
            );
            Self {
                app: router(state).layer(middleware::from_fn(crate::correlation::attach)),
                instances,
                authentication,
            }
        }
    }

    #[tokio::test]
    async fn 列表使用活跃会话并返回实例所属设备状态() {
        let fixture = Fixture::new(AgentInstanceMatrixCleanup::Complete);
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .uri("/agent-instances")
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .expect("实例列表请求有效"),
            )
            .await
            .expect("实例列表路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["instances"][0]["agentInstanceId"], INSTANCE_UUID);
        assert_eq!(payload["instances"][0]["status"], "online");
        assert_eq!(payload["instances"][0]["device"]["platform"], "windows");
        assert_eq!(
            fixture
                .instances
                .list_requests
                .lock()
                .expect("请求锁可用")
                .len(),
            1
        );
        assert_eq!(
            *fixture
                .authentication
                .requirements
                .lock()
                .expect("认证要求锁可用"),
            vec![AuthenticationRequirement::ActiveSession]
        );
    }

    #[tokio::test]
    async fn 远端清理暂挂返回_202_并强制同源近期认证() {
        let fixture = Fixture::new(AgentInstanceMatrixCleanup::Pending {
            reason: agent_room_application::agent_instance_management::AgentInstanceCleanupFailureKind::DependencyUnavailable,
        });
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/agent-instances/{INSTANCE_UUID}"))
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .expect("实例撤销请求有效"),
            )
            .await
            .expect("实例撤销路由可调用");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let payload = response_json(response).await;
        assert_eq!(payload["matrixCleanup"], "pending");
        assert_eq!(
            payload["matrixCleanupPendingReason"],
            "dependencyUnavailable"
        );
        let requests = fixture
            .instances
            .revoke_requests
            .lock()
            .expect("请求锁可用");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].instance_id.to_string(), INSTANCE_UUID);
        assert_eq!(
            *fixture
                .authentication
                .requirements
                .lock()
                .expect("认证要求锁可用"),
            vec![AuthenticationRequirement::RecentAuthentication]
        );
    }

    #[tokio::test]
    async fn 跨站撤销在认证和用例之前失败关闭() {
        let fixture = Fixture::new(AgentInstanceMatrixCleanup::Complete);
        let response = fixture
            .app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/agent-instances/{INSTANCE_UUID}"))
                    .header(header::ORIGIN, "https://evil.test")
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .expect("跨站撤销请求可构造"),
            )
            .await
            .expect("实例撤销路由可调用");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            fixture
                .instances
                .revoke_requests
                .lock()
                .expect("请求锁可用")
                .is_empty()
        );
        assert!(
            fixture
                .authentication
                .requirements
                .lock()
                .expect("认证要求锁可用")
                .is_empty()
        );
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    fn management_record(status: AgentInstanceStatus) -> AgentInstanceManagementRecord {
        let lease = (status == AgentInstanceStatus::Online).then(|| time(NOW + 60_000));
        let instance = AgentInstance::restore(
            AgentInstanceId::from_uuid(uuid(INSTANCE_UUID)),
            AgentId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e44")),
            DeviceId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e43")),
            AdapterBindingId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e46")),
            AgentInstancePublicSigningKey::new(vec![7; 32]).expect("实例公钥有效"),
            AgentMatrixDeviceId::new("AR_TEST".to_owned()).expect("Matrix 设备标识有效"),
            status,
            lease,
        )
        .expect("实例记录有效");
        AgentInstanceManagementRecord {
            instance,
            agent_matrix_user_id: "@_agent_test:matrix.agent-room.test".to_owned(),
            agent_display_name: "Codex Builder".to_owned(),
            agent_avatar_content_id: None,
            adapter_type: "codex-desktop".to_owned(),
            capability_version: "2026-08-25".to_owned(),
            device_label: "Windows 工作站".to_owned(),
            device_platform: DevicePlatform::Windows,
            device_trust_state: DeviceTrustState::Verified,
            created_at: time(NOW - 10_000),
            last_seen_at: Some(time(NOW - 1_000)),
            revoked_at: (status == AgentInstanceStatus::Revoked).then(|| time(NOW)),
            matrix_device_revoked_at: None,
        }
    }

    fn authenticated_principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: PrincipalId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e42")),
            matrix_user_id: "@operator:matrix.agent-room.test".to_owned(),
            display_name: "操作人".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(NOW - 1_000),
            expires_at: time(NOW + 60_000),
            recently_authenticated: true,
        }
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
