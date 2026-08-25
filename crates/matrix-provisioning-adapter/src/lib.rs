//! Matrix Application Service 身份与受控房间操作基础设施适配器。

use std::{net::IpAddr, time::Duration};

use agent_room_application::ports::{
    MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
    MatrixAgentUserRegistration, MatrixDeviceId, MatrixFailure, MatrixFailureKind, MatrixOperation,
    MatrixResult, MatrixSession, MatrixSessionMetadata, MatrixUserId, PortFuture, SecretValue,
};
use agent_room_domain::time::DurationMillis;
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode, Url, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

mod rooms;

pub use rooms::MatrixApplicationServiceRoomMembership;

const MAX_RESPONSE_BYTES: usize = 16 * 1_024;
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_mins(2);
const MAX_SERVER_NAME_LENGTH: usize = 255;

#[derive(Clone)]
pub struct MatrixApplicationServiceConfiguration {
    homeserver_url: Url,
    server_name: String,
    access_token: SecretValue,
    request_timeout: Duration,
}

impl std::fmt::Debug for MatrixApplicationServiceConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixApplicationServiceConfiguration")
            .field("homeserver_url", &self.homeserver_url)
            .field("server_name", &self.server_name)
            .field("access_token", &self.access_token)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl MatrixApplicationServiceConfiguration {
    /// 创建受控 Matrix Application Service 客户端配置。
    ///
    /// # Errors
    ///
    /// Homeserver URL、服务名或请求超时越界时返回配置错误。
    pub fn new(
        homeserver_url: impl AsRef<str>,
        server_name: impl Into<String>,
        access_token: SecretValue,
        request_timeout: Duration,
    ) -> Result<Self, MatrixApplicationServiceConfigurationError> {
        let homeserver_url = Url::parse(homeserver_url.as_ref())
            .map_err(|_| MatrixApplicationServiceConfigurationError::InvalidHomeserverUrl)?;
        validate_homeserver_url(&homeserver_url)?;
        let server_name = server_name.into();
        validate_server_name(&server_name)?;
        if request_timeout.is_zero() || request_timeout > MAX_REQUEST_TIMEOUT {
            return Err(MatrixApplicationServiceConfigurationError::InvalidRequestTimeout);
        }
        Ok(Self {
            homeserver_url,
            server_name,
            access_token,
            request_timeout,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MatrixApplicationServiceConfigurationError {
    #[error("Matrix Homeserver 地址无效")]
    InvalidHomeserverUrl,
    #[error("Matrix Homeserver 生产地址必须使用 HTTPS")]
    InsecureHomeserverUrl,
    #[error("Matrix 服务名无效")]
    InvalidServerName,
    #[error("Matrix 请求超时必须处于 1 毫秒到 120 秒之间")]
    InvalidRequestTimeout,
    #[error("无法创建 Matrix Application Service HTTP 客户端")]
    HttpClient,
}

#[derive(Clone)]
pub struct MatrixApplicationServiceProvisioner {
    client: Client,
    homeserver_url: Url,
    server_name: String,
    access_token: SecretValue,
}

impl MatrixApplicationServiceProvisioner {
    /// 创建不跟随重定向且带有严格超时的 Application Service 适配器。
    ///
    /// # Errors
    ///
    /// HTTP 客户端无法初始化时返回错误。
    pub fn new(
        configuration: MatrixApplicationServiceConfiguration,
    ) -> Result<Self, MatrixApplicationServiceConfigurationError> {
        let client = Client::builder()
            .timeout(configuration.request_timeout)
            .connect_timeout(configuration.request_timeout)
            .redirect(Policy::none())
            .build()
            .map_err(|_| MatrixApplicationServiceConfigurationError::HttpClient)?;
        Ok(Self {
            client,
            homeserver_url: configuration.homeserver_url,
            server_name: configuration.server_name,
            access_token: configuration.access_token,
        })
    }

    /// 为一个受 Application Service 管理的 Agent 用户绑定房间成员能力。
    ///
    /// # Errors
    ///
    /// 用户不属于受控 Agent 命名空间时拒绝创建，避免 AS token 被滥用于冒充普通用户。
    pub fn room_membership(
        &self,
        user_id: MatrixUserId,
    ) -> MatrixResult<MatrixApplicationServiceRoomMembership> {
        self.ensure_managed_user(&user_id, MatrixOperation::Join)?;
        Ok(MatrixApplicationServiceRoomMembership::new(
            std::sync::Arc::new(self.clone()),
            user_id,
        ))
    }

    async fn ensure_user_internal(
        &self,
        registration: &MatrixAgentUserRegistration,
    ) -> MatrixResult<MatrixUserId> {
        let operation = MatrixOperation::ProvisionAgentUser;
        let expected_user_id = self.expected_user_id(registration)?;
        let response = self
            .client
            .post(self.endpoint("_matrix/client/v3/register", operation)?)
            .bearer_auth(self.access_token.expose())
            .json(&ApplicationServiceRegistrationRequest {
                login_type: "m.login.application_service",
                username: registration.localpart().as_str(),
                inhibit_login: true,
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            let registered: UserRegistrationResponse = decode_json(&body, operation)?;
            let user_id =
                MatrixUserId::new(registered.user_id).map_err(|_| invalid_response(operation))?;
            if user_id != expected_user_id {
                return Err(invalid_response(operation));
            }
            return Ok(user_id);
        }

        let error = decode_matrix_error(&body, operation)?;
        if status == StatusCode::BAD_REQUEST && error.errcode == "M_USER_IN_USE" {
            return Ok(expected_user_id);
        }
        Err(map_matrix_error(operation, status, &error))
    }

    async fn issue_device_session_internal(
        &self,
        request: &MatrixAgentDeviceSessionRequest,
    ) -> MatrixResult<MatrixSession> {
        let operation = MatrixOperation::IssueAgentDeviceSession;
        self.ensure_managed_user(request.user_id(), operation)?;
        let response = self
            .client
            .post(self.endpoint("_matrix/client/v3/login", operation)?)
            .bearer_auth(self.access_token.expose())
            .json(&ApplicationServiceLoginRequest {
                login_type: "m.login.application_service",
                identifier: MatrixUserIdentifier {
                    identifier_type: "m.id.user",
                    user: request.user_id().as_str(),
                },
                device_id: request.device_id().as_str(),
                initial_device_display_name: request.initial_device_display_name(),
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }

        let issued: DeviceSessionResponse = decode_json(&body, operation)?;
        let user_id = MatrixUserId::new(issued.user_id).map_err(|_| invalid_response(operation))?;
        let device_id =
            MatrixDeviceId::new(issued.device_id).map_err(|_| invalid_response(operation))?;
        if &user_id != request.user_id() || &device_id != request.device_id() {
            return Err(invalid_response(operation));
        }
        let access_token =
            SecretValue::new(issued.access_token).map_err(|_| invalid_response(operation))?;
        let refresh_token = issued
            .refresh_token
            .map(SecretValue::new)
            .transpose()
            .map_err(|_| invalid_response(operation))?;
        Ok(MatrixSession::new(
            MatrixSessionMetadata::new(user_id, device_id),
            access_token,
            refresh_token,
        ))
    }

    fn expected_user_id(
        &self,
        registration: &MatrixAgentUserRegistration,
    ) -> MatrixResult<MatrixUserId> {
        MatrixUserId::new(format!(
            "@{}:{}",
            registration.localpart().as_str(),
            self.server_name
        ))
        .map_err(|_| invalid_response(MatrixOperation::ProvisionAgentUser))
    }

    fn ensure_managed_user(
        &self,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let suffix = format!(":{}", self.server_name);
        let localpart = user_id
            .as_str()
            .strip_prefix('@')
            .and_then(|value| value.strip_suffix(&suffix))
            .ok_or_else(|| MatrixFailure::new(operation, MatrixFailureKind::Forbidden))?;
        if MatrixAgentLocalpart::new(localpart).is_err() {
            return Err(MatrixFailure::new(operation, MatrixFailureKind::Forbidden));
        }
        Ok(())
    }

    fn endpoint(&self, path: &str, operation: MatrixOperation) -> MatrixResult<Url> {
        self.homeserver_url
            .join(path)
            .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))
    }
}

impl MatrixAgentIdentityProvisioner for MatrixApplicationServiceProvisioner {
    fn ensure_user<'a>(
        &'a self,
        registration: &'a MatrixAgentUserRegistration,
    ) -> PortFuture<'a, MatrixResult<MatrixUserId>> {
        Box::pin(self.ensure_user_internal(registration))
    }

    fn issue_device_session<'a>(
        &'a self,
        request: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
        Box::pin(self.issue_device_session_internal(request))
    }
}

#[derive(Serialize)]
struct ApplicationServiceRegistrationRequest<'a> {
    #[serde(rename = "type")]
    login_type: &'static str,
    username: &'a str,
    inhibit_login: bool,
}

