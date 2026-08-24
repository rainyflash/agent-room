use std::{env, fmt, net::SocketAddr, time::Duration};

use thiserror::Error;
use url::Url;

const DEFAULT_DEPENDENCY_TIMEOUT_MILLIS: u64 = 2_000;
const DEFAULT_OTEL_EXPORT_TIMEOUT_MILLIS: u64 = 5_000;
const DEFAULT_LOGIN_ATTEMPT_TTL_MILLIS: u64 = 10 * 60 * 1_000;
const DEFAULT_WEB_SESSION_TTL_MILLIS: u64 = 8 * 60 * 60 * 1_000;
const DEFAULT_RECENT_AUTHENTICATION_MILLIS: u64 = 5 * 60 * 1_000;
const DEFAULT_CLOCK_SKEW_MILLIS: u64 = 60 * 1_000;
const MAX_TEXT_LENGTH: usize = 1_024;

trait EnvironmentSource {
    fn read(&self, name: &'static str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn read(&self, name: &'static str) -> Option<String> {
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
    pub(crate) redirect_url: Url,
    pub(crate) frontend_origin: Url,
    pub(crate) matrix_server_name: String,
    pub(crate) login_attempt_ttl: Duration,
    pub(crate) web_session_ttl: Duration,
    pub(crate) recent_authentication_window: Duration,
    pub(crate) allowed_clock_skew: Duration,
}

#[derive(Clone)]
pub(crate) struct ControlPlaneConfig {
    pub(crate) bind_address: SocketAddr,
    pub(crate) dependencies: DependencyConfig,
    pub(crate) authentication: AuthenticationConfig,
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
            observability: read_observability_config(source)?,
        })
    }
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
    let value = source.read(name).ok_or(ConfigError::Missing { name })?;
    if value.trim().is_empty() || value.len() > MAX_TEXT_LENGTH || value.contains('\0') {
        return Err(ConfigError::invalid(name, "必须是非空且长度受限的值"));
    }
    Ok(value)
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
        fn read(&self, name: &'static str) -> Option<String> {
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
        ]))
    }

    #[test]
    fn 启动配置不会在调试输出泄漏密码() {
        let config = ControlPlaneConfig::from_source(&valid_environment()).expect("配置有效");

        assert_eq!(
            format!("{:?}", config.dependencies.database.password),
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

    #[tokio::test]
    async fn 有效配置可构建惰性依赖探针() {
        let config = ControlPlaneConfig::from_source(&valid_environment()).expect("配置有效");
        let runtime = crate::features::health::HealthRuntime::initialize(&config.dependencies)
            .expect("探针可在外部依赖不可用时完成惰性构建");

        runtime.shutdown().await;
    }
}
