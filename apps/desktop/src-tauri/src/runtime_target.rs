use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use uuid::{Uuid, Version};

const SETTINGS_SCHEMA_VERSION: u8 = 1;
const SETTINGS_DIRECTORY: &str = "desktop";
const SETTINGS_FILENAME: &str = "agent-target.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopAgentTarget {
    agent_id: String,
    public_lobby_catalog_id: String,
    lobby_language: String,
}

impl DesktopAgentTarget {
    pub(crate) fn new(
        agent_id: &str,
        public_lobby_catalog_id: &str,
        lobby_language: &str,
    ) -> Result<Self, RuntimeTargetFailure> {
        validate_uuid_v7(agent_id)?;
        validate_uuid_v7(public_lobby_catalog_id)?;
        if lobby_language.chars().any(char::is_control) {
            return Err(RuntimeTargetFailure::new(
                "desktop.agent_target.language_invalid",
                false,
            ));
        }
        let lobby_language = lobby_language.trim();
        if lobby_language.is_empty() || lobby_language.len() > 35 {
            return Err(RuntimeTargetFailure::new(
                "desktop.agent_target.language_invalid",
                false,
            ));
        }
        Ok(Self {
            agent_id: agent_id.to_owned(),
            public_lobby_catalog_id: public_lobby_catalog_id.to_owned(),
            lobby_language: lobby_language.to_owned(),
        })
    }

    pub(crate) fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub(crate) fn public_lobby_catalog_id(&self) -> &str {
        &self.public_lobby_catalog_id
    }

    pub(crate) fn lobby_language(&self) -> &str {
        &self.lobby_language
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedRuntimeTarget {
    schema_version: u8,
    target: DesktopAgentTarget,
}

pub(crate) struct RuntimeTargetStore {
    path: PathBuf,
    current: Mutex<Option<DesktopAgentTarget>>,
}

impl RuntimeTargetStore {
    pub(crate) fn open(data_root: &Path) -> Result<Self, RuntimeTargetFailure> {
        let path = data_root.join(SETTINGS_DIRECTORY).join(SETTINGS_FILENAME);
        let current = load_target(&path)?;
        Ok(Self {
            path,
            current: Mutex::new(current),
        })
    }

    pub(crate) fn current(&self) -> Result<Option<DesktopAgentTarget>, RuntimeTargetFailure> {
        self.current
            .lock()
            .map(|target| target.clone())
            .map_err(|_| RuntimeTargetFailure::new("desktop.agent_target.state_unavailable", true))
    }

    pub(crate) fn persist(&self, target: DesktopAgentTarget) -> Result<(), RuntimeTargetFailure> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| RuntimeTargetFailure::new("desktop.agent_target.path_invalid", false))?;
        fs::create_dir_all(parent).map_err(|_| {
            RuntimeTargetFailure::new("desktop.agent_target.directory_unavailable", true)
        })?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|_| {
            RuntimeTargetFailure::new("desktop.agent_target.write_unavailable", true)
        })?;
        serde_json::to_writer_pretty(
            temporary.as_file_mut(),
            &PersistedRuntimeTarget {
                schema_version: SETTINGS_SCHEMA_VERSION,
                target: target.clone(),
            },
        )
        .map_err(|_| RuntimeTargetFailure::new("desktop.agent_target.serialize_failed", false))?;
        temporary.as_file_mut().sync_all().map_err(|_| {
            RuntimeTargetFailure::new("desktop.agent_target.write_unavailable", true)
        })?;
        temporary.persist(&self.path).map_err(|_| {
            RuntimeTargetFailure::new("desktop.agent_target.replace_unavailable", true)
        })?;
        *self.current.lock().map_err(|_| {
            RuntimeTargetFailure::new("desktop.agent_target.state_unavailable", true)
        })? = Some(target);
        Ok(())
    }
}

fn load_target(path: &Path) -> Result<Option<DesktopAgentTarget>, RuntimeTargetFailure> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(RuntimeTargetFailure::new(
                "desktop.agent_target.read_unavailable",
                true,
            ));
        }
    };
    if bytes.len() > 16 * 1_024 {
        return Err(RuntimeTargetFailure::new(
            "desktop.agent_target.document_invalid",
            false,
        ));
    }
    let persisted: PersistedRuntimeTarget = serde_json::from_slice(&bytes)
        .map_err(|_| RuntimeTargetFailure::new("desktop.agent_target.document_invalid", false))?;
    if persisted.schema_version != SETTINGS_SCHEMA_VERSION {
        return Err(RuntimeTargetFailure::new(
            "desktop.agent_target.schema_unsupported",
            false,
        ));
    }
    DesktopAgentTarget::new(
        persisted.target.agent_id(),
        persisted.target.public_lobby_catalog_id(),
        persisted.target.lobby_language(),
    )
    .map(Some)
}

fn validate_uuid_v7(value: &str) -> Result<(), RuntimeTargetFailure> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| RuntimeTargetFailure::new("desktop.agent_target.identifier_invalid", false))?;
    if parsed.get_version() != Some(Version::SortRand) {
        return Err(RuntimeTargetFailure::new(
            "desktop.agent_target.identifier_invalid",
            false,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeTargetFailure {
    code: &'static str,
    retryable: bool,
}

impl RuntimeTargetFailure {
    const fn new(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }

    pub(crate) const fn retryable(self) -> bool {
        self.retryable
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{DesktopAgentTarget, RuntimeTargetStore};

    const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const LOBBY_ID: &str = "0198b601-77a2-7f41-b4f4-940f291951b8";

    #[test]
    fn 目标配置原子持久化并可恢复() {
        let root = tempdir().expect("临时目录有效");
        let store = RuntimeTargetStore::open(root.path()).expect("目标存储可初始化");
        assert!(store.current().expect("目标状态可读取").is_none());

        let target = DesktopAgentTarget::new(AGENT_ID, LOBBY_ID, "zh-CN").expect("有效目标可创建");
        store.persist(target.clone()).expect("目标可持久化");

        let restored = RuntimeTargetStore::open(root.path())
            .expect("目标存储可重开")
            .current()
            .expect("目标状态可读取");
        assert_eq!(restored, Some(target));
    }

    #[test]
    fn 目标配置拒绝非_uuidv7_和控制字符语言() {
        assert!(
            DesktopAgentTarget::new("00000000-0000-0000-0000-000000000001", LOBBY_ID, "en")
                .is_err()
        );
        assert!(DesktopAgentTarget::new(AGENT_ID, LOBBY_ID, "en\n").is_err());
    }
}
