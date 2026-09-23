//! Task-local identity and acknowledged inbox progress. No login or Matrix secrets are stored here.
use crate::output::{CliFailure as Failure, CliResult as Result};
use agent_room_bridge_ipc::{IpcMethod, IpcOpenHostSessionRequest};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Invitation {
    pub(crate) version: u8,
    pub(crate) session_key: String,
    pub(crate) display_name: String,
    pub(crate) room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) catalog_id: Option<String>,
}

impl Invitation {
    pub(crate) fn decode(encoded: &str) -> Result<Self> {
        if encoded.len() > 4096 {
            return Err(Failure::validation("cli.invitation_invalid"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| Failure::validation("cli.invitation_invalid"))?;
        let invite: Self = serde_json::from_slice(&bytes)
            .map_err(|_| Failure::validation("cli.invitation_invalid"))?;
        invite.validate()?;
        Ok(invite)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Failure::validation("cli.invitation_version_unsupported"));
        }
        // 只有 Matrix 房间没有目录的旧邀请仍然有效：进默认大厅，连上后核对房间是否一致。
        IpcMethod::OpenHostSession(self.request())
            .validate()
            .map_err(|_| Failure::validation("cli.invitation_invalid"))?;
        if let Some(room) = &self.room_id
            && (room.len() > 512
                || !room.starts_with('!')
                || !room.contains(':')
                || room.chars().any(char::is_whitespace)
                || room.chars().any(char::is_control))
        {
            return Err(Failure::validation("cli.invitation_room_invalid"));
        }
        Ok(())
    }

    /// 按名字接入时本机合成的邀请：与应用里复制出来的邀请同构，后续命令看不出区别。
    pub(crate) fn for_room(session_key: String, display_name: String, target: &RoomTarget) -> Self {
        Self {
            version: 1,
            session_key,
            display_name,
            room_id: target.room_id.clone(),
            catalog_id: target.catalog_id.clone(),
        }
    }

    pub(crate) fn request(&self) -> IpcOpenHostSessionRequest {
        IpcOpenHostSessionRequest {
            session_key: self.session_key.clone(),
            display_name: self.display_name.clone(),
            room: self.catalog_id.as_ref().map(|catalog| {
                agent_room_bridge_ipc::IpcHostRoomTarget {
                    catalog_id: catalog.clone(),
                    room_id: self.room_id.clone(),
                }
            }),
        }
    }
}

/// `join --room` 解析出的目标房间；默认公开大厅两项皆空，由 Bridge 选择。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomTarget {
    pub(crate) catalog_id: Option<String>,
    pub(crate) room_id: Option<String>,
}

impl RoomTarget {
    pub(crate) const DEFAULT_LOBBY: Self = Self {
        catalog_id: None,
        room_id: None,
    };

    /// 同一目录条目就是同一个房间；Matrix 房间实例变了由连接时的房间核对来报。
    pub(crate) fn matches(&self, invitation: &Invitation) -> bool {
        invitation.catalog_id == self.catalog_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profile {
    pub(crate) invitation: Invitation,
    pub(crate) bridge_service: String,
    pub(crate) task_id: Option<String>,
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    pub(crate) room_id: Option<String>,
    pub(crate) after_event_id: Option<String>,
    pub(crate) delivered: Vec<String>,
}

impl Profile {
    pub(crate) fn new(invitation: Invitation, service: &str, task_id: Option<String>) -> Self {
        Self {
            invitation,
            bridge_service: service.into(),
            task_id,
            session_id: None,
            agent_id: None,
            room_id: None,
            after_event_id: None,
            delivered: Vec::new(),
        }
    }

    pub(crate) fn validate_binding(&self, service: &str, task_id: Option<&str>) -> Result<()> {
        if self.bridge_service != service {
            return Err(Failure::validation("cli.profile.connection_mismatch"));
        }
        if let Some(bound) = &self.task_id
            && task_id != Some(bound.as_str())
        {
            return Err(Failure::validation("cli.profile.task_mismatch"));
        }
        Ok(())
    }

    pub(crate) fn acknowledge(&mut self, event: &str) -> Result<()> {
        if self.after_event_id.as_deref() == Some(event) {
            return Ok(());
        }
        let index = self
            .delivered
            .iter()
            .position(|id| id == event)
            .ok_or_else(|| Failure::validation("cli.profile.event_not_delivered"))?;
        self.after_event_id = Some(event.into());
        self.delivered.drain(..=index);
        Ok(())
    }

    pub(crate) fn acknowledged_since<'a>(&self, before: &'a Self) -> Result<&'a [String]> {
        if self.after_event_id == before.after_event_id {
            return Ok(&[]);
        }
        let index = before
            .delivered
            .iter()
            .position(|id| Some(id) == self.after_event_id.as_ref())
            .ok_or_else(|| Failure::validation("cli.profile.cursor_mismatch"))?;
        Ok(&before.delivered[..=index])
    }

