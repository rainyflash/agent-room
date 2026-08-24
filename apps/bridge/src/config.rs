use std::{env, path::PathBuf, time::Duration};

use agent_room_bridge_local_adapter::{BridgeLocationFailureKind, resolve_bridge_data_root};
use agent_room_domain::ids::AgentId;
use uuid::{Uuid, Version};

const DEFAULT_REQUEST_TIMEOUT_MILLIS: u64 = 10_000;
const DEFAULT_AUTHORIZATION_TIMEOUT_MILLIS: u64 = 10 * 60 * 1_000;
const DEFAULT_REFRESH_LEAD_MILLIS: u64 = 2 * 60 * 1_000;
const DEFAULT_RECONNECT_INITIAL_MILLIS: u64 = 1_000;
const DEFAULT_RECONNECT_MAXIMUM_MILLIS: u64 = 60_000;
const MAX_TEXT_LENGTH: usize = 1_024;
const MAX_DEVICE_LABEL_LENGTH: usize = 128;

trait EnvironmentSource {
    fn read(&self, name: &'static str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn read(&self, name: &'static str) -> Option<String> {
        env::var(name).ok()
    }
}

pub(crate) struct BridgeConfig {
    pub(crate) agent_id: Option<AgentId>,
    pub(crate) control_plane_url: String,
    pub(crate) matrix_homeserver_url: String,
    pub(crate) oidc_issuer_url: String,
    pub(crate) oidc_client_id: String,
    pub(crate) device_label: String,
    pub(crate) request_timeout: Duration,
    pub(crate) authorization_timeout: Duration,
    pub(crate) refresh_lead_time: Duration,
    pub(crate) reconnect_initial_delay: Duration,
    pub(crate) reconnect_maximum_delay: Duration,
    pub(crate) import_oidc_profile: bool,
    pub(crate) data_root: PathBuf,
}

impl BridgeConfig {
    pub(crate) fn from_environment() -> Result<Self, BridgeConfigError> {
        Self::from_source(&ProcessEnvironment)
    }

    fn from_source(source: &impl EnvironmentSource) -> Result<Self, BridgeConfigError> {
        let reconnect_initial_delay = read_bounded_duration(
            source,
            "AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS",
            DEFAULT_RECONNECT_INITIAL_MILLIS,
            100..=60_000,
        )?;
        let reconnect_maximum_delay = read_bounded_duration(
            source,
            "AGENT_ROOM_BRIDGE_RECONNECT_MAXIMUM_MS",
            DEFAULT_RECONNECT_MAXIMUM_MILLIS,
            1_000..=15 * 60 * 1_000,
        )?;
        if reconnect_initial_delay > reconnect_maximum_delay {
            return Err(BridgeConfigError::invalid(
                "AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS",
                "不能大于最大重连延迟",
            ));
        }
        Ok(Self {
            agent_id: read_optional_agent_id(source)?,
            control_plane_url: read_required_text(source, "AGENT_ROOM_CONTROL_PLANE_URL")?,
            matrix_homeserver_url: read_required_text(source, "AGENT_ROOM_MATRIX_BASE_URL")?,
            oidc_issuer_url: read_required_text(source, "AGENT_ROOM_OIDC_ISSUER_URL")?,
            oidc_client_id: read_required_text(source, "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID")?,
            device_label: read_device_label(source)?,
            request_timeout: read_bounded_duration(
                source,
                "AGENT_ROOM_BRIDGE_REQUEST_TIMEOUT_MS",
                DEFAULT_REQUEST_TIMEOUT_MILLIS,
                100..=30_000,
            )?,
            authorization_timeout: read_bounded_duration(
                source,
                "AGENT_ROOM_BRIDGE_AUTHORIZATION_TIMEOUT_MS",
                DEFAULT_AUTHORIZATION_TIMEOUT_MILLIS,
                5 * 60 * 1_000..=30 * 60 * 1_000,
            )?,
            refresh_lead_time: read_bounded_duration(
                source,
                "AGENT_ROOM_BRIDGE_REFRESH_LEAD_MS",
                DEFAULT_REFRESH_LEAD_MILLIS,
                30_000..=10 * 60 * 1_000,
            )?,
            reconnect_initial_delay,
            reconnect_maximum_delay,
            import_oidc_profile: read_bool(source, "AGENT_ROOM_BRIDGE_IMPORT_OIDC_PROFILE", false)?,
            data_root: read_data_root(source)?,
        })
    }
}

