//! 按房间名接入：名字在 Bridge 给出的可进房间里解析，身份按宿主任务保存：同一任务一直是同一个
//! Agent，进别的房间也用它（`specs/agent-identity/design.md`）。这里不持有任何登录或 Matrix 秘密。

use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
};

use agent_room_bridge_ipc::{IpcHostRoomTarget, IpcOpenHostSessionRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一个任务在某个房间里的人物：会话键决定人物，显示名在重试时必须一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct JoinIdentity {
    pub(crate) session_key: String,
    pub(crate) display_name: String,
    /// 开会话时进的房间；默认公开大厅为空。回到上次的房间时按它重新开会话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) room: Option<IpcHostRoomTarget>,
    /// 开会话以后又进的这个房间：这条记录记的是它。开完会话再进一次（进过的再进也一样）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) entered: Option<IpcHostRoomTarget>,
}

impl JoinIdentity {
    pub(crate) fn request(&self) -> IpcOpenHostSessionRequest {
        IpcOpenHostSessionRequest {
            session_key: self.session_key.clone(),
            display_name: self.display_name.clone(),
            room: self.room.clone(),
        }
    }

    /// 这条记录记的房间：后来进的，或者开会话时进的。
    pub(crate) fn target(&self) -> Option<&IpcHostRoomTarget> {
        self.entered.as_ref().or(self.room.as_ref())
    }

    /// 换成这次解析到的房间（同一目录的私人房间实例可能换过）。
    pub(crate) fn place(&mut self, room: Option<IpcHostRoomTarget>) {
        if self.entered.is_some() {
            self.entered = room;
        } else {
            self.room = room;
        }
    }

    /// 同一个 Agent 再进一个房间：会话照旧开在原来的房间，开完再进这一间。
    fn entering(&self, room: Option<IpcHostRoomTarget>) -> Self {
        let same = room_key(room.as_ref()) == room_key(self.room.as_ref());
        Self {
            session_key: self.session_key.clone(),
            display_name: self.display_name.clone(),
            room: self.room.clone(),
            entered: if same { None } else { room },
        }
    }
}

impl From<IpcOpenHostSessionRequest> for JoinIdentity {
    fn from(request: IpcOpenHostSessionRequest) -> Self {
        Self {
            session_key: request.session_key,
            display_name: request.display_name,
            room: request.room,
            entered: None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskJoins {
    rooms: BTreeMap<String, JoinIdentity>,
    /// 最近进的房间，只说“接入”时回到这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<String>,
    /// 这个任务建过的房间：房间名到房间 ID（`specs/agent-rooms/design.md`）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    created: BTreeMap<String, String>,
}

/// 按目录区分房间；默认公开大厅没有目录。
fn room_key(room: Option<&IpcHostRoomTarget>) -> String {
    room.map_or_else(|| "default".to_owned(), |room| room.catalog_id.clone())
}

impl TaskJoins {
    /// 任务里已经有的 Agent：给了名字找叫这个名字的，优先最近进的房间；没给名字就是最近用的那个。
    fn agent(&self, name: Option<&str>) -> Option<&JoinIdentity> {
        let last = self.last.as_ref().and_then(|key| self.rooms.get(key));
        match name {
            None => last,
            Some(name) => last
                .filter(|identity| identity.display_name == name)
                .or_else(|| {
                    self.rooms
                        .values()
                        .find(|identity| identity.display_name == name)
                }),
        }
    }
}

/// 身份怎么来的，回给调用方好让它知道是否复用了旧人物。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IdentityOrigin {
    /// 同一宿主任务（或同一 MCP 连接）已经有的 Agent：这个房间里用过的，或者带进这个房间。
    Reused,
    /// 新建的人物。
    Created,
    /// 桌面端接入面板正在等的人物。
    Invited,
}

/// 不知道宿主任务时，人物只在这条 MCP 连接里复用：重试不会多出人物，但不写盘。
const CONNECTION_SCOPE: &str = "";

/// 进程内缓存加可选的磁盘副本：桌面/STDIO 模式按数据目录持久化，宿主重启后仍能找回人物。
pub(crate) struct JoinIdentities {
    root: Option<PathBuf>,
    memory: Mutex<HashMap<String, TaskJoins>>,
}

impl JoinIdentities {
    pub(crate) fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            memory: Mutex::new(HashMap::new()),
        }
    }

    /// 取这个任务为该房间保存的人物；这个房间里还没有时，任务里已经有的 Agent（给的名字是它的，
    /// 或者没给名字）带进这个房间；给了没用过的名字、或任务里还没有 Agent 时新建。
    /// 没有宿主任务标识时退回到这条连接内复用。
    pub(crate) fn resolve(
        &self,
        task_id: Option<&str>,
        room: Option<IpcHostRoomTarget>,
        display_name: Option<String>,
        default_name: impl FnOnce() -> String,
    ) -> (JoinIdentity, IdentityOrigin) {
        let key = room_key(room.as_ref());
        self.update(task_id, |joins| {
            let saved = joins.rooms.get(&key).filter(|saved| {
                display_name
                    .as_deref()
                    .is_none_or(|name| name == saved.display_name)
            });
            // 默认公开大厅没有目录，进不了“别的房间”：开会话时就在它里面的 Agent 才带得进去。
            let agent = joins
                .agent(display_name.as_deref())
                .filter(|agent| room.is_some() || agent.room.is_none());
            let (identity, origin) = match (saved, agent) {
                (Some(saved), _) => {
                    let mut identity = saved.clone();
                    identity.place(room);
                    (identity, IdentityOrigin::Reused)
                }
                (None, Some(agent)) => (agent.entering(room), IdentityOrigin::Reused),
                (None, None) => (
                    JoinIdentity {
                        session_key: uuid::Uuid::now_v7().to_string(),
                        display_name: display_name.unwrap_or_else(default_name),
                        room,
                        entered: None,
                    },
                    IdentityOrigin::Created,
                ),
            };
            joins.rooms.insert(key.clone(), identity.clone());
            joins.last = Some(key);
            (identity, origin)
        })
    }

    /// 记下接入面板交来的人物（或者建好房间的那条记录），之后这个任务再说“接入”或按名字进同一
    /// 房间都回到它。
    pub(crate) fn adopt(&self, task_id: Option<&str>, identity: JoinIdentity) {
        let key = room_key(identity.target());
        self.update(task_id, |joins| {
            joins.rooms.insert(key.clone(), identity);
            joins.last = Some(key);
        });
    }

    /// 这个任务里叫这个名字的人物，优先最近进的房间；找到了就把它记成最近进的。
    pub(crate) fn named(&self, task_id: Option<&str>, name: &str) -> Option<JoinIdentity> {
        self.update(task_id, |joins| {
            let key = joins
                .last
                .clone()
                .filter(|key| {
                    joins
                        .rooms
                        .get(key)
                        .is_some_and(|identity| identity.display_name == name)
                })
                .or_else(|| {
                    joins
                        .rooms
                        .iter()
                        .find(|(_, identity)| identity.display_name == name)
                        .map(|(key, _)| key.clone())
                })?;
            let identity = joins.rooms.get(&key).cloned();
            joins.last = Some(key);
            identity
        })
    }

    /// 这个任务用这个名字建过的房间 ID，没建过就生成一个记下。先记下再去建房：结果没回来时重试，
    /// 或者之后用同一个名字再建，都是同一间。
    pub(crate) fn created_room(&self, task_id: Option<&str>, name: &str) -> String {
        self.update(task_id, |joins| {
            joins
                .created
                .entry(name.to_owned())
                .or_insert_with(|| uuid::Uuid::now_v7().to_string())
                .clone()
        })
    }

    /// 这个任务在这个房间（目录 ID）里的人物。
    pub(crate) fn in_room(&self, task_id: Option<&str>, catalog_id: &str) -> Option<JoinIdentity> {
        self.update(task_id, |joins| joins.rooms.get(catalog_id).cloned())
    }

    /// 这个任务记着的房间：目录 ID、Matrix 房间 ID，或者这个任务建房时起的名字。
    pub(crate) fn catalog_of(&self, task_id: Option<&str>, wanted: &str) -> Option<String> {
        self.update(task_id, |joins| {
            joins
                .rooms
                .values()
                .flat_map(|identity| [identity.room.as_ref(), identity.entered.as_ref()])
                .flatten()
                .find(|room| room.catalog_id == wanted || room.room_id.as_deref() == Some(wanted))
                .map(|room| room.catalog_id.clone())
                .or_else(|| joins.created.get(wanted).cloned())
        })
    }

    /// 离开了这个房间：忘掉这个任务进这个房间的那条记录，最近进的回到开会话的那个房间。开会话时
    /// 进的那个房间离不开，它的记录不动。
    pub(crate) fn forget(&self, task_id: Option<&str>, catalog_id: &str) {
        self.update(task_id, |joins| {
            let Some(mut identity) = joins
                .rooms
                .remove(catalog_id)
                .filter(|identity| identity.entered.is_some())
            else {
                return;
            };
            identity.entered = None;
            let home = room_key(identity.room.as_ref());
            // 同一个 Agent 还要找得回来：开会话的那个房间没有记录时补一条。
            joins.rooms.entry(home.clone()).or_insert(identity);
            if joins.last.as_deref() == Some(catalog_id) {
                joins.last = Some(home);
            }
        });
    }

    /// 这个任务最近进的房间和人物。
    pub(crate) fn last(&self, task_id: Option<&str>) -> Option<JoinIdentity> {
        self.update(task_id, |joins| {
            joins
                .last
                .as_ref()
                .and_then(|key| joins.rooms.get(key))
                .cloned()
        })
    }

    fn update<T>(&self, task_id: Option<&str>, change: impl FnOnce(&mut TaskJoins) -> T) -> T {
        let mut memory = self
            .memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let joins = memory
            .entry(task_id.unwrap_or(CONNECTION_SCOPE).to_owned())
            .or_insert_with(|| task_id.map(|id| self.load(id)).unwrap_or_default());
        let before = serde_json::to_vec(&*joins).ok();
        let result = change(joins);
        if let Some(task_id) = task_id
            && serde_json::to_vec(&*joins).ok() != before
        {
            self.store(task_id, joins);
        }
        result
    }

    fn path(&self, task_id: &str) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|root| root.join(format!("{task_id}.json")))
    }

    fn load(&self, task_id: &str) -> TaskJoins {
        self.path(task_id)
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// 写失败只丢掉磁盘副本；本次接入仍按内存里的身份进行。
    fn store(&self, task_id: &str, joins: &TaskJoins) {
        let Some(path) = self.path(task_id) else {
            return;
        };
        let Some(directory) = path.parent() else {
            return;
        };
        // 数据根可能还不存在；和 Bridge 一样只对当前用户开放，免得 Bridge 之后拒绝启动。
        if agent_room_bridge_local_adapter::create_private_directories(directory).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(joins) else {
            return;
        };
        let _ = write_replace(directory, &path, &bytes);
    }
}