    pub(crate) fn record_delivery(
        &mut self,
        events: impl IntoIterator<Item = String>,
    ) -> Result<()> {
        for event in events {
            if self.after_event_id.as_deref() != Some(&event) && !self.delivered.contains(&event) {
                self.delivered.push(event);
            }
        }
        if self.delivered.len() > 1000 {
            return Err(Failure::validation("cli.profile.ack_required"));
        }
        Ok(())
    }
}

pub(crate) struct ProfileStore {
    _lock: File,
    path: PathBuf,
}

impl ProfileStore {
    pub(crate) fn reader_lock(root: &Path, key: &str) -> Result<File> {
        validate_key(key)?;
        let directory = root.join("cli-profiles");
        agent_room_bridge_local_adapter::create_private_directories(&directory)
            .map_err(|_| Failure::local("cli.profile.storage_unavailable"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{key}.inbox.lock")))
            .map_err(|_| Failure::local("cli.profile.storage_unavailable"))?;
        lock.try_lock().map_err(|_| {
            let mut error = Failure::local("cli.profile.reader_busy");
            error.retryable = true;
            error
        })?;
        Ok(lock)
    }

    pub(crate) fn open(root: &Path, key: &str) -> Result<Self> {
        validate_key(key)?;
        let directory = root.join("cli-profiles");
        agent_room_bridge_local_adapter::create_private_directories(&directory)
            .map_err(|_| Failure::local("cli.profile.storage_unavailable"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{key}.lock")))
            .map_err(|_| Failure::local("cli.profile.storage_unavailable"))?;
        lock.try_lock().map_err(|_| {
            let mut error = Failure::local("cli.profile.busy");
            error.retryable = true;
            error
        })?;
        Ok(Self {
            _lock: lock,
            path: directory.join(format!("{key}.json")),
        })
    }

    /// 找这个宿主任务已经为同一房间保存过的身份，让重跑 `join --room` 复用人物而不是再造一个。
    /// 指定了不同显示名就视为要另一个人物。无法读取的文件跳过；多个候选取最新的键。
    pub(crate) fn find_bound(
        root: &Path,
        service: &str,
        task_id: &str,
        target: &RoomTarget,
        display_name: Option<&str>,
    ) -> Option<String> {
        let entries = fs::read_dir(root.join("cli-profiles")).ok()?;
        entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| {
                let bytes = fs::read(entry.path()).ok()?;
                let profile: Profile = serde_json::from_slice(&bytes).ok()?;
                let key = entry.path().file_stem()?.to_str()?.to_owned();
                (profile.invitation.session_key == key
                    && validate_key(&key).is_ok()
                    && profile.bridge_service == service
                    && profile.task_id.as_deref() == Some(task_id)
                    && target.matches(&profile.invitation)
                    && display_name.is_none_or(|name| name == profile.invitation.display_name))
                .then_some(key)
            })
            .max()
    }

    pub(crate) fn load(&self) -> Result<Option<Profile>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Failure::local("cli.profile.storage_unavailable")),
        };
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::local("cli.profile.read_failed"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(Failure::local("cli.profile.corrupt"));
        }
        let profile: Profile =
            serde_json::from_slice(&bytes).map_err(|_| Failure::local("cli.profile.corrupt"))?;
        profile
            .invitation
            .validate()
            .map_err(|_| Failure::local("cli.profile.corrupt"))?;
        if self.path.file_stem().and_then(|name| name.to_str())
            != Some(&profile.invitation.session_key)
            || profile.delivered.len() > 1000
        {
            return Err(Failure::local("cli.profile.corrupt"));
        }
        Ok(Some(profile))
    }

    pub(crate) fn save(&self, profile: &Profile) -> Result<()> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| Failure::local("cli.profile.path_invalid"))?;
        let mut file = tempfile::NamedTempFile::new_in(directory)
            .map_err(|_| Failure::local("cli.profile.storage_unavailable"))?;
        serde_json::to_writer(&mut file, profile)
            .map_err(|_| Failure::local("cli.profile.write_failed"))?;
        file.flush()
            .and_then(|()| file.as_file().sync_all())
            .map_err(|_| Failure::local("cli.profile.write_failed"))?;
        file.persist(&self.path)
            .map_err(|_| Failure::local("cli.profile.write_failed"))?;
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<()> {
    IpcMethod::OpenHostSession(IpcOpenHostSessionRequest {
        room: None,
        session_key: key.into(),
        display_name: "Profile".into(),
    })
    .validate()
    .map_err(|_| Failure::validation("cli.profile.id_invalid"))
}

