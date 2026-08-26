use std::time::Duration;

use agent_room_application::ports::{
    MatrixAccountLifecycleGateway, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
    MatrixUserId, PortFuture, SecretValue,
};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    MAX_REQUEST_TIMEOUT, decode_matrix_error, map_matrix_error, map_transport_error,
    read_limited_body, validate_homeserver_url, validate_server_name,
};

#[derive(Clone)]
pub struct SynapseAccountLifecycleConfiguration {
    homeserver_url: Url,
    server_name: String,
    admin_access_token: SecretValue,
    request_timeout: Duration,
}

impl std::fmt::Debug for SynapseAccountLifecycleConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SynapseAccountLifecycleConfiguration")
            .field("homeserver_url", &self.homeserver_url)
            .field("server_name", &self.server_name)
            .field("admin_access_token", &self.admin_access_token)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl SynapseAccountLifecycleConfiguration {
    /// 创建只用于后台账户擦除的 Synapse Admin API 配置。
    ///
    /// # Errors
    ///
    /// 地址、服务名或超时不满足生产边界时拒绝启动。
    pub fn new(
        homeserver_url: impl AsRef<str>,
        server_name: impl Into<String>,
        admin_access_token: SecretValue,
        request_timeout: Duration,
    ) -> Result<Self, SynapseAccountLifecycleConfigurationError> {
        let homeserver_url = Url::parse(homeserver_url.as_ref())
            .map_err(|_| SynapseAccountLifecycleConfigurationError::InvalidHomeserverUrl)?;
        validate_homeserver_url(&homeserver_url)
            .map_err(|_| SynapseAccountLifecycleConfigurationError::InvalidHomeserverUrl)?;
        let server_name = server_name.into();
        validate_server_name(&server_name)
            .map_err(|_| SynapseAccountLifecycleConfigurationError::InvalidServerName)?;
        if request_timeout.is_zero() || request_timeout > MAX_REQUEST_TIMEOUT {
            return Err(SynapseAccountLifecycleConfigurationError::InvalidRequestTimeout);
        }
        Ok(Self {
            homeserver_url,
            server_name,
            admin_access_token,
            request_timeout,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SynapseAccountLifecycleConfigurationError {
    #[error("Synapse Homeserver 地址无效")]
    InvalidHomeserverUrl,
    #[error("Matrix 服务名无效")]
    InvalidServerName,
    #[error("Matrix 请求超时必须处于 1 毫秒到 120 秒之间")]
    InvalidRequestTimeout,
    #[error("无法创建 Synapse 账户生命周期客户端")]
    HttpClient,
}

#[derive(Clone)]
pub struct SynapseAccountLifecycleGateway {
    client: Client,
    homeserver_url: Url,
    server_name: String,
    admin_access_token: SecretValue,
}

impl SynapseAccountLifecycleGateway {
    /// 创建不跟随重定向的受限 Synapse Admin API 客户端。
    ///
    /// # Errors
    ///
    /// HTTP 客户端初始化失败时返回配置错误。
    pub fn new(
        configuration: SynapseAccountLifecycleConfiguration,
    ) -> Result<Self, SynapseAccountLifecycleConfigurationError> {
        let client = Client::builder()
            .timeout(configuration.request_timeout)
            .connect_timeout(configuration.request_timeout)
            .redirect(Policy::none())
            .build()
            .map_err(|_| SynapseAccountLifecycleConfigurationError::HttpClient)?;
        Ok(Self {
            client,
            homeserver_url: configuration.homeserver_url,
            server_name: configuration.server_name,
            admin_access_token: configuration.admin_access_token,
        })
    }

    async fn deactivate_and_erase_internal(&self, user_id: &MatrixUserId) -> MatrixResult<()> {
        let operation = MatrixOperation::DeactivateAccount;
        self.ensure_local_user(user_id, operation)?;
        let endpoint = self.user_endpoint("_synapse/admin/v1/deactivate", user_id, operation)?;
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(self.admin_access_token.expose())
            .json(&DeactivateAccountRequest { erase: true })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            self.clear_external_ids(user_id, operation).await?;
            self.delete_media(user_id, operation).await?;
            return Ok(());
        }
        let error = decode_matrix_error(&body, operation)?;
        if status == StatusCode::NOT_FOUND && error.errcode == "M_NOT_FOUND" {
            return Ok(());
        }
        Err(map_matrix_error(operation, status, &error))
    }

    async fn clear_external_ids(
        &self,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let endpoint = self.user_endpoint("_synapse/admin/v2/users", user_id, operation)?;
        let response = self
            .client
            .put(endpoint)
            .bearer_auth(self.admin_access_token.expose())
            .json(&ClearExternalIdsRequest {
                external_ids: Vec::new(),
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        self.require_success(response, operation).await
    }

    async fn delete_media(
        &self,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        const PAGE_LIMIT: usize = 100;
        const MAXIMUM_PAGES_PER_ATTEMPT: usize = 1_000;
        for _ in 0..MAXIMUM_PAGES_PER_ATTEMPT {
            let mut endpoint = self.user_endpoint("_synapse/admin/v1/users", user_id, operation)?;
            endpoint
                .path_segments_mut()
                .map_err(|()| {
                    MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration)
                })?
                .push("media");
            endpoint
                .query_pairs_mut()
                .append_pair("limit", &PAGE_LIMIT.to_string());
            let response = self
                .client
                .delete(endpoint)
                .bearer_auth(self.admin_access_token.expose())
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            let status = response.status();
            let body = read_limited_body(response, operation).await?;
            if !status.is_success() {
                let error = decode_matrix_error(&body, operation)?;
                return Err(map_matrix_error(operation, status, &error));
            }
            let deleted: DeletedMediaResponse = serde_json::from_slice(&body)
                .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse))?;
            if deleted.deleted_media.len() < PAGE_LIMIT {
                return Ok(());
            }
        }
        Err(MatrixFailure::new(
            operation,
            MatrixFailureKind::DependencyUnavailable,
        ))
    }

    async fn require_success(
        &self,
        response: reqwest::Response,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            return Ok(());
        }
        let error = decode_matrix_error(&body, operation)?;
        Err(map_matrix_error(operation, status, &error))
    }

    fn ensure_local_user(
        &self,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let suffix = format!(":{}", self.server_name);
        let localpart = user_id
            .as_str()
            .strip_prefix('@')
            .and_then(|value| value.strip_suffix(&suffix));
        if localpart.is_none_or(str::is_empty) {
            return Err(MatrixFailure::new(operation, MatrixFailureKind::Forbidden));
        }
        Ok(())
    }

    fn user_endpoint(
        &self,
        prefix: &str,
        user_id: &MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<Url> {
        let mut endpoint = self
            .homeserver_url
            .join(&format!("{prefix}/"))
            .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?;
        endpoint
            .path_segments_mut()
            .map_err(|()| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?
            .pop_if_empty()
            .push(user_id.as_str());
        Ok(endpoint)
    }
}

impl MatrixAccountLifecycleGateway for SynapseAccountLifecycleGateway {
    fn deactivate_and_erase<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.deactivate_and_erase_internal(user_id))
    }
}

#[derive(Serialize)]
struct DeactivateAccountRequest {
    erase: bool,
}

#[derive(Serialize)]
struct ClearExternalIdsRequest {
    external_ids: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct DeletedMediaResponse {
    deleted_media: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixAccountLifecycleGateway, MatrixFailureKind, MatrixUserId, SecretValue,
    };
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{delete, post, put},
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

    use super::{SynapseAccountLifecycleConfiguration, SynapseAccountLifecycleGateway};

    #[tokio::test]
    async fn 本地账户使用管理员令牌执行擦除且路径安全编码() {
        let server = TestServer::start(false).await;
        let gateway = gateway(&server.url);
        gateway
            .deactivate_and_erase(
                &MatrixUserId::new("@human name:matrix.agent-room.localhost").expect("用户有效"),
            )
            .await
            .expect("本地账户可擦除");
        assert_eq!(*server.requests.lock().await, 3);
    }

    #[tokio::test]
    async fn 已不存在账户等价于幂等成功() {
        let server = TestServer::start(true).await;
        gateway(&server.url)
            .deactivate_and_erase(
                &MatrixUserId::new("@human:matrix.agent-room.localhost").expect("用户有效"),
            )
            .await
            .expect("不存在账户已满足擦除目标");
    }

    #[tokio::test]
    async fn 远端账户在发请求前被拒绝() {
        let server = TestServer::start(false).await;
        let failure = gateway(&server.url)
            .deactivate_and_erase(&MatrixUserId::new("@human:remote.example").expect("用户有效"))
            .await
            .expect_err("不得用本地管理员擦除远端账户");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
        assert_eq!(*server.requests.lock().await, 0);
    }

    fn gateway(url: &str) -> SynapseAccountLifecycleGateway {
        SynapseAccountLifecycleGateway::new(
            SynapseAccountLifecycleConfiguration::new(
                url,
                "matrix.agent-room.localhost",
                SecretValue::new("admin-access-token").expect("令牌有效"),
                Duration::from_secs(2),
            )
            .expect("配置有效"),
        )
        .expect("客户端有效")
    }

    struct TestServer {
        url: String,
        requests: Arc<Mutex<usize>>,
        task: JoinHandle<()>,
    }

    impl TestServer {
        async fn start(missing: bool) -> Self {
            let requests = Arc::new(Mutex::new(0));
            let state = Arc::new(TestState {
                requests: requests.clone(),
                missing,
            });
            let app = Router::new()
                .route("/_synapse/admin/v1/deactivate/{user_id}", post(deactivate))
                .route(
                    "/_synapse/admin/v2/users/{user_id}",
                    put(clear_external_ids),
                )
                .route(
                    "/_synapse/admin/v1/users/{user_id}/media",
                    delete(delete_media),
                )
                .with_state(state);
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务正常");
            });
            Self {
                url: format!("http://{address}"),
                requests,
                task,
            }
        }
    }

    struct TestState {
        requests: Arc<Mutex<usize>>,
        missing: bool,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn deactivate(
        State(state): State<Arc<TestState>>,
        Path(user_id): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        *state.requests.lock().await += 1;
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer admin-access-token")
        );
        assert!(user_id.starts_with('@'));
        assert_eq!(body, json!({ "erase": true }));
        if state.missing {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "errcode": "M_NOT_FOUND" })),
            )
        } else {
            (
                StatusCode::OK,
                Json(json!({ "id_server_unbind_result": "no-support" })),
            )
        }
    }

    async fn clear_external_ids(
        State(state): State<Arc<TestState>>,
        Path(user_id): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        *state.requests.lock().await += 1;
        assert_admin_request(&headers, &user_id);
        assert_eq!(body, json!({ "external_ids": [] }));
        (StatusCode::OK, Json(json!({})))
    }

    async fn delete_media(
        State(state): State<Arc<TestState>>,
        Path(user_id): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        *state.requests.lock().await += 1;
        assert_admin_request(&headers, &user_id);
        (
            StatusCode::OK,
            Json(json!({ "deleted_media": [], "total": 0 })),
        )
    }

    fn assert_admin_request(headers: &HeaderMap, user_id: &str) {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer admin-access-token")
        );
        assert!(user_id.starts_with('@'));
    }
}