fn read_optional_agent_id(
    source: &impl EnvironmentSource,
) -> Result<Option<AgentId>, BridgeConfigError> {
    let Some(value) = read_optional(source, "AGENT_ROOM_AGENT_ID") else {
        return Ok(None);
    };
    let id = Uuid::parse_str(value.trim()).map_err(|_| {
        BridgeConfigError::invalid("AGENT_ROOM_AGENT_ID", "必须是控制面签发的 UUIDv7")
    })?;
    if id.get_version() != Some(Version::SortRand) {
        return Err(BridgeConfigError::invalid(
            "AGENT_ROOM_AGENT_ID",
            "必须是控制面签发的 UUIDv7",
        ));
    }
    Ok(Some(AgentId::from_uuid(id)))
}

fn read_data_root(source: &impl EnvironmentSource) -> Result<PathBuf, BridgeConfigError> {
    resolve_bridge_data_root(|name| source.read(name)).map_err(|failure| match failure.kind() {
        BridgeLocationFailureKind::Missing => BridgeConfigError::Missing {
            name: failure.variable(),
        },
        BridgeLocationFailureKind::Invalid => {
            BridgeConfigError::invalid(failure.variable(), "必须是安全的绝对路径")
        }
    })
}

fn read_device_label(source: &impl EnvironmentSource) -> Result<String, BridgeConfigError> {
    let label = read_optional(source, "AGENT_ROOM_BRIDGE_DEVICE_LABEL")
        .map_or_else(default_device_label, |value| value.trim().to_owned());
    if label.is_empty()
        || label.len() > MAX_DEVICE_LABEL_LENGTH
        || label.chars().any(char::is_control)
    {
        return Err(BridgeConfigError::invalid(
            "AGENT_ROOM_BRIDGE_DEVICE_LABEL",
            "必须是非空、长度不超过 128 且不含控制字符的名称",
        ));
    }
    Ok(label)
}

fn default_device_label() -> String {
    #[cfg(target_os = "windows")]
    return "Windows 设备".to_owned();
    #[cfg(target_os = "macos")]
    return "macOS 设备".to_owned();
    #[cfg(target_os = "linux")]
    return "Linux 设备".to_owned();
    #[allow(unreachable_code)]
    "Agent Room 设备".to_owned()
}

fn read_optional(source: &impl EnvironmentSource, name: &'static str) -> Option<String> {
    source.read(name).filter(|value| !value.trim().is_empty())
}

fn read_required_text(
    source: &impl EnvironmentSource,
    name: &'static str,
) -> Result<String, BridgeConfigError> {
    let value = read_optional(source, name).ok_or(BridgeConfigError::Missing { name })?;
    let value = value.trim().to_owned();
    if value.len() > MAX_TEXT_LENGTH || value.chars().any(char::is_control) {
        return Err(BridgeConfigError::invalid(name, "长度超限或包含控制字符"));
    }
    Ok(value)
}

fn read_bool(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: bool,
) -> Result<bool, BridgeConfigError> {
    read_optional(source, name).map_or(Ok(default), |value| match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(BridgeConfigError::invalid(name, "仅支持 true 或 false")),
    })
}

fn read_bounded_duration(
    source: &impl EnvironmentSource,
    name: &'static str,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<Duration, BridgeConfigError> {
    let value = read_optional(source, name).map_or(Ok(default), |value| {
        value
            .parse::<u64>()
            .map_err(|_| BridgeConfigError::invalid(name, "必须是正整数毫秒数"))
    })?;
    if !range.contains(&value) {
        return Err(BridgeConfigError::invalid(name, "超出允许的安全范围"));
    }
    Ok(Duration::from_millis(value))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BridgeConfigError {
    Missing {
        name: &'static str,
    },
    Invalid {
        name: &'static str,
        reason: &'static str,
    },
}

impl BridgeConfigError {
    const fn invalid(name: &'static str, reason: &'static str) -> Self {
        Self::Invalid { name, reason }
    }
}

impl std::fmt::Display for BridgeConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { name } => write!(formatter, "缺少必需配置：{name}"),
            Self::Invalid { name, reason } => write!(formatter, "配置 {name} 无效：{reason}"),
        }
    }
}

