use std::{env, fmt, fs, net::SocketAddr, time::Duration};

use agent_room_domain::{content::MAX_CONTENT_BYTES, ids::AgentId};
use thiserror::Error;
use url::Url;
use uuid::{Uuid, Version};

const DEFAULT_DEPENDENCY_TIMEOUT_MILLIS: u64 = 2_000;
const DEFAULT_OTEL_EXPORT_TIMEOUT_MILLIS: u64 = 5_000;
const DEFAULT_LOGIN_ATTEMPT_TTL_MILLIS: u64 = 10 * 60 * 1_000;
const DEFAULT_WEB_SESSION_TTL_MILLIS: u64 = 8 * 60 * 60 * 1_000;
const DEFAULT_RECENT_AUTHENTICATION_MILLIS: u64 = 5 * 60 * 1_000;
const DEFAULT_CLOCK_SKEW_MILLIS: u64 = 60 * 1_000;
const DEFAULT_DEVICE_ACCESS_TOKEN_TTL_MILLIS: u64 = 15 * 60 * 1_000;
const DEFAULT_DEVICE_REFRESH_TOKEN_TTL_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;
const DEFAULT_DEVICE_PROOF_MAXIMUM_AGE_MILLIS: u64 = 2 * 60 * 1_000;
const DEFAULT_DEVICE_AUTHORIZATION_MAXIMUM_AGE_MILLIS: u64 = 10 * 60 * 1_000;
const DEFAULT_LOBBY_RESERVATION_LIFETIME_MILLIS: u64 = 60 * 1_000;
const DEFAULT_LOBBY_PROVISIONING_LEASE_MILLIS: u64 = 30 * 1_000;
const DEFAULT_CONTENT_OBJECT_TIMEOUT_MILLIS: u64 = 30_000;
const DEFAULT_CONTENT_SCANNER_CONNECT_TIMEOUT_MILLIS: u64 = 2_000;
const DEFAULT_CONTENT_SCANNER_TIMEOUT_MILLIS: u64 = 60_000;
const DEFAULT_CONTENT_READ_TICKET_TTL_MILLIS: u64 = 60_000;
const DEFAULT_CONTENT_DOWNLOAD_WINDOW_MILLIS: u64 = 60_000;
const DEFAULT_CONTENT_DOWNLOAD_MAX_REQUESTS: u32 = 30;
const DEFAULT_CONTENT_DOWNLOAD_MAX_BYTES: u64 = 250 * 1_024 * 1_024;
const DEFAULT_CONTENT_CLEANUP_INTERVAL_MILLIS: u64 = 60_000;
const DEFAULT_CONTENT_ORPHAN_GRACE_MILLIS: u64 = 15 * 60 * 1_000;
const DEFAULT_CONTENT_CLEANUP_BATCH: u16 = 100;
const MAX_TEXT_LENGTH: usize = 1_024;

trait EnvironmentSource {
    fn read(&self, name: &str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn read(&self, name: &str) -> Option<String> {
        env::var(name).ok()
    }
}

#[derive(Clone)]
pub(crate) struct SecretValue(String);

impl SecretValue {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[已脱敏]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatabaseTlsMode {
    Disable,
    Prefer,
    Require,
    VerifyCertificate,
    VerifyIdentity,
}

#[derive(Clone)]
pub(crate) struct DatabaseConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) database: String,
    pub(crate) username: String,
    pub(crate) password: SecretValue,
    pub(crate) tls_mode: DatabaseTlsMode,
}