#[derive(Deserialize)]
struct UserRegistrationResponse {
    user_id: String,
}

#[derive(Serialize)]
struct ApplicationServiceLoginRequest<'a> {
    #[serde(rename = "type")]
    login_type: &'static str,
    identifier: MatrixUserIdentifier<'a>,
    device_id: &'a str,
    initial_device_display_name: &'a str,
}

#[derive(Serialize)]
struct MatrixUserIdentifier<'a> {
    #[serde(rename = "type")]
    identifier_type: &'static str,
    user: &'a str,
}

#[derive(Deserialize)]
struct DeviceSessionResponse {
    user_id: String,
    device_id: String,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct MatrixErrorResponse {
    errcode: String,
    #[serde(default)]
    retry_after_ms: Option<u64>,
}

async fn read_limited_body(
    response: Response,
    operation: MatrixOperation,
) -> MatrixResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX))
    {
        return Err(invalid_response(operation));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| map_transport_error(operation, &error))?;
        let next_length = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| invalid_response(operation))?;
        if next_length > MAX_RESPONSE_BYTES {
            return Err(invalid_response(operation));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn decode_json<T: DeserializeOwned>(body: &[u8], operation: MatrixOperation) -> MatrixResult<T> {
    serde_json::from_slice(body).map_err(|_| invalid_response(operation))
}

fn decode_matrix_error(
    body: &[u8],
    operation: MatrixOperation,
) -> MatrixResult<MatrixErrorResponse> {
    decode_json(body, operation)
}

fn map_matrix_error(
    operation: MatrixOperation,
    status: StatusCode,
    error: &MatrixErrorResponse,
) -> MatrixFailure {
    if error.errcode == "M_APPSERVICE_LOGIN_UNSUPPORTED" || error.errcode == "M_UNRECOGNIZED" {
        return MatrixFailure::new(operation, MatrixFailureKind::UnsupportedVersion);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = error
            .retry_after_ms
            .and_then(|value| DurationMillis::new(value.max(1)).ok());
        return MatrixFailure::rate_limited(operation, retry_after);
    }
    if matches!(
        error.errcode.as_str(),
        "M_ROOM_IN_USE" | "M_BAD_STATE" | "M_INVALID_ROOM_STATE"
    ) {
        return MatrixFailure::new(operation, MatrixFailureKind::Conflict);
    }
    let kind = match status.as_u16() {
        401 => MatrixFailureKind::Unauthenticated,
        403 => MatrixFailureKind::Forbidden,
        404 => MatrixFailureKind::NotFound,
        409 => MatrixFailureKind::Conflict,
        500..=599 => MatrixFailureKind::DependencyUnavailable,
        _ => MatrixFailureKind::InvalidResponse,
    };
    MatrixFailure::new(operation, kind)
}

fn map_transport_error(operation: MatrixOperation, error: &reqwest::Error) -> MatrixFailure {
    let kind = if error.is_timeout() {
        MatrixFailureKind::Timeout
    } else if error.is_connect() {
        MatrixFailureKind::DependencyUnavailable
    } else {
        MatrixFailureKind::InvalidResponse
    };
    MatrixFailure::new(operation, kind)
}

const fn invalid_response(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

fn validate_homeserver_url(url: &Url) -> Result<(), MatrixApplicationServiceConfigurationError> {
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(MatrixApplicationServiceConfigurationError::InvalidHomeserverUrl);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        _ => Err(MatrixApplicationServiceConfigurationError::InsecureHomeserverUrl),
    }
}

fn validate_server_name(
    server_name: &str,
) -> Result<(), MatrixApplicationServiceConfigurationError> {
    if server_name.is_empty()
        || server_name.len() > MAX_SERVER_NAME_LENGTH
        || server_name.chars().any(char::is_control)
        || server_name.contains(['/', '\\', '@', '#'])
    {
        return Err(MatrixApplicationServiceConfigurationError::InvalidServerName);
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
        MatrixAgentUserRegistration, MatrixDeviceId, MatrixFailureKind, SecretValue,
    };
    use agent_room_domain::ids::AgentId;
    use axum::{
        Json, Router,
        body::Body,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
    use uuid::Uuid;

    use super::{
        MAX_RESPONSE_BYTES, MatrixApplicationServiceConfiguration,
        MatrixApplicationServiceProvisioner, MatrixErrorResponse, map_matrix_error,
    };

    #[test]
    fn 房间别名占用必须进入冲突对账而不是伪装协议损坏() {
        let failure = map_matrix_error(
            agent_room_application::ports::MatrixOperation::CreateRoom,
            StatusCode::BAD_REQUEST,
            &MatrixErrorResponse {
                errcode: "M_ROOM_IN_USE".to_owned(),
                retry_after_ms: None,
            },
        );

        assert_eq!(failure.kind(), MatrixFailureKind::Conflict);
    }

    #[tokio::test]
    async fn 已存在用户被视为幂等对账成功() {
        let server = TestServer::start(TestMode::ExistingUser).await;
        let provisioner = provisioner(&server.url);
        let registration = registration();

        let user_id = provisioner
            .ensure_user(&registration)
            .await
            .expect("重复创建应返回稳定用户");
        assert_eq!(
            user_id.as_str(),
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost"
        );
        server.assert_authenticated_requests(1).await;
    }

    #[tokio::test]
    async fn 设备会话必须与请求的用户和设备完全一致() {
        let server = TestServer::start(TestMode::Success).await;
        let provisioner = provisioner(&server.url);
        let user_id = provisioner
            .ensure_user(&registration())
            .await
            .expect("可创建 Agent 用户");
        let request = MatrixAgentDeviceSessionRequest::new(
            user_id,
            MatrixDeviceId::new("AR_INSTANCE_1").expect("设备标识有效"),
            "Agent Room 实例".to_owned(),
        )
        .expect("会话请求有效");

        let session = provisioner
            .issue_device_session(&request)
            .await
            .expect("可签发设备会话");
        assert_eq!(session.metadata().device_id(), request.device_id());
        assert_eq!(session.metadata().user_id(), request.user_id());
        assert_eq!(session.access_token().expose(), "matrix-device-access");
        server.assert_authenticated_requests(2).await;
    }

    #[tokio::test]
    async fn 过大响应会在_json_解析前被拒绝() {
        let server = TestServer::start(TestMode::Oversized).await;
        let failure = provisioner(&server.url)
            .ensure_user(&registration())
            .await
            .expect_err("过大响应必须失败");
        assert_eq!(failure.kind(), MatrixFailureKind::InvalidResponse);
    }

    #[test]
    fn 配置调试输出不泄漏_application_service_token() {
        let configuration = configuration("http://127.0.0.1:8008");
        let rendered = format!("{configuration:?}");
        assert!(!rendered.contains("application-service-secret"));
        assert!(rendered.contains("已脱敏"));
    }

    fn registration() -> MatrixAgentUserRegistration {
        let agent_id = AgentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"),
        );
        MatrixAgentUserRegistration::new(MatrixAgentLocalpart::from_agent_id(agent_id))
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        MatrixApplicationServiceProvisioner::new(configuration(url)).expect("适配器配置有效")
    }

    fn configuration(url: &str) -> MatrixApplicationServiceConfiguration {
        MatrixApplicationServiceConfiguration::new(
            url,
            "matrix.agent-room.localhost",
            SecretValue::new("application-service-secret").expect("测试密钥有效"),
            Duration::from_secs(2),
        )
        .expect("配置有效")
    }

    #[derive(Clone, Copy)]
    enum TestMode {
        Success,
        ExistingUser,
        Oversized,
    }

    struct TestServer {
        url: String,
        state: Arc<TestState>,
        task: JoinHandle<()>,
    }

    struct TestState {
        mode: TestMode,
        authenticated_requests: Mutex<usize>,
    }

    impl TestServer {
        async fn start(mode: TestMode) -> Self {
            let state = Arc::new(TestState {
                mode,
                authenticated_requests: Mutex::new(0),
            });
            let app = Router::new()
                .route("/_matrix/client/v3/register", post(register))
                .route("/_matrix/client/v3/login", post(login))
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务正常");
            });
            Self {
                url: format!("http://{address}"),
                state,
                task,
            }
        }

        async fn assert_authenticated_requests(&self, expected: usize) {
            assert_eq!(*self.state.authenticated_requests.lock().await, expected);
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn register(
        State(state): State<Arc<TestState>>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        record_authentication(&state, &headers).await;
        assert_eq!(payload["type"], "m.login.application_service");
        assert_eq!(payload["inhibit_login"], true);
        match state.mode {
            TestMode::Success => (
                StatusCode::OK,
                Json(json!({
                    "user_id": "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost"
                })),
            )
                .into_response(),
            TestMode::ExistingUser => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "errcode": "M_USER_IN_USE" })),
            )
                .into_response(),
            TestMode::Oversized => ResponseWithBody::oversized(),
        }
    }

    async fn login(
        State(state): State<Arc<TestState>>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        record_authentication(&state, &headers).await;
        assert_eq!(payload["type"], "m.login.application_service");
        assert_eq!(payload["device_id"], "AR_INSTANCE_1");
        (
            StatusCode::OK,
            Json(json!({
                "user_id": "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
                "device_id": "AR_INSTANCE_1",
                "access_token": "matrix-device-access"
            })),
        )
    }

    async fn record_authentication(state: &TestState, headers: &HeaderMap) {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer application-service-secret")
        );
        *state.authenticated_requests.lock().await += 1;
    }

    struct ResponseWithBody;

    impl ResponseWithBody {
        fn oversized() -> Response {
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(vec![b'x'; MAX_RESPONSE_BYTES + 1]))
                .expect("测试响应有效")
                .into_response()
        }
    }
}