pub(crate) fn codex_task_id() -> Result<Option<String>> {
    task_id_from("CODEX_THREAD_ID")
}

/// Claude Code 把会话 ID 传给子进程；`claude --resume` 用的正是它，所以它就是接待要的任务 ID。
pub(crate) fn claude_code_task_id() -> Result<Option<String>> {
    task_id_from("CLAUDE_CODE_SESSION_ID")
}

/// 当前宿主任务的标识，用来把身份绑定到任务；没有已知宿主时为空。
pub(crate) fn host_task_id() -> Result<Option<String>> {
    match codex_task_id()? {
        Some(id) => Ok(Some(id)),
        None => claude_code_task_id(),
    }
}

/// 默认显示名：宿主加工作目录名，例如 `Claude Code · agent-room`，让房间里一眼看出是谁。
pub(crate) fn default_display_name() -> String {
    let host = if std::env::var_os("CODEX_THREAD_ID").is_some() {
        "Codex"
    } else if std::env::var_os("CLAUDE_CODE_SESSION_ID").is_some()
        || std::env::var_os("CLAUDECODE").is_some()
    {
        "Claude Code"
    } else {
        "Agent"
    };
    let workspace = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .map(|name| {
            name.chars()
                .filter(|character| !character.is_control())
                .take(64)
                .collect::<String>()
        })
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    match workspace {
        Some(workspace) => format!("{host} · {workspace}"),
        None => host.to_owned(),
    }
}

fn task_id_from(variable: &str) -> Result<Option<String>> {
    match std::env::var(variable) {
        Ok(id) => {
            validate_task_id(&id)?;
            Ok(Some(id))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(Failure::validation("cli.task_id_invalid")),
    }
}

pub(crate) fn validate_task_id(id: &str) -> Result<()> {
    let value =
        uuid::Uuid::parse_str(id).map_err(|_| Failure::validation("cli.task_id_invalid"))?;
    if value.is_nil() || value.to_string() != id {
        return Err(Failure::validation("cli.task_id_invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invitation() -> Invitation {
        Invitation {
            catalog_id: None,
            version: 1,
            session_key: uuid::Uuid::now_v7().to_string(),
            display_name: "调试 Agent".into(),
            room_id: Some("!room:test.invalid".into()),
        }
    }

    #[test]
    fn 邀请接受中文并拒绝额外凭据字段和非法版本() {
        let original = invitation();
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&original).unwrap());
        assert_eq!(Invitation::decode(&encoded).unwrap(), original);
        let mut value = serde_json::to_value(original).unwrap();
        value["token"] = "not-allowed".into();
        assert!(
            Invitation::decode(&URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap()))
                .is_err()
        );
        value.as_object_mut().unwrap().remove("token");
        value["version"] = 2.into();
        assert!(
            Invitation::decode(&URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap()))
                .is_err()
        );
    }

    #[test]
    fn 新进程恢复身份与未确认消息且拒绝跨任务接管() {
        let root = tempfile::tempdir().unwrap();
        let mut profile =
            Profile::new(invitation(), "test", Some(uuid::Uuid::now_v7().to_string()));
        let key = profile.invitation.session_key.clone();
        let reader = ProfileStore::reader_lock(root.path(), &key).unwrap();
        assert!(ProfileStore::reader_lock(root.path(), &key).is_err());
        profile
            .record_delivery(["$one".into(), "$two".into()])
            .unwrap();
        let store = ProfileStore::open(root.path(), &key).unwrap();
        store.save(&profile).unwrap();
        assert!(ProfileStore::open(root.path(), &key).is_err());
        drop(store);
        let store = ProfileStore::open(root.path(), &key).unwrap();
        let mut restored = store.load().unwrap().unwrap();
        assert!(
            restored
                .validate_binding("other", profile.task_id.as_deref())
                .is_err()
        );
        assert!(restored.validate_binding("test", None).is_err());
        assert!(
            restored
                .validate_binding("test", Some(&uuid::Uuid::now_v7().to_string()))
                .is_err()
        );
        assert!(restored.acknowledge("$unseen").is_err());
        restored.acknowledge("$one").unwrap();
        restored.acknowledge("$one").unwrap();
        assert_eq!(restored.delivered, ["$two"]);
        store.save(&restored).unwrap();
        drop(reader);
        assert!(ProfileStore::reader_lock(root.path(), &key).is_ok());
        assert_eq!(
            store.load().unwrap().unwrap().after_event_id.as_deref(),
            Some("$one")
        );
        fs::write(&store.path, b"broken").unwrap();
        assert_eq!(store.load().unwrap_err().code, "cli.profile.corrupt");
    }
}