#[derive(Clone)]
pub(crate) struct DependencyConfig {
    pub(crate) database: DatabaseConfig,
    pub(crate) matrix_base_url: Url,
    pub(crate) object_store_health_url: Url,
    pub(crate) timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct ObservabilityConfig {
    pub(crate) log_filter: String,
    pub(crate) otlp_traces_endpoint: Option<Url>,
    pub(crate) export_timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct AuthenticationConfig {
    pub(crate) issuer_url: Url,
    pub(crate) client_id: String,
    pub(crate) client_secret: SecretValue,
    pub(crate) device_client_id: String,
    pub(crate) redirect_url: Url,
    pub(crate) frontend_origin: Url,
    pub(crate) matrix_server_name: String,
    pub(crate) login_attempt_ttl: Duration,
    pub(crate) web_session_ttl: Duration,
    pub(crate) recent_authentication_window: Duration,
    pub(crate) allowed_clock_skew: Duration,
    pub(crate) device_access_token_ttl: Duration,
    pub(crate) device_refresh_token_ttl: Duration,
    pub(crate) device_proof_maximum_age: Duration,
    pub(crate) device_authorization_maximum_age: Duration,
}

#[derive(Clone)]
pub(crate) struct AgentIdentityConfig {
    pub(crate) matrix_application_service_token: SecretValue,
}

#[derive(Clone)]
pub(crate) struct LobbyConfig {
    pub(crate) reservation_lifetime: Duration,
    pub(crate) provisioning_lease_lifetime: Duration,
}

#[derive(Clone)]
pub(crate) struct ContentConfig {
    pub(crate) object_store_endpoint: Url,
    pub(crate) object_store_bucket: String,
    pub(crate) object_store_region: String,
    pub(crate) object_store_access_key: SecretValue,
    pub(crate) object_store_secret_key: SecretValue,
    pub(crate) object_store_timeout: Duration,
    pub(crate) scanner_address: String,
    pub(crate) scanner_connect_timeout: Duration,
    pub(crate) scanner_timeout: Duration,
    pub(crate) ticket_key_id: String,
    pub(crate) ticket_secret: SecretValue,
    pub(crate) matrix_authority_agent_id: AgentId,
    pub(crate) read_ticket_ttl: Duration,
    pub(crate) download_window: Duration,
    pub(crate) download_max_requests: u32,
    pub(crate) download_max_bytes: u64,
    pub(crate) cleanup_interval: Duration,
    pub(crate) orphan_grace: Duration,
    pub(crate) cleanup_batch: u16,
}

#[derive(Clone)]
pub(crate) struct ControlPlaneConfig {
    pub(crate) bind_address: SocketAddr,
    pub(crate) dependencies: DependencyConfig,
    pub(crate) authentication: AuthenticationConfig,
    pub(crate) agent_identity: AgentIdentityConfig,
    pub(crate) lobby: LobbyConfig,
    pub(crate) content: ContentConfig,
    pub(crate) observability: ObservabilityConfig,
}

impl ControlPlaneConfig {
    pub(crate) fn from_environment() -> Result<Self, ConfigError> {
        Self::from_source(&ProcessEnvironment)
    }

    fn from_source(source: &impl EnvironmentSource) -> Result<Self, ConfigError> {
        Ok(Self {
            bind_address: read_bind_address(source)?,
            dependencies: read_dependency_config(source)?,
            authentication: read_authentication_config(source)?,
            agent_identity: read_agent_identity_config(source)?,
            lobby: read_lobby_config(source)?,
            content: read_content_config(source)?,
            observability: read_observability_config(source)?,
        })
    }
}

fn read_lobby_config(source: &impl EnvironmentSource) -> Result<LobbyConfig, ConfigError> {
    Ok(LobbyConfig {
        reservation_lifetime: read_bounded_duration(
            source,
            "AGENT_ROOM_LOBBY_RESERVATION_TTL_MS",
            DEFAULT_LOBBY_RESERVATION_LIFETIME_MILLIS,
            1_000..=5 * 60 * 1_000,
        )?,
        provisioning_lease_lifetime: read_bounded_duration(
            source,
            "AGENT_ROOM_LOBBY_PROVISIONING_LEASE_MS",
            DEFAULT_LOBBY_PROVISIONING_LEASE_MILLIS,
            1_000..=5 * 60 * 1_000,
        )?,
    })
}

fn read_content_config(source: &impl EnvironmentSource) -> Result<ContentConfig, ConfigError> {
    let scanner_connect_timeout = read_bounded_duration(
        source,
        "AGENT_ROOM_CONTENT_SCANNER_CONNECT_TIMEOUT_MS",
        DEFAULT_CONTENT_SCANNER_CONNECT_TIMEOUT_MILLIS,
        100..=30_000,
    )?;
    let scanner_timeout = read_bounded_duration(
        source,
        "AGENT_ROOM_CONTENT_SCANNER_TIMEOUT_MS",
        DEFAULT_CONTENT_SCANNER_TIMEOUT_MILLIS,
        1_000..=5 * 60 * 1_000,
    )?;
    if scanner_connect_timeout > scanner_timeout {
        return Err(ConfigError::invalid(
            "AGENT_ROOM_CONTENT_SCANNER_CONNECT_TIMEOUT_MS",
            "不得大于扫描总超时",
        ));
    }
    Ok(ContentConfig {
        object_store_endpoint: parse_http_url(
            "AGENT_ROOM_CONTENT_S3_ENDPOINT",
            &read_required_text(source, "AGENT_ROOM_CONTENT_S3_ENDPOINT")?,
        )?,
        object_store_bucket: read_required_text(source, "AGENT_ROOM_CONTENT_S3_BUCKET")?,
        object_store_region: read_required_text(source, "AGENT_ROOM_CONTENT_S3_REGION")?,
        object_store_access_key: SecretValue(read_required_secret(
            source,
            "AGENT_ROOM_CONTENT_S3_ACCESS_KEY",
        )?),
        object_store_secret_key: SecretValue(read_required_secret(
            source,
            "AGENT_ROOM_CONTENT_S3_SECRET_KEY",
        )?),
        object_store_timeout: read_bounded_duration(
            source,
            "AGENT_ROOM_CONTENT_S3_TIMEOUT_MS",
            DEFAULT_CONTENT_OBJECT_TIMEOUT_MILLIS,
            100..=5 * 60 * 1_000,
        )?,
        scanner_address: read_required_text(source, "AGENT_ROOM_CONTENT_SCANNER_ADDRESS")?,
        scanner_connect_timeout,
        scanner_timeout,
        ticket_key_id: read_required_text(source, "AGENT_ROOM_CONTENT_TICKET_KEY_ID")?,
        ticket_secret: SecretValue(read_required_secret(
            source,
            "AGENT_ROOM_CONTENT_TICKET_SECRET",
        )?),
        matrix_authority_agent_id: read_agent_id(source, "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID")?,
        read_ticket_ttl: read_bounded_duration(
            source,
            "AGENT_ROOM_CONTENT_READ_TICKET_TTL_MS",
            DEFAULT_CONTENT_READ_TICKET_TTL_MILLIS,
            1_000..=5 * 60 * 1_000,
        )?,
        download_window: read_bounded_duration(
            source,
            "AGENT_ROOM_CONTENT_DOWNLOAD_WINDOW_MS",
            DEFAULT_CONTENT_DOWNLOAD_WINDOW_MILLIS,
            1_000..=60 * 60 * 1_000,
        )?,
        download_max_requests: read_bounded_u32(
            source,
            "AGENT_ROOM_CONTENT_DOWNLOAD_MAX_REQUESTS",
            DEFAULT_CONTENT_DOWNLOAD_MAX_REQUESTS,
            1..=10_000,
        )?,
        download_max_bytes: read_bounded_u64(
            source,
            "AGENT_ROOM_CONTENT_DOWNLOAD_MAX_BYTES",
            DEFAULT_CONTENT_DOWNLOAD_MAX_BYTES,
            MAX_CONTENT_BYTES..=1_024 * 1_024 * 1_024 * 1_024,
        )?,
        cleanup_interval: read_bounded_duration(
            source,
            "AGENT_ROOM_CONTENT_CLEANUP_INTERVAL_MS",
            DEFAULT_CONTENT_CLEANUP_INTERVAL_MILLIS,
            1_000..=60 * 60 * 1_000,
        )?,
        orphan_grace: read_bounded_duration(
            source,
            "AGENT_ROOM_CONTENT_ORPHAN_GRACE_MS",
            DEFAULT_CONTENT_ORPHAN_GRACE_MILLIS,
            1_000..=24 * 60 * 60 * 1_000,
        )?,
        cleanup_batch: read_bounded_u16(
            source,
            "AGENT_ROOM_CONTENT_CLEANUP_BATCH",
            DEFAULT_CONTENT_CLEANUP_BATCH,
            1..=500,
        )?,
    })
}

fn read_agent_identity_config(
    source: &impl EnvironmentSource,
) -> Result<AgentIdentityConfig, ConfigError> {
    Ok(AgentIdentityConfig {
        matrix_application_service_token: SecretValue(read_required_secret(
            source,
            "AGENT_ROOM_MATRIX_APPSERVICE_TOKEN",
        )?),
    })
}

fn read_bind_address(source: &impl EnvironmentSource) -> Result<SocketAddr, ConfigError> {
    read_optional(source, "AGENT_ROOM_BIND_ADDRESS")
        .unwrap_or_else(|| "127.0.0.1:8090".to_owned())
        .parse::<SocketAddr>()
        .map_err(|_| ConfigError::invalid("AGENT_ROOM_BIND_ADDRESS", "必须是 IP:端口"))
}

fn read_dependency_config(
    source: &impl EnvironmentSource,
) -> Result<DependencyConfig, ConfigError> {
    let timeout = read_bounded_duration(
        source,
        "AGENT_ROOM_DEPENDENCY_TIMEOUT_MS",
        DEFAULT_DEPENDENCY_TIMEOUT_MILLIS,
        100..=30_000,
    )?;
    Ok(DependencyConfig {
        database: DatabaseConfig {
            host: read_required_text(source, "AGENT_ROOM_DB_HOST")?,
            port: read_required_u16(source, "AGENT_ROOM_DB_PORT")?,
            database: read_required_text(source, "AGENT_ROOM_DB_NAME")?,
            username: read_required_text(source, "AGENT_ROOM_DB_USER")?,
            password: SecretValue(read_required_secret(
                source,
                "AGENT_ROOM_DB_RUNTIME_PASSWORD",
            )?),
            tls_mode: parse_database_tls_mode(&read_required_text(
                source,
                "AGENT_ROOM_DB_TLS_MODE",
            )?)?,
        },
        matrix_base_url: parse_http_url(
            "AGENT_ROOM_MATRIX_BASE_URL",
            &read_required_text(source, "AGENT_ROOM_MATRIX_BASE_URL")?,
        )?,
        object_store_health_url: parse_http_url(
            "AGENT_ROOM_OBJECT_STORE_HEALTH_URL",
            &read_required_text(source, "AGENT_ROOM_OBJECT_STORE_HEALTH_URL")?,
        )?,
        timeout,
    })
}

fn read_authentication_config(
    source: &impl EnvironmentSource,
) -> Result<AuthenticationConfig, ConfigError> {
    Ok(AuthenticationConfig {
        issuer_url: parse_http_url(
            "AGENT_ROOM_OIDC_ISSUER_URL",
            &read_required_text(source, "AGENT_ROOM_OIDC_ISSUER_URL")?,
        )?,
        client_id: read_required_text(source, "AGENT_ROOM_OIDC_CLIENT_ID")?,
        client_secret: SecretValue(read_required_secret(
            source,
            "AGENT_ROOM_OIDC_CLIENT_SECRET",
        )?),
        device_client_id: read_required_text(source, "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID")?,
        redirect_url: parse_http_url(
            "AGENT_ROOM_OIDC_REDIRECT_URL",
            &read_required_text(source, "AGENT_ROOM_OIDC_REDIRECT_URL")?,
        )?,
        frontend_origin: parse_origin(
            "AGENT_ROOM_FRONTEND_ORIGIN",
            &read_required_text(source, "AGENT_ROOM_FRONTEND_ORIGIN")?,
        )?,
        matrix_server_name: read_required_text(source, "AGENT_ROOM_MATRIX_SERVER_NAME")?,
        login_attempt_ttl: read_bounded_duration(
            source,
            "AGENT_ROOM_LOGIN_ATTEMPT_TTL_MS",
            DEFAULT_LOGIN_ATTEMPT_TTL_MILLIS,
            60_000..=30 * 60 * 1_000,
        )?,
        web_session_ttl: read_bounded_duration(
            source,
            "AGENT_ROOM_WEB_SESSION_TTL_MS",
            DEFAULT_WEB_SESSION_TTL_MILLIS,
            5 * 60 * 1_000..=30 * 24 * 60 * 60 * 1_000,
        )?,
        recent_authentication_window: read_bounded_duration(
            source,
            "AGENT_ROOM_RECENT_AUTHENTICATION_MS",
            DEFAULT_RECENT_AUTHENTICATION_MILLIS,
            60_000..=60 * 60 * 1_000,
        )?,
        allowed_clock_skew: read_bounded_duration(
            source,
            "AGENT_ROOM_ALLOWED_CLOCK_SKEW_MS",
            DEFAULT_CLOCK_SKEW_MILLIS,
            1_000..=5 * 60 * 1_000,
        )?,
        device_access_token_ttl: read_bounded_duration(
            source,
            "AGENT_ROOM_DEVICE_ACCESS_TOKEN_TTL_MS",
            DEFAULT_DEVICE_ACCESS_TOKEN_TTL_MILLIS,
            60_000..=60 * 60 * 1_000,
        )?,
        device_refresh_token_ttl: read_bounded_duration(
            source,
            "AGENT_ROOM_DEVICE_REFRESH_TOKEN_TTL_MS",
            DEFAULT_DEVICE_REFRESH_TOKEN_TTL_MILLIS,
            60 * 60 * 1_000..=90 * 24 * 60 * 60 * 1_000,
        )?,
        device_proof_maximum_age: read_bounded_duration(
            source,
            "AGENT_ROOM_DEVICE_PROOF_MAXIMUM_AGE_MS",
            DEFAULT_DEVICE_PROOF_MAXIMUM_AGE_MILLIS,
            5_000..=5 * 60 * 1_000,
        )?,
        device_authorization_maximum_age: read_bounded_duration(
            source,
            "AGENT_ROOM_DEVICE_AUTHORIZATION_MAXIMUM_AGE_MS",
            DEFAULT_DEVICE_AUTHORIZATION_MAXIMUM_AGE_MILLIS,
            5 * 60 * 1_000..=30 * 60 * 1_000,
        )?,
    })
}

fn read_observability_config(
    source: &impl EnvironmentSource,
) -> Result<ObservabilityConfig, ConfigError> {
    let log_filter = read_optional(source, "AGENT_ROOM_LOG_FILTER")
        .unwrap_or_else(|| "agent_room_control_plane=info,tower_http=info,sqlx=warn".to_owned());
    validate_text("AGENT_ROOM_LOG_FILTER", &log_filter)?;
    Ok(ObservabilityConfig {
        log_filter,
        otlp_traces_endpoint: read_optional(source, "AGENT_ROOM_OTLP_TRACES_ENDPOINT")
            .map(|value| parse_http_url("AGENT_ROOM_OTLP_TRACES_ENDPOINT", &value))
            .transpose()?,
        export_timeout: read_bounded_duration(
            source,
            "AGENT_ROOM_OTEL_EXPORT_TIMEOUT_MS",
            DEFAULT_OTEL_EXPORT_TIMEOUT_MILLIS,
            100..=30_000,
        )?,
    })
}

fn read_optional(source: &impl EnvironmentSource, name: &'static str) -> Option<String> {
    source.read(name).filter(|value| !value.trim().is_empty())
}

fn read_required_text(
    source: &impl EnvironmentSource,
    name: &'static str,
) -> Result<String, ConfigError> {
    let value = read_optional(source, name).ok_or(ConfigError::Missing { name })?;
    let value = value.trim().to_owned();
    validate_text(name, &value)?;
    Ok(value)
}

fn read_required_secret(
    source: &impl EnvironmentSource,
    name: &'static str,
) -> Result<String, ConfigError> {
    let direct = source.read(name);
    let file_name = format!("{name}_FILE");
    let file = source
        .read(&file_name)
        .filter(|value| !value.trim().is_empty());
    if direct.is_some() && file.is_some() {
        return Err(ConfigError::invalid(
            name,
            "不得同时设置值与对应的 _FILE 配置",
        ));
    }

    let value = match (direct, file) {
        (Some(value), None) => value,
        (None, Some(path)) => read_secret_file(name, &path)?,
        (None, None) => return Err(ConfigError::Missing { name }),
        (Some(_), Some(_)) => unreachable!("上方已拒绝歧义 Secret 来源"),
    };
    if value.trim().is_empty() || value.len() > MAX_TEXT_LENGTH || value.contains('\0') {
        return Err(ConfigError::invalid(name, "必须是非空且长度受限的值"));
    }
    Ok(value)
}

fn read_secret_file(name: &'static str, path: &str) -> Result<String, ConfigError> {
    validate_text(name, path)?;
    let value = fs::read_to_string(path).map_err(|_| ConfigError::SecretFileUnreadable { name })?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn read_required_u16(
    source: &impl EnvironmentSource,
    name: &'static str,
) -> Result<u16, ConfigError> {
    let value = read_required_text(source, name)?;
    let parsed = value
        .parse::<u16>()
        .map_err(|_| ConfigError::invalid(name, "必须是有效端口"))?;
    if parsed == 0 {
        return Err(ConfigError::invalid(name, "端口不能为零"));
    }
    Ok(parsed)
}

fn read_optional_u64(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    read_optional(source, name).map_or(Ok(default), |value| {
        value
            .parse::<u64>()
            .map_err(|_| ConfigError::invalid(name, "必须是正整数"))
    })
}

fn read_bounded_u64(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<u64, ConfigError> {
    let value = read_optional_u64(source, name, default)?;
    if !range.contains(&value) {
        return Err(ConfigError::invalid(name, "超出允许的安全范围"));
    }
    Ok(value)
}

fn read_bounded_u32(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u32,
    range: std::ops::RangeInclusive<u32>,
) -> Result<u32, ConfigError> {
    let value = read_optional(source, name).map_or(Ok(default), |value| {
        value
            .parse::<u32>()
            .map_err(|_| ConfigError::invalid(name, "必须是正整数"))
    })?;
    if !range.contains(&value) {
        return Err(ConfigError::invalid(name, "超出允许的安全范围"));
    }
    Ok(value)
}

fn read_bounded_u16(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u16,
    range: std::ops::RangeInclusive<u16>,
) -> Result<u16, ConfigError> {
    let value = read_optional(source, name).map_or(Ok(default), |value| {
        value
            .parse::<u16>()
            .map_err(|_| ConfigError::invalid(name, "必须是正整数"))
    })?;
    if !range.contains(&value) {
        return Err(ConfigError::invalid(name, "超出允许的安全范围"));
    }
    Ok(value)
}

fn read_agent_id(
    source: &impl EnvironmentSource,
    name: &'static str,
) -> Result<AgentId, ConfigError> {
    let value = Uuid::parse_str(&read_required_text(source, name)?)
        .map_err(|_| ConfigError::invalid(name, "必须是 UUIDv7"))?;
    if value.get_version() != Some(Version::SortRand) {
        return Err(ConfigError::invalid(name, "必须是 UUIDv7"));
    }
    Ok(AgentId::from_uuid(value))
}

fn read_bounded_duration(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<Duration, ConfigError> {
    let value = read_optional_u64(source, name, default)?;
    if !range.contains(&value) {
        return Err(ConfigError::invalid(name, "超出允许的安全范围"));
    }
    Ok(Duration::from_millis(value))
}

fn validate_text(name: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty() || value.len() > MAX_TEXT_LENGTH || value.chars().any(char::is_control) {
        return Err(ConfigError::invalid(name, "包含空值、控制字符或长度超限"));
    }
    Ok(())
}

fn parse_database_tls_mode(value: &str) -> Result<DatabaseTlsMode, ConfigError> {
    match value {
        "disable" => Ok(DatabaseTlsMode::Disable),
        "prefer" => Ok(DatabaseTlsMode::Prefer),
        "require" => Ok(DatabaseTlsMode::Require),
        "verify-ca" => Ok(DatabaseTlsMode::VerifyCertificate),
        "verify-full" => Ok(DatabaseTlsMode::VerifyIdentity),
        _ => Err(ConfigError::invalid(
            "AGENT_ROOM_DB_TLS_MODE",
            "仅支持 disable、prefer、require、verify-ca、verify-full",
        )),
    }
}

fn parse_http_url(name: &'static str, value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|_| ConfigError::invalid(name, "必须是绝对 URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ConfigError::invalid(
            name,
            "必须是无用户信息、查询参数和片段的 HTTP(S) URL",
        ));
    }
    Ok(url)
}

fn parse_origin(name: &'static str, value: &str) -> Result<Url, ConfigError> {
    let url = parse_http_url(name, value)?;
    if url.path() != "/" {
        return Err(ConfigError::invalid(name, "必须是无路径的 HTTP(S) Origin"));
    }
    Ok(url)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum ConfigError {
    #[error("缺少必需配置：{name}")]
    Missing { name: &'static str },
    #[error("配置 {name} 无效：{reason}")]
    Invalid {
        name: &'static str,
        reason: &'static str,
    },
    #[error("无法读取配置 {name} 指向的 Secret 文件")]
    SecretFileUnreadable { name: &'static str },
}

impl ConfigError {
    const fn invalid(name: &'static str, reason: &'static str) -> Self {
        Self::Invalid { name, reason }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ConfigError, ControlPlaneConfig, EnvironmentSource};

    #[derive(Default)]
    struct MapEnvironment(BTreeMap<&'static str, String>);

    impl EnvironmentSource for MapEnvironment {
        fn read(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn valid_environment() -> MapEnvironment {
        MapEnvironment(BTreeMap::from([
            ("AGENT_ROOM_DB_HOST", "127.0.0.1".to_owned()),
            ("AGENT_ROOM_DB_PORT", "55432".to_owned()),
            ("AGENT_ROOM_DB_NAME", "agent_room".to_owned()),
            ("AGENT_ROOM_DB_USER", "agent_room_runtime".to_owned()),
            ("AGENT_ROOM_DB_RUNTIME_PASSWORD", "secret-value".to_owned()),
            ("AGENT_ROOM_DB_TLS_MODE", "disable".to_owned()),
            (
                "AGENT_ROOM_MATRIX_BASE_URL",
                "http://127.0.0.1:18008".to_owned(),
            ),
            (
                "AGENT_ROOM_OBJECT_STORE_HEALTH_URL",
                "http://127.0.0.1:19333/cluster/status".to_owned(),
            ),
            (
                "AGENT_ROOM_OIDC_ISSUER_URL",
                "http://127.0.0.1:18080/realms/agent-room".to_owned(),
            ),
            ("AGENT_ROOM_OIDC_CLIENT_ID", "agent-room-web".to_owned()),
            (
                "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID",
                "agent-room-bridge".to_owned(),
            ),
            (
                "AGENT_ROOM_OIDC_CLIENT_SECRET",
                "local-client-secret".to_owned(),
            ),
            (
                "AGENT_ROOM_OIDC_REDIRECT_URL",
                "https://api.agent-room.localhost/auth/oidc/callback".to_owned(),
            ),
            (
                "AGENT_ROOM_FRONTEND_ORIGIN",
                "https://app.agent-room.localhost".to_owned(),
            ),
            (
                "AGENT_ROOM_MATRIX_SERVER_NAME",
                "matrix.agent-room.localhost".to_owned(),
            ),
            (
                "AGENT_ROOM_MATRIX_APPSERVICE_TOKEN",
                "local-application-service-token".to_owned(),
            ),
            (
                "AGENT_ROOM_CONTENT_S3_ENDPOINT",
                "http://127.0.0.1:18333".to_owned(),
            ),
            (
                "AGENT_ROOM_CONTENT_S3_BUCKET",
                "agent-room-content".to_owned(),
            ),
            ("AGENT_ROOM_CONTENT_S3_REGION", "us-east-1".to_owned()),
            (
                "AGENT_ROOM_CONTENT_S3_ACCESS_KEY",
                "local-content-access-key".to_owned(),
            ),
            (
                "AGENT_ROOM_CONTENT_S3_SECRET_KEY",
                "local-content-secret-key".to_owned(),
            ),
            (
                "AGENT_ROOM_CONTENT_SCANNER_ADDRESS",
                "127.0.0.1:13310".to_owned(),
            ),
            ("AGENT_ROOM_CONTENT_TICKET_KEY_ID", "local-v1".to_owned()),
            (
                "AGENT_ROOM_CONTENT_TICKET_SECRET",
                "local-content-ticket-secret-at-least-32-bytes".to_owned(),
            ),
            (
                "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID",
                "01991aaa-0000-7000-8000-000000000001".to_owned(),
            ),
        ]))
    }

    #[test]
    fn 启动配置不会在调试输出泄漏密码() {
        let config = ControlPlaneConfig::from_source(&valid_environment()).expect("配置有效");

        assert_eq!(
            format!("{:?}", config.dependencies.database.password),
            "[已脱敏]"
        );
        assert_eq!(format!("{:?}", config.content.ticket_secret), "[已脱敏]");
        assert_eq!(
            format!("{:?}", config.content.object_store_secret_key),
            "[已脱敏]"
        );
    }

    #[test]
    fn 缺少关键依赖配置时立即失败() {
        let mut environment = valid_environment();
        environment.0.remove("AGENT_ROOM_MATRIX_BASE_URL");

        assert!(matches!(
            ControlPlaneConfig::from_source(&environment),
            Err(ConfigError::Missing {
                name: "AGENT_ROOM_MATRIX_BASE_URL"
            })
        ));
    }

    #[test]
    fn 依赖地址不得夹带凭据() {
        let mut environment = valid_environment();
        environment.0.insert(
            "AGENT_ROOM_MATRIX_BASE_URL",
            "https://user:password@example.test".to_owned(),
        );

        assert!(matches!(
            ControlPlaneConfig::from_source(&environment),
            Err(ConfigError::Invalid {
                name: "AGENT_ROOM_MATRIX_BASE_URL",
                ..
            })
        ));
    }

    #[test]
    fn 内容授权身份必须是稳定的_uuidv7() {
        let mut environment = valid_environment();
        environment.0.insert(
            "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID",
            "550e8400-e29b-41d4-a716-446655440000".to_owned(),
        );

        assert!(matches!(
            ControlPlaneConfig::from_source(&environment),
            Err(ConfigError::Invalid {
                name: "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID",
                ..
            })
        ));
    }

    #[test]
    fn 内容预算越界时启动失败() {
        let mut environment = valid_environment();
        environment
            .0
            .insert("AGENT_ROOM_CONTENT_DOWNLOAD_MAX_BYTES", "1024".to_owned());

        assert!(matches!(
            ControlPlaneConfig::from_source(&environment),
            Err(ConfigError::Invalid {
                name: "AGENT_ROOM_CONTENT_DOWNLOAD_MAX_BYTES",
                ..
            })
        ));
    }

    #[test]
    fn 生产_secret_可以从只读文件加载() {
        let directory = tempfile::tempdir().expect("可创建临时目录");
        let path = directory.path().join("database-password");
        std::fs::write(&path, "file-backed-secret\n").expect("可写入测试 Secret");
        let mut environment = valid_environment();
        environment.0.remove("AGENT_ROOM_DB_RUNTIME_PASSWORD");
        environment.0.insert(
            "AGENT_ROOM_DB_RUNTIME_PASSWORD_FILE",
            path.to_string_lossy().into_owned(),
        );

        let config = ControlPlaneConfig::from_source(&environment).expect("文件 Secret 有效");

        assert_eq!(
            config.dependencies.database.password.expose(),
            "file-backed-secret"
        );
    }

    #[test]
    fn 同时设置_secret_值和文件时立即失败() {
        let mut environment = valid_environment();
        environment.0.insert(
            "AGENT_ROOM_DB_RUNTIME_PASSWORD_FILE",
            "C:/run/secrets/database-password".to_owned(),
        );

        assert!(matches!(
            ControlPlaneConfig::from_source(&environment),
            Err(ConfigError::Invalid {
                name: "AGENT_ROOM_DB_RUNTIME_PASSWORD",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn 有效配置可构建惰性依赖探针() {
        let config = ControlPlaneConfig::from_source(&valid_environment()).expect("配置有效");
        let runtime = crate::features::health::HealthRuntime::initialize(&config.dependencies)
            .expect("探针可在外部依赖不可用时完成惰性构建");

        runtime.shutdown().await;
    }
}