fn write_replace(directory: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = directory.join(format!(
        ".{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// 当前宿主任务的标识：Codex 在调用元数据里传 threadId，Claude Code 把会话 ID 放进环境。
/// 只接受规范的非零 UUID，避免把任意字符串当成文件名。
pub(crate) fn host_task_id(metadata_thread: Option<&Value>) -> Option<String> {
    let candidate = match metadata_thread {
        Some(value) => value.as_str().map(str::to_owned),
        None => std::env::var("CLAUDE_CODE_SESSION_ID").ok(),
    };
    candidate.filter(|id| {
        uuid::Uuid::parse_str(id).is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == *id)
    })
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

#[cfg(test)]
mod tests {
    use super::{IdentityOrigin, JoinIdentities, JoinIdentity, default_display_name, host_task_id};
    use agent_room_bridge_ipc::IpcHostRoomTarget;
    use serde_json::json;

    fn room(name: &str) -> IpcHostRoomTarget {
        IpcHostRoomTarget {
            catalog_id: name.to_owned(),
            room_id: None,
        }
    }

    #[test]
    fn 默认名是_agent_加工作区_不按宿主应用区分() {
        let name = default_display_name();
        assert!(name == "Agent" || name.starts_with("Agent · "), "{name}");
    }

    #[test]
    fn 同任务同房间复用人物_换名字或换任务才新建_并持久化到磁盘() {
        let directory = tempfile::tempdir().unwrap();
        let task = uuid::Uuid::now_v7().to_string();
        let identities = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        let (first, origin) =
            identities.resolve(Some(&task), Some(room("room-a")), None, || "Scout".into());
        assert_eq!(origin, IdentityOrigin::Created);
        assert_eq!(first.display_name, "Scout");
        let (again, origin) =
            identities.resolve(Some(&task), Some(room("room-a")), None, || "Other".into());
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(again, first);
        let (same_name, origin) = identities.resolve(
            Some(&task),
            Some(room("room-a")),
            Some("Scout".into()),
            || "x".into(),
        );
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(same_name, first);
        let (renamed, origin) = identities.resolve(
            Some(&task),
            Some(room("room-a")),
            Some("Pilot".into()),
            || "x".into(),
        );
        assert_eq!(origin, IdentityOrigin::Created);
        assert_ne!(renamed.session_key, first.session_key);
        let (other_room, _) = identities.resolve(Some(&task), None, None, || "Scout".into());
        assert_ne!(other_room.session_key, renamed.session_key);
        // 新进程从磁盘找回最新的人物。
        let restarted = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        let (restored, origin) =
            restarted.resolve(Some(&task), Some(room("room-a")), None, || "x".into());
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(restored, renamed);
        // 别的任务不共享人物；没有任务标识的调用不能落盘。
        let other_task = uuid::Uuid::now_v7().to_string();
        let (foreign, origin) =
            restarted.resolve(Some(&other_task), Some(room("room-a")), None, || "x".into());
        assert_eq!(origin, IdentityOrigin::Created);
        assert_ne!(foreign.session_key, renamed.session_key);
        // 不知道任务时只在这条连接里复用，重试不多造人物，也不写盘。
        let (unbound, origin) =
            restarted.resolve(None, Some(room("room-a")), None, || "Scout".into());
        assert_eq!(origin, IdentityOrigin::Created);
        assert_ne!(unbound.session_key, renamed.session_key);
        let (retried, origin) = restarted.resolve(None, Some(room("room-a")), None, || "x".into());
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(retried, unbound);
        let (fresh_connection, _) = JoinIdentities::new(Some(directory.path().join("mcp-joins")))
            .resolve(None, Some(room("room-a")), None, || "Scout".into());
        assert_ne!(fresh_connection.session_key, unbound.session_key);
        let files: Vec<_> = std::fs::read_dir(directory.path().join("mcp-joins"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&format!("{task}.json")));
        assert!(files.contains(&format!("{other_task}.json")));
    }

    #[test]
    fn 记下最近进的房间_面板交来的人物也算() {
        let directory = tempfile::tempdir().unwrap();
        let task = uuid::Uuid::now_v7().to_string();
        let identities = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        assert!(identities.last(Some(&task)).is_none());
        let (game, _) =
            identities.resolve(Some(&task), Some(room("game")), None, || "Scout".into());
        assert_eq!(identities.last(Some(&task)), Some(game.clone()));
        let invited = JoinIdentity {
            session_key: uuid::Uuid::now_v7().to_string(),
            display_name: "面板里的名字".into(),
            room: Some(room("ops")),
            entered: None,
        };
        identities.adopt(Some(&task), invited.clone());
        assert_eq!(identities.last(Some(&task)), Some(invited.clone()));
        // 之后按名字进同一房间，回到面板交来的人物。
        let (again, origin) =
            identities.resolve(Some(&task), Some(room("ops")), None, || "x".into());
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(again, invited);
        // 新进程也记得最近的房间。
        let restarted = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        assert_eq!(restarted.last(Some(&task)), Some(invited));
    }

    #[test]
    fn 同一任务进别的房间还是同一个_agent_给了没用过的名字才另起() {
        let directory = tempfile::tempdir().unwrap();
        let task = uuid::Uuid::now_v7().to_string();
        let identities = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        let (first, _) =
            identities.resolve(Some(&task), Some(room("room-a")), None, || "Scout".into());
        // 不给名字、或者给它的名字：带进第二个房间，会话照旧开在第一个房间。
        let (second, origin) =
            identities.resolve(Some(&task), Some(room("room-b")), None, || "x".into());
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(second.session_key, first.session_key);
        assert_eq!(second.room, first.room);
        assert_eq!(second.entered, Some(room("room-b")));
        assert_eq!(second.target(), Some(&room("room-b")));
        let (third, origin) = identities.resolve(
            Some(&task),
            Some(room("room-c")),
            Some("Scout".into()),
            || "x".into(),
        );
        assert_eq!(origin, IdentityOrigin::Reused);
        assert_eq!(third.session_key, first.session_key);
        // 回到开会话的那个房间不用再进。
        let (home, _) = identities.resolve(Some(&task), Some(room("room-a")), None, || "x".into());
        assert_eq!(home, first);
        // 最近进的房间记的是后来进的那一间。
        identities.resolve(Some(&task), Some(room("room-b")), None, || "x".into());
        assert_eq!(identities.last(Some(&task)), Some(second.clone()));
        // 给了没用过的名字才另起一个。
        let (pilot, origin) = identities.resolve(
            Some(&task),
            Some(room("room-d")),
            Some("Pilot".into()),
            || "x".into(),
        );
        assert_eq!(origin, IdentityOrigin::Created);
        assert_ne!(pilot.session_key, first.session_key);
        assert_eq!(pilot.entered, None);
        // 新进程照样记得。
        let restarted = JoinIdentities::new(Some(directory.path().join("mcp-joins")));
        let (again, _) = restarted.resolve(Some(&task), Some(room("room-b")), None, || "x".into());
        assert_eq!(again, second);
    }

    #[test]
    fn 任务标识只接受规范uuid() {
        let id = uuid::Uuid::now_v7().to_string();
        assert_eq!(host_task_id(Some(&json!(id))), Some(id.clone()));
        assert_eq!(host_task_id(Some(&json!("../escape"))), None);
        assert_eq!(host_task_id(Some(&json!(id.to_uppercase()))), None);
        assert_eq!(host_task_id(Some(&json!(42))), None);
    }
}
