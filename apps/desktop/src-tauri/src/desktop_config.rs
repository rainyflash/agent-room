use std::{collections::BTreeMap, env, path::PathBuf};

use agent_room_bridge_local_adapter::{
    SecureStorageService, bridge_data_root_from_environment, bridge_runtime_root,
    secure_storage_service_from_environment,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::runtime_target::DesktopAgentTarget;

const DEFAULT_CONTROL_PLANE_URL: &str = "https://api.room.the-zeroth.com/";
const DEFAULT_MATRIX_BASE_URL: &str = "https://matrix.room.the-zeroth.com";
const DEFAULT_OIDC_ISSUER_URL: &str = "https://id.room.the-zeroth.com/realms/agent-room";
const DEFAULT_OIDC_DEVICE_CLIENT_ID: &str = "agent-room-bridge";

#[derive(Debug, Clone)]
pub(crate) struct DesktopBridgeConfig {
    runtime_root: PathBuf,
    secure_storage_service: SecureStorageService,
    control_plane_url: Url,
    environment: BTreeMap<String, String>,
}

impl DesktopBridgeConfig {
    pub(crate) fn from_environment() -> Result<Self, DesktopConfigFailure> {
        let data_root = bridge_data_root_from_environment()
            .map_err(|_| DesktopConfigFailure::new("desktop.config.bridge_data_root_invalid"))?;
        let secure_storage_service = secure_storage_service_from_environment()
            .map_err(|_| DesktopConfigFailure::new("desktop.config.secure_storage_invalid"))?;
        let mut environment = BTreeMap::new();
        let control_plane_url = Url::parse(&validated_url(
            "AGENT_ROOM_CONTROL_PLANE_URL",
            DEFAULT_CONTROL_PLANE_URL,
        )?)
        .map_err(|_| DesktopConfigFailure::new("desktop.config.service_url_invalid"))?;
        environment.insert(
            "AGENT_ROOM_CONTROL_PLANE_URL".to_owned(),
            control_plane_url.to_string(),
        );
        environment.insert(
            "AGENT_ROOM_MATRIX_BASE_URL".to_owned(),
            validated_url("AGENT_ROOM_MATRIX_BASE_URL", DEFAULT_MATRIX_BASE_URL)?,
        );
        environment.insert(
            "AGENT_ROOM_OIDC_ISSUER_URL".to_owned(),
            validated_url("AGENT_ROOM_OIDC_ISSUER_URL", DEFAULT_OIDC_ISSUER_URL)?,
        );
        environment.insert(
            "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID".to_owned(),
            bounded_environment_value(
                "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID",
                DEFAULT_OIDC_DEVICE_CLIENT_ID,
            )?,
        );
        environment.insert(
            "AGENT_ROOM_BRIDGE_DATA_DIR".to_owned(),
            data_root.to_string_lossy().into_owned(),
        );
        environment.insert(
            "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE".to_owned(),
            secure_storage_service.as_str().to_owned(),
        );
        environment.insert(
            "AGENT_ROOM_BRIDGE_DEVICE_LABEL".to_owned(),
            default_device_label().to_owned(),
        );
        environment.insert("AGENT_ROOM_BRIDGE_SUPERVISED".to_owned(), "true".to_owned());
        copy_optional_environment(&mut environment, "AGENT_ROOM_AGENT_ID")?;
        copy_optional_environment(&mut environment, "AGENT_ROOM_PUBLIC_LOBBY_CATALOG_ID")?;
        copy_optional_environment(&mut environment, "AGENT_ROOM_LOBBY_LANGUAGE")?;
        copy_optional_environment(&mut environment, "AGENT_ROOM_LOBBY_REGION")?;
        Ok(Self {
            runtime_root: bridge_runtime_root(&data_root),
            environment,
            secure_storage_service,
            control_plane_url,
        })
    }

    pub(crate) fn with_agent_target(mut self, target: &DesktopAgentTarget) -> Self {
        self.environment.insert(
            "AGENT_ROOM_AGENT_ID".to_owned(),
            target.agent_id().to_owned(),
        );
        self.environment.insert(
            "AGENT_ROOM_PUBLIC_LOBBY_CATALOG_ID".to_owned(),
            target.public_lobby_catalog_id().to_owned(),
        );
        self.environment.insert(
            "AGENT_ROOM_LOBBY_LANGUAGE".to_owned(),
            target.lobby_language().to_owned(),
        );
        self
    }

    pub(crate) fn data_root(&self) -> PathBuf {
        self.runtime_root
            .parent()
            .map_or_else(|| self.runtime_root.clone(), PathBuf::from)
    }

    pub(crate) fn runtime_root(&self) -> PathBuf {
        self.runtime_root.clone()
    }

    pub(crate) const fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    pub(crate) fn secure_storage_service(&self) -> SecureStorageService {
        self.secure_storage_service.clone()
    }

    pub(crate) fn control_plane_url(&self) -> Url {
        self.control_plane_url.clone()
    }

    /// 人类云端会话必须与 Bridge/Agent 凭据物理隔离，同时保留测试安装的命名空间隔离。
    pub(crate) fn human_session_storage_service(&self) -> String {
        let digest = URL_SAFE_NO_PAD.encode(Sha256::digest(
            self.secure_storage_service.as_str().as_bytes(),
        ));
        format!("dev.agent-room.desktop-human.{}", &digest[..22])
    }
}

fn validated_url(
    name: &'static str,
    default: &'static str,
) -> Result<String, DesktopConfigFailure> {
    let value = bounded_environment_value(name, default)?;
    let parsed = Url::parse(&value)
        .map_err(|_| DesktopConfigFailure::new("desktop.config.service_url_invalid"))?;
    let loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if (parsed.scheme() != "https" && !loopback_http)
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return Err(DesktopConfigFailure::new(
            "desktop.config.service_url_invalid",
        ));
    }
    Ok(value)
}

