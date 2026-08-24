use std::path::{Path, PathBuf};

const MAX_PATH_TEXT_LENGTH: usize = 1_024;

/// 从当前进程环境解析 Bridge 数据根目录。
///
/// # Errors
///
/// 显式路径或平台默认根目录缺失或无效时返回错误。
pub fn bridge_data_root_from_environment() -> Result<PathBuf, BridgeLocationFailure> {
    resolve_bridge_data_root(|name| std::env::var(name).ok())
}

/// 使 Bridge、桌面壳和 MCP 从同一组环境值解析唯一数据根目录。
///
/// # Errors
///
/// 显式路径或平台默认根目录缺失、非绝对路径或包含不安全文本时返回错误。
pub fn resolve_bridge_data_root(
    mut read: impl FnMut(&'static str) -> Option<String>,
) -> Result<PathBuf, BridgeLocationFailure> {
    if let Some(explicit) = present(read("AGENT_ROOM_BRIDGE_DATA_DIR")) {
        return validate_root("AGENT_ROOM_BRIDGE_DATA_DIR", &explicit);
    }
    default_data_root(&mut read)
}

pub fn bridge_runtime_root(data_root: &Path) -> PathBuf {
    data_root.join("runtime")
}

#[cfg(target_os = "windows")]
fn default_data_root(
    read: &mut impl FnMut(&'static str) -> Option<String>,
) -> Result<PathBuf, BridgeLocationFailure> {
    let root = present(read("LOCALAPPDATA")).ok_or_else(|| missing("LOCALAPPDATA"))?;
    validate_root("LOCALAPPDATA", &root).map(|root| root.join("AgentRoom").join("Bridge"))
}

#[cfg(target_os = "macos")]
fn default_data_root(
    read: &mut impl FnMut(&'static str) -> Option<String>,
) -> Result<PathBuf, BridgeLocationFailure> {
    let root = present(read("HOME")).ok_or_else(|| missing("HOME"))?;
    validate_root("HOME", &root).map(|root| {
        root.join("Library")
            .join("Application Support")
            .join("Agent Room")
            .join("Bridge")
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_data_root(
    read: &mut impl FnMut(&'static str) -> Option<String>,
) -> Result<PathBuf, BridgeLocationFailure> {
    if let Some(root) = present(read("XDG_DATA_HOME")) {
        return validate_root("XDG_DATA_HOME", &root).map(|root| root.join("agent-room/bridge"));
    }
    let root = present(read("HOME")).ok_or_else(|| missing("HOME"))?;
    validate_root("HOME", &root).map(|root| root.join(".local/share/agent-room/bridge"))
}

#[cfg(not(any(windows, unix)))]
fn default_data_root(
    _read: &mut impl FnMut(&'static str) -> Option<String>,
) -> Result<PathBuf, BridgeLocationFailure> {
    Err(missing("AGENT_ROOM_BRIDGE_DATA_DIR"))
}

fn present(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn validate_root(variable: &'static str, value: &str) -> Result<PathBuf, BridgeLocationFailure> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PATH_TEXT_LENGTH
        || value.chars().any(char::is_control)
        || !Path::new(value).is_absolute()
    {
        return Err(invalid(variable));
    }
    Ok(PathBuf::from(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLocationFailureKind {
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeLocationFailure {
    variable: &'static str,
    kind: BridgeLocationFailureKind,
}

impl BridgeLocationFailure {
    pub const fn variable(self) -> &'static str {
        self.variable
    }

    pub const fn kind(self) -> BridgeLocationFailureKind {
        self.kind
    }
}

const fn missing(variable: &'static str) -> BridgeLocationFailure {
    BridgeLocationFailure {
        variable,
        kind: BridgeLocationFailureKind::Missing,
    }
}

const fn invalid(variable: &'static str) -> BridgeLocationFailure {
    BridgeLocationFailure {
        variable,
        kind: BridgeLocationFailureKind::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{BridgeLocationFailureKind, bridge_runtime_root, resolve_bridge_data_root};

    #[test]
    fn 显式数据目录是所有本地进程的唯一事实源() {
        let explicit = std::env::temp_dir().join("agent-room-location-test");
        let values = BTreeMap::from([(
            "AGENT_ROOM_BRIDGE_DATA_DIR",
            explicit.to_string_lossy().into_owned(),
        )]);

        let resolved =
            resolve_bridge_data_root(|name| values.get(name).cloned()).expect("显式绝对路径有效");

        assert_eq!(resolved, explicit);
        assert_eq!(bridge_runtime_root(&resolved), resolved.join("runtime"));
    }

    #[test]
    fn 相对路径不能导致_bridge_与_mcp_连到不同端点() {
        let failure = resolve_bridge_data_root(|name| {
            (name == "AGENT_ROOM_BRIDGE_DATA_DIR").then(|| "relative/path".to_owned())
        })
        .expect_err("相对路径必须失败");

        assert_eq!(failure.kind(), BridgeLocationFailureKind::Invalid);
        assert_eq!(failure.variable(), "AGENT_ROOM_BRIDGE_DATA_DIR");
    }
}
