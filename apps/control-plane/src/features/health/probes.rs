use std::{io::ErrorKind, sync::Arc};

use agent_room_application::{
    health::{
        DependencyKind, DependencyProbe, ProbeFailure, ProbeFailureKind, ProbeResult,
        ReadinessService,
    },
    ports::PortFuture,
};
use futures_util::StreamExt;
use reqwest::{Client, Response, redirect::Policy};
use serde::Deserialize;
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use thiserror::Error;
use url::Url;

use crate::{
    SERVICE_NAME,
    config::{DatabaseConfig, DatabaseTlsMode, DependencyConfig},
    correlation::outbound_headers,
};

const MATRIX_RESPONSE_LIMIT_BYTES: usize = 64 * 1_024;

pub(crate) struct HealthRuntime {
    pub(crate) readiness: Arc<ReadinessService>,
    pool: PgPool,
}

impl HealthRuntime {
    pub(crate) fn initialize(config: &DependencyConfig) -> Result<Self, ProbeInitializationError> {
        let pool = create_pool(&config.database, config.timeout);
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.timeout)
            .redirect(Policy::none())
            .no_proxy()
            .user_agent(format!("{SERVICE_NAME}/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProbeInitializationError::HttpClient)?;
        let matrix_versions_url = config
            .matrix_base_url
            .join("/_matrix/client/versions")
            .map_err(|_| ProbeInitializationError::MatrixEndpoint)?;

        let probes: Vec<Arc<dyn DependencyProbe>> = vec![
            Arc::new(PostgreSqlProbe { pool: pool.clone() }),
            Arc::new(MatrixProbe {
                client: client.clone(),
                endpoint: matrix_versions_url,
            }),
            Arc::new(ObjectStoreProbe {
                client,
                endpoint: config.object_store_health_url.clone(),
            }),
        ];
        let readiness =
            ReadinessService::new(probes).map_err(|_| ProbeInitializationError::HealthModel)?;

        Ok(Self {
            readiness: Arc::new(readiness),
            pool,
        })
    }

    pub(crate) async fn shutdown(&self) {
        self.pool.close().await;
    }
}

fn create_pool(config: &DatabaseConfig, timeout: std::time::Duration) -> PgPool {
    let options = PgConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .database(&config.database)
        .username(&config.username)
        .password(config.password.expose())
        .ssl_mode(map_tls_mode(config.tls_mode))
        .disable_statement_logging();

    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(5)
        .acquire_timeout(timeout)
        .connect_lazy_with(options)
}

const fn map_tls_mode(mode: DatabaseTlsMode) -> PgSslMode {
    match mode {
        DatabaseTlsMode::Disable => PgSslMode::Disable,
        DatabaseTlsMode::Prefer => PgSslMode::Prefer,
        DatabaseTlsMode::Require => PgSslMode::Require,
        DatabaseTlsMode::VerifyCertificate => PgSslMode::VerifyCa,
        DatabaseTlsMode::VerifyIdentity => PgSslMode::VerifyFull,
    }
}

struct PostgreSqlProbe {
    pool: PgPool,
}

impl DependencyProbe for PostgreSqlProbe {
    fn dependency(&self) -> DependencyKind {
        DependencyKind::PostgreSql
    }

    fn check<'a>(&'a self, _correlation_id: &'a str) -> PortFuture<'a, ProbeResult> {
        Box::pin(async move {
            sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&self.pool)
                .await
                .map(|_| ())
                .map_err(|error| ProbeFailure::new(classify_sqlx_error(&error)))
        })
    }
}

fn classify_sqlx_error(error: &sqlx::Error) -> ProbeFailureKind {
    match error {
        sqlx::Error::PoolTimedOut => ProbeFailureKind::Timeout,
        sqlx::Error::Io(error) if error.kind() == ErrorKind::TimedOut => ProbeFailureKind::Timeout,
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::PoolClosed => {
            ProbeFailureKind::Connection
        }
        sqlx::Error::Database(_) => ProbeFailureKind::RejectedResponse,
        sqlx::Error::Protocol(_) => ProbeFailureKind::InvalidResponse,
        _ => ProbeFailureKind::Internal,
    }
}

#[derive(Debug, Deserialize)]
struct MatrixVersions {
    versions: Vec<String>,
}

struct MatrixProbe {
    client: Client,
    endpoint: Url,
}

impl DependencyProbe for MatrixProbe {
    fn dependency(&self) -> DependencyKind {
        DependencyKind::Matrix
    }

    fn check<'a>(&'a self, correlation_id: &'a str) -> PortFuture<'a, ProbeResult> {
        Box::pin(async move {
            let response = self
                .client
                .get(self.endpoint.clone())
                .headers(outbound_headers(correlation_id))
                .send()
                .await
                .map_err(|error| ProbeFailure::new(classify_reqwest_error(&error)))?;
            if !response.status().is_success() {
                return Err(ProbeFailure::new(ProbeFailureKind::RejectedResponse));
            }

            let body = read_limited_body(response, MATRIX_RESPONSE_LIMIT_BYTES).await?;
            let versions = serde_json::from_slice::<MatrixVersions>(&body)
                .map_err(|_| ProbeFailure::new(ProbeFailureKind::InvalidResponse))?;
            if versions.versions.is_empty() {
                return Err(ProbeFailure::new(ProbeFailureKind::InvalidResponse));
            }
            Ok(())
        })
    }
}

struct ObjectStoreProbe {
    client: Client,
    endpoint: Url,
}

impl DependencyProbe for ObjectStoreProbe {
    fn dependency(&self) -> DependencyKind {
        DependencyKind::ObjectStore
    }

    fn check<'a>(&'a self, correlation_id: &'a str) -> PortFuture<'a, ProbeResult> {
        Box::pin(async move {
            let response = self
                .client
                .get(self.endpoint.clone())
                .headers(outbound_headers(correlation_id))
                .send()
                .await
                .map_err(|error| ProbeFailure::new(classify_reqwest_error(&error)))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(ProbeFailure::new(ProbeFailureKind::RejectedResponse))
            }
        })
    }
}

async fn read_limited_body(response: Response, limit: usize) -> Result<Vec<u8>, ProbeFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX))
    {
        return Err(ProbeFailure::new(ProbeFailureKind::InvalidResponse));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ProbeFailure::new(classify_reqwest_error(&error)))?;
        let next_length = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| ProbeFailure::new(ProbeFailureKind::InvalidResponse))?;
        if next_length > limit {
            return Err(ProbeFailure::new(ProbeFailureKind::InvalidResponse));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn classify_reqwest_error(error: &reqwest::Error) -> ProbeFailureKind {
    if error.is_timeout() {
        ProbeFailureKind::Timeout
    } else if error.is_connect() {
        ProbeFailureKind::Connection
    } else if error.is_status() {
        ProbeFailureKind::RejectedResponse
    } else if error.is_decode() {
        ProbeFailureKind::InvalidResponse
    } else {
        ProbeFailureKind::Internal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum ProbeInitializationError {
    #[error("HTTP 依赖客户端初始化失败")]
    HttpClient,
    #[error("Matrix 健康地址初始化失败")]
    MatrixEndpoint,
    #[error("健康模型初始化失败")]
    HealthModel,
}
