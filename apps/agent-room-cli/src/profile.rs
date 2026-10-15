//! Task-local identity and acknowledged inbox progress. No login or Matrix secrets are stored here.
use crate::output::{CliFailure as Failure, CliResult as Result};
use agent_room_bridge_ipc::{IpcMethod, IpcOpenHostSessionRequest, limits::SESSION_ROOMS};
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
    /// 应用里复制出来的邀请可以不带名字，由接上的 Agent 自己起；存进档案的邀请总有名字。
    #[serde(default)]
    pub(crate) display_name: String,
    pub(crate) room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) catalog_id: Option<String>,
}

impl Invitation {
    /// `chosen` 是 Agent 自己起的名字（`--name`）：邀请里有人定了名字就用邀请的，
    /// 两边都没有才用 `Agent` 加工作目录名生成。
    pub(crate) fn decode(encoded: &str, chosen: Option<&str>) -> Result<Self> {
        if encoded.len() > 4096 {
            return Err(Failure::validation("cli.invitation_invalid"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| Failure::validation("cli.invitation_invalid"))?;
        let mut invite: Self = serde_json::from_slice(&bytes)
            .map_err(|_| Failure::validation("cli.invitation_invalid"))?;
        if invite.display_name.is_empty() {
            invite.display_name = chosen.map_or_else(default_display_name, str::to_owned);
        }
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
    /// 房间名：记进档案，之后 `--room` 能按名字找。
    pub(crate) name: Option<String>,
}

impl RoomTarget {
    pub(crate) const DEFAULT_LOBBY: Self = Self {
        catalog_id: None,
        room_id: None,
        name: None,
    };

    /// 同一目录条目就是同一个房间；Matrix 房间实例变了由连接时的房间核对来报。
    pub(crate) fn matches(&self, invitation: &Invitation) -> bool {
        invitation.catalog_id == self.catalog_id
    }
}

/// 开会话以后这个 Agent 又进的房间（`specs/agent-identity/design.md`）：一个任务一个 Agent，
/// 进别的房间不另起人物，会话照旧开在第一次进的房间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EnteredRoom {
    pub(crate) catalog_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// 这个 Agent 建的：同一任务用同一个房间名再建就回到它，不再建一间。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) created: bool,
}

/// `--room` 说的、这个 Agent 在的房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownRoom {
    pub(crate) room_id: Option<String>,
    /// 后来进的房间的目录 ID；会话开在的那个房间为空（管房间时交给 Bridge 默认）。
    pub(crate) catalog_id: Option<String>,
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
    /// 旧版记在档案里的确认位置；连上以后交给 Bridge，之后一直是空的。
    pub(crate) after_event_id: Option<String>,
    /// 旧版记的已交出、还没确认的消息；不再用。
    pub(crate) delivered: Vec<String>,
    /// `create-room` 为建哪个房间造的这个人物（房间名）。同一任务用同一个名字再建就回到它；
    /// 别的人物没有这一项，档案写出来和以前一样，旧版照样读得出来。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_room: Option<String>,
    /// 会话开在的那个房间的名字（按名字接入时记下），`--room` 按名字找它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) room_name: Option<String>,
    /// 开会话以后又进的房间。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) rooms: Vec<EnteredRoom>,
    /// 最近进的房间（Matrix 房间 ID）：说话、看前后、管房间不给 `--room` 时用它；
    /// 空着就是会话开在的那个房间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) current_room: Option<String>,
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
            created_room: None,
            room_name: None,
            rooms: Vec::new(),
            current_room: None,
        }
    }

    /// 这个 Agent 在不在这个房间：会话开在的，或者后来进的。
    pub(crate) fn in_room(&self, target: &RoomTarget) -> bool {
        target.matches(&self.invitation)
            || target
                .catalog_id
                .as_ref()
                .is_some_and(|catalog| self.rooms.iter().any(|room| &room.catalog_id == catalog))
    }

    /// 这个 Agent 用这个名字建过、后来进去的房间。
    pub(crate) fn created(&self, name: &str) -> Option<&EnteredRoom> {
        self.rooms
            .iter()
            .find(|room| room.created && room.name.as_deref() == Some(name))
    }

    /// 记下又进的房间；进过的就更新它（建房的标记不丢）。最多记 [`SESSION_ROOMS`] 个。
    pub(crate) fn remember(&mut self, room: EnteredRoom) {
        if let Some(known) = self
            .rooms
            .iter_mut()
            .find(|known| known.catalog_id == room.catalog_id)
        {
            known.created |= room.created;
            if room.room_id.is_some() {
                known.room_id = room.room_id;
            }
            if room.name.is_some() {
                known.name = room.name;
            }
        } else if self.rooms.len() < SESSION_ROOMS {
            self.rooms.push(room);
        }
    }

    /// 离开了这个房间：不再记它，最近进的是它时回到会话开在的那个。
    pub(crate) fn forget(&mut self, catalog_id: &str) {
        let Some(index) = self
            .rooms
            .iter()
            .position(|room| room.catalog_id == catalog_id)
        else {
            return;
        };
        let room = self.rooms.remove(index);
        if room.room_id.is_some() && self.current_room == room.room_id {
            self.current_room = None;
        }
    }

    /// `--room` 说的房间：房间名（不分大小写）、Matrix 房间 ID 或目录 ID，在这个 Agent 在的房间里找。
    pub(crate) fn room_for(&self, wanted: &str) -> Option<KnownRoom> {
        let wanted = wanted.trim();
        let named = |name: Option<&String>| {
            name.is_some_and(|name| name.to_lowercase() == wanted.to_lowercase())
        };
        let home = self.room_id.as_ref().or(self.invitation.room_id.as_ref());
        if home.is_some_and(|room| room == wanted)
            || self.invitation.catalog_id.as_deref() == Some(wanted)
            || named(self.room_name.as_ref().or(self.created_room.as_ref()))
        {
            return Some(KnownRoom {
                room_id: home.cloned(),
                catalog_id: None,
            });
        }
        self.rooms
            .iter()
            .find(|room| {
                room.catalog_id == wanted
                    || room.room_id.as_deref() == Some(wanted)
                    || named(room.name.as_ref())
            })
            .map(|room| KnownRoom {
                room_id: room.room_id.clone(),
                catalog_id: Some(room.catalog_id.clone()),
            })
    }

    /// 最近进的房间；没有就是会话开在的那个。
    pub(crate) fn current(&self) -> KnownRoom {
        self.current_room
            .as_deref()
            .and_then(|room| self.room_for(room))
            .unwrap_or_else(|| KnownRoom {
                room_id: self.room_id.clone(),
                catalog_id: None,
            })
    }

    /// 这个 Agent 在的房间的名字，找不到房间时告诉 Agent 能说哪些。
    pub(crate) fn room_names(&self) -> String {
        self.room_name
            .iter()
            .chain(self.created_room.iter())
            .chain(self.rooms.iter().filter_map(|room| room.name.as_ref()))
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("; ")
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
        Self::task_profiles(root, service, task_id)
            .filter(|(_, _, profile)| {
                profile.in_room(target)
                    && display_name.is_none_or(|name| name == profile.invitation.display_name)
            })
            .map(|(_, key, _)| key)
            .max()
    }

    /// 这个宿主任务最近用过的人物（按档案最后写入的时间）：只说“接入”时回到上次的房间。
    /// 带了名字就只找叫这个名字的人物。
    pub(crate) fn latest_bound(
        root: &Path,
        service: &str,
        task_id: &str,
        display_name: Option<&str>,
    ) -> Option<String> {
        Self::task_profiles(root, service, task_id)
            .filter(|(_, _, profile)| {
                display_name.is_none_or(|name| name == profile.invitation.display_name)
            })
            .filter_map(|(entry, key, _)| {
                let modified = entry.metadata().and_then(|meta| meta.modified()).ok()?;
                Some((modified, key))
            })
            .max()
            .map(|(_, key)| key)
    }

    /// 这个宿主任务用同一个房间名建过的房间：重跑 `create-room` 回到那间和建它的人物，不再建一间。
    pub(crate) fn find_created(
        root: &Path,
        service: &str,
        task_id: &str,
        room_name: &str,
    ) -> Option<String> {
        Self::task_profiles(root, service, task_id)
            .filter(|(_, _, profile)| {
                profile.created_room.as_deref() == Some(room_name)
                    || profile.created(room_name).is_some()
            })
            .map(|(_, key, _)| key)
            .max()
    }

    /// 这个连接下绑在这个宿主任务上的人物：目录项、键和档案。读不出来、键对不上的文件跳过。
    fn task_profiles(
        root: &Path,
        service: &str,
        task_id: &str,
    ) -> impl Iterator<Item = (fs::DirEntry, String, Profile)> {
        fs::read_dir(root.join("cli-profiles"))
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(move |entry| {
                let profile: Profile =
                    serde_json::from_slice(&fs::read(entry.path()).ok()?).ok()?;
                let key = entry.path().file_stem()?.to_str()?.to_owned();
                (profile.invitation.session_key == key
                    && validate_key(&key).is_ok()
                    && profile.bridge_service == service
                    && profile.task_id.as_deref() == Some(task_id))
                .then_some((entry, key, profile))
            })
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
            || profile.rooms.len() > SESSION_ROOMS
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

/// Agent 没给自己起名时的默认显示名：`Agent` 加工作目录名，例如 `Agent · agent-room`。
/// 不按宿主应用区分名字。
pub(crate) fn default_display_name() -> String {
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
        Some(workspace) => format!("Agent · {workspace}"),
        None => "Agent".to_owned(),
    }
}