impl std::error::Error for BridgeConfigError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{BridgeConfig, BridgeConfigError, EnvironmentSource};

    #[derive(Default)]
    struct MapEnvironment(BTreeMap<&'static str, String>);

    impl EnvironmentSource for MapEnvironment {
        fn read(&self, name: &'static str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn valid_environment() -> MapEnvironment {
        MapEnvironment(BTreeMap::from([
            (
                "AGENT_ROOM_CONTROL_PLANE_URL",
                "http://127.0.0.1:8090/".to_owned(),
            ),
            (
                "AGENT_ROOM_OIDC_ISSUER_URL",
                "http://127.0.0.1:18080/realms/agent-room".to_owned(),
            ),
            (
                "AGENT_ROOM_MATRIX_BASE_URL",
                "http://127.0.0.1:18008".to_owned(),
            ),
            (
                "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID",
                "agent-room-bridge".to_owned(),
            ),
            (
                "AGENT_ROOM_BRIDGE_DATA_DIR",
                std::env::temp_dir()
                    .join("agent-room-bridge-config-test")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]))
    }

    #[test]
    fn 最小配置使用安全默认值且不导入第三方资料() {
        let config = BridgeConfig::from_source(&valid_environment()).expect("最小配置有效");

        assert_eq!(config.request_timeout.as_millis(), 10_000);
        assert_eq!(config.authorization_timeout.as_millis(), 600_000);
        assert_eq!(config.refresh_lead_time.as_millis(), 120_000);
        assert_eq!(config.reconnect_initial_delay.as_millis(), 1_000);
        assert_eq!(config.reconnect_maximum_delay.as_millis(), 60_000);
        assert!(!config.import_oidc_profile);
        assert!(config.agent_id.is_none());
        assert!(!config.device_label.is_empty());
        assert!(config.data_root.is_absolute());
    }

    #[test]
    fn 缺少控制面地址时立即失败() {
        let mut environment = valid_environment();
        environment.0.remove("AGENT_ROOM_CONTROL_PLANE_URL");

        assert!(matches!(
            BridgeConfig::from_source(&environment),
            Err(BridgeConfigError::Missing {
                name: "AGENT_ROOM_CONTROL_PLANE_URL"
            })
        ));
    }

    #[test]
    fn 资料导入必须由显式布尔配置开启() {
        let mut environment = valid_environment();
        environment
            .0
            .insert("AGENT_ROOM_BRIDGE_IMPORT_OIDC_PROFILE", "yes".to_owned());

        assert!(matches!(
            BridgeConfig::from_source(&environment),
            Err(BridgeConfigError::Invalid {
                name: "AGENT_ROOM_BRIDGE_IMPORT_OIDC_PROFILE",
                ..
            })
        ));
    }

    #[test]
    fn 数据目录拒绝相对路径() {
        let mut environment = valid_environment();
        environment
            .0
            .insert("AGENT_ROOM_BRIDGE_DATA_DIR", "relative/path".to_owned());

        assert!(matches!(
            BridgeConfig::from_source(&environment),
            Err(BridgeConfigError::Invalid {
                name: "AGENT_ROOM_BRIDGE_DATA_DIR",
                ..
            })
        ));
    }

    #[test]
    fn 初始重连延迟不能大于封顶值() {
        let mut environment = valid_environment();
        environment
            .0
            .insert("AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS", "60000".to_owned());
        environment
            .0
            .insert("AGENT_ROOM_BRIDGE_RECONNECT_MAXIMUM_MS", "1000".to_owned());

        assert!(matches!(
            BridgeConfig::from_source(&environment),
            Err(BridgeConfigError::Invalid {
                name: "AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS",
                ..
            })
        ));
    }

    #[test]
    fn agent_标识必须是_uuidv7() {
        let mut environment = valid_environment();
        environment.0.insert(
            "AGENT_ROOM_AGENT_ID",
            "00000000-0000-0000-0000-000000000001".to_owned(),
        );

        assert!(matches!(
            BridgeConfig::from_source(&environment),
            Err(BridgeConfigError::Invalid {
                name: "AGENT_ROOM_AGENT_ID",
                ..
            })
        ));

        environment.0.insert(
            "AGENT_ROOM_AGENT_ID",
            "0198b601-77a1-7bb8-83eb-a8fe68c97e44".to_owned(),
        );
        assert!(
            BridgeConfig::from_source(&environment)
                .expect("UUIDv7 Agent 标识有效")
                .agent_id
                .is_some()
        );
    }
}