fn bounded_environment_value(
    name: &'static str,
    default: &'static str,
) -> Result<String, DesktopConfigFailure> {
    let value = env::var(name).unwrap_or_else(|_| default.to_owned());
    let value = value.trim();
    if value.is_empty() || value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(DesktopConfigFailure::new(
            "desktop.config.environment_value_invalid",
        ));
    }
    Ok(value.to_owned())
}

fn copy_optional_environment(
    target: &mut BTreeMap<String, String>,
    name: &'static str,
) -> Result<(), DesktopConfigFailure> {
    let Ok(value) = env::var(name) else {
        return Ok(());
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(DesktopConfigFailure::new(
            "desktop.config.environment_value_invalid",
        ));
    }
    target.insert(name.to_owned(), value.to_owned());
    Ok(())
}

const fn default_device_label() -> &'static str {
    #[cfg(target_os = "windows")]
    return "Agent Room desktop · Windows";
    #[cfg(target_os = "macos")]
    return "Agent Room desktop · macOS";
    #[cfg(target_os = "linux")]
    return "Agent Room desktop · Linux";
    #[allow(unreachable_code)]
    "Agent Room desktop"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DesktopConfigFailure {
    code: &'static str,
}

impl DesktopConfigFailure {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }
}

#[cfg(test)]
mod tests {
    use super::validated_url;

    #[test]
    fn 服务地址只接受_https_或本机_http() {
        assert!(validated_url("AGENT_ROOM_TEST_MISSING", "https://room.example/path").is_ok());
        assert!(validated_url("AGENT_ROOM_TEST_MISSING", "http://127.0.0.1:8090").is_ok());
        assert!(validated_url("AGENT_ROOM_TEST_MISSING", "http://room.example").is_err());
        assert!(validated_url("AGENT_ROOM_TEST_MISSING", "https://user@room.example").is_err());
    }
}