fn task_id_from(variable: &str) -> Result<Option<String>> {
    match std::env::var(variable) {
        // 空值等于没设置，不能让所有命令都因为一个空环境变量失败。
        Ok(id) if id.is_empty() => Ok(None),
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
        assert_eq!(Invitation::decode(&encoded, None).unwrap(), original);
        // 邀请里有人定了名字，Agent 自己起的名字不覆盖它。
        assert_eq!(
            Invitation::decode(&encoded, Some("Scout"))
                .unwrap()
                .display_name,
            "调试 Agent"
        );
        let mut value = serde_json::to_value(original).unwrap();
        value["token"] = "not-allowed".into();
        assert!(
            Invitation::decode(
                &URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap()),
                None
            )
            .is_err()
        );
        value.as_object_mut().unwrap().remove("token");
        value["version"] = 2.into();
        assert!(
            Invitation::decode(
                &URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn 不带名字的邀请由_agent_自己起名_没起就用_agent_加工作区() {
        let mut value = serde_json::to_value(invitation()).unwrap();
        value.as_object_mut().unwrap().remove("displayName");
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap());
        assert_eq!(
            Invitation::decode(&encoded, Some("Scout"))
                .unwrap()
                .display_name,
            "Scout"
        );
        assert_eq!(
            Invitation::decode(&encoded, None).unwrap().display_name,
            default_display_name()
        );
        // 默认名不按宿主应用区分：设没设 CODEX_THREAD_ID、CLAUDE_CODE_SESSION_ID 都一样。
        let name = default_display_name();
        assert!(name == "Agent" || name.starts_with("Agent · "), "{name}");
        // 自己起的名字同样要是 1 到 128 个可见字符。
        assert!(Invitation::decode(&encoded, Some(" 前后空格 ")).is_err());
        assert!(Invitation::decode(&encoded, Some(&"名".repeat(129))).is_err());
        assert!(Invitation::decode(&encoded, Some(&"名".repeat(128))).is_ok());
    }

    #[test]
    fn 新进程恢复身份且拒绝跨任务接管_旧版记的位置照样读得出来() {
        let root = tempfile::tempdir().unwrap();
        let mut profile =
            Profile::new(invitation(), "test", Some(uuid::Uuid::now_v7().to_string()));
        let key = profile.invitation.session_key.clone();
        let reader = ProfileStore::reader_lock(root.path(), &key).unwrap();
        assert!(ProfileStore::reader_lock(root.path(), &key).is_err());
        // 旧版把确认位置和交出去的消息记在档案里；新版读得出来，连上以后交给 Bridge。
        profile.after_event_id = Some("$one".into());
        profile.delivered = vec!["$two".into()];
        let store = ProfileStore::open(root.path(), &key).unwrap();
        store.save(&profile).unwrap();
        assert!(ProfileStore::open(root.path(), &key).is_err());
        drop(store);
        let store = ProfileStore::open(root.path(), &key).unwrap();
        let restored = store.load().unwrap().unwrap();
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
        assert_eq!(restored.after_event_id.as_deref(), Some("$one"));
        assert_eq!(restored.delivered, ["$two"]);
        drop(reader);
        assert!(ProfileStore::reader_lock(root.path(), &key).is_ok());
        fs::write(&store.path, b"broken").unwrap();
        assert_eq!(store.load().unwrap_err().code, "cli.profile.corrupt");
    }
}
