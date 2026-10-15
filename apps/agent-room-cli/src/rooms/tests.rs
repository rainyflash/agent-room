use super::*;
use crate::profile::host_task_id;
use agent_room_agent_client::{BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcBridgeState, IpcHostSessionState, IpcHostSessionSummary,
    IpcOpenHostSessionRequest, IpcRoomKind, IpcRoomKnock, IpcRoomMembership,
};
use std::{collections::BTreeMap, sync::Mutex};

const CODE: &str = "K7P3-Q9XW-2DMA";
const BUILT_ROOM: &str = "!built:test.invalid";
const GUIDE: &str = "https://api.example.test/agents.txt";

/// 像 Bridge 一样建房（同一个房间 ID 就是同一间）、开会话、管房间，记下收到的请求。
struct RoomsBridge {
    created: Mutex<Vec<IpcCreateRoomRequest>>,
    opened: Mutex<Vec<IpcOpenHostSessionRequest>>,
    actions: Mutex<Vec<IpcRoomAccessAction>>,
    /// 看口令时说的房间（目录 ID），和开好会话以后又进的房间。
    access_rooms: Mutex<Vec<Option<String>>>,
    entered: Mutex<Vec<String>>,
    managed: Mutex<Vec<IpcMethod>>,
    refuse_create: Mutex<Option<&'static str>>,
    code: Mutex<IpcRoomCode>,
    knocks: Mutex<Vec<IpcRoomKnock>>,
}

impl RoomsBridge {
    fn new() -> Self {
        Self {
            created: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            actions: Mutex::new(Vec::new()),
            access_rooms: Mutex::new(Vec::new()),
            entered: Mutex::new(Vec::new()),
            managed: Mutex::new(Vec::new()),
            refuse_create: Mutex::new(None),
            code: Mutex::new(IpcRoomCode::Visible { code: CODE.into() }),
            knocks: Mutex::new(Vec::new()),
        }
    }

    fn summary(request: &IpcCreateRoomRequest) -> IpcRoomSummary {
        IpcRoomSummary {
            kind: IpcRoomKind::PrivateRoom,
            catalog_id: request.catalog_id.clone(),
            matrix_room_id: Some(BUILT_ROOM.into()),
            name: request.name.clone(),
            slug: None,
            membership: Some(IpcRoomMembership::Joined),
        }
    }

    fn latest_catalog(&self) -> String {
        self.created
            .lock()
            .unwrap()
            .last()
            .map(|request| request.catalog_id.clone())
            .unwrap_or_default()
    }

    fn create(
        &self,
        request: IpcCreateRoomRequest,
    ) -> std::result::Result<IpcResponse, BridgeToolFailure> {
        if let Some(code) = *self.refuse_create.lock().unwrap() {
            return Err(refusal(code));
        }
        let room = Self::summary(&request);
        self.created.lock().unwrap().push(request);
        Ok(IpcResponse::CreatedRoom { room })
    }

    fn in_session(&self, method: IpcMethod) -> std::result::Result<IpcResponse, BridgeToolFailure> {
        let catalog_id = self.latest_catalog();
        match method {
            IpcMethod::GetSelf => Ok(IpcResponse::SelfSummary {
                summary: self_summary(&catalog_id),
            }),
            IpcMethod::EnterRoom(request) => {
                self.entered
                    .lock()
                    .unwrap()
                    .push(request.room.catalog_id.clone());
                Ok(IpcResponse::RoomEntered {
                    room: agent_room_bridge_ipc::IpcSessionRoom {
                        catalog_id: request.room.catalog_id,
                        room_id: BUILT_ROOM.into(),
                    },
                })
            }
            IpcMethod::RoomAccess(request) => {
                self.actions.lock().unwrap().push(request.action);
                self.access_rooms.lock().unwrap().push(request.catalog_id);
                let mut code = self.code.lock().unwrap();
                match request.action {
                    IpcRoomAccessAction::Replace => {
                        *code = IpcRoomCode::Visible {
                            code: "N3WC-0DE9-ABCD".into(),
                        };
                    }
                    IpcRoomAccessAction::Disable => *code = IpcRoomCode::Missing,
                    IpcRoomAccessAction::Show => {}
                }
                Ok(IpcResponse::RoomAccess {
                    access: IpcRoomAccess {
                        catalog_id,
                        code: code.clone(),
                        knocks: self.knocks.lock().unwrap().clone(),
                        guide_url: GUIDE.into(),
                    },
                })
            }
            method @ (IpcMethod::AnswerKnock(_)
            | IpcMethod::InvitePerson(_)
            | IpcMethod::RemoveAgent(_)) => {
                self.managed.lock().unwrap().push(method);
                Ok(IpcResponse::RoomUpdated { catalog_id })
            }
            _ => Err(refusal("test.unexpected_call")),
        }
    }
}

impl BridgeToolClient for RoomsBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let response = match method {
            IpcMethod::CreateRoom(request) => self.create(request),
            IpcMethod::OpenHostSession(request) => {
                self.opened.lock().unwrap().push(request);
                Ok(IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: uuid::Uuid::now_v7().to_string(),
                        state: IpcHostSessionState::Ready,
                        agent_id: None,
                        error_code: None,
                    },
                })
            }
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: self
                    .created
                    .lock()
                    .unwrap()
                    .iter()
                    .map(Self::summary)
                    .collect(),
            }),
            IpcMethod::WithSession { method, .. } => self.in_session(*method),
            _ => Err(refusal("test.unexpected_call")),
        };
        Box::pin(async move { response })
    }
}

fn self_summary(catalog_id: &str) -> IpcSelfSummary {
    IpcSelfSummary {
        owner: None,
        room_catalog_id: Some(catalog_id.to_owned()),
        agent: IpcAgentSummary {
            agent_id: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b40".into(),
            display_name: "Scout".into(),
            matrix_user_id: "@scout:test.invalid".into(),
            avatar_url: None,
        },
        instance_id: uuid::Uuid::now_v7().to_string(),
        matrix_device_id: "TEST".into(),
        room_id: BUILT_ROOM.into(),
        connection_state: IpcBridgeState::Ready,
        granted_capabilities: vec![],
    }
}

fn refusal(code: &str) -> BridgeToolFailure {
    BridgeToolFailure::new(
        code,
        IpcErrorCategory::DependencyUnavailable,
        true,
        BTreeMap::new(),
    )
}

fn creation(room: &str, name: Option<&str>) -> Creation {
    Creation {
        room: room.to_owned(),
        name: name.map(str::to_owned),
    }
}

fn saved_profiles(root: &Path) -> Vec<Profile> {
    let Ok(entries) = std::fs::read_dir(root.join("cli-profiles")) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
        .collect()
}

#[tokio::test]
async fn 建房后进去_结果里有口令和要转给别的_agent_的话() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = RoomsBridge::new();
    let task = uuid::Uuid::now_v7().to_string();

    let value = create(
        &bridge,
        directory.path(),
        "test",
        None,
        Some(task.clone()),
        creation("  Research  ", Some("Scout")),
    )
    .await
    .unwrap();

    let created = bridge.created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].name, "Research", "房间名前后的空格去掉");
    assert_eq!(created[0].display_name, "Scout");
    // 建房的人物就是进房间的人物，房间 ID 是档案里先定下的那个。
    let profiles = saved_profiles(directory.path());
    assert_eq!(profiles.len(), 1);
    let profile = &profiles[0];
    assert_eq!(profile.created_room.as_deref(), Some("Research"));
    assert_eq!(profile.task_id.as_deref(), Some(task.as_str()));
    assert_eq!(profile.invitation.session_key, created[0].session_key);
    assert_eq!(
        profile.invitation.catalog_id.as_deref(),
        Some(created[0].catalog_id.as_str())
    );
    assert_eq!(profile.invitation.room_id.as_deref(), Some(BUILT_ROOM));
    assert_eq!(profile.room_id.as_deref(), Some(BUILT_ROOM));
    let opened = bridge.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].session_key, created[0].session_key);
    assert_eq!(*bridge.actions.lock().unwrap(), [IpcRoomAccessAction::Show]);

    assert_eq!(value["profileId"], profile.invitation.session_key.as_str());
    assert_eq!(value["room"]["name"], "Research");
    assert_eq!(value["access"]["code"]["code"], CODE);
    assert_eq!(value["access"]["guideUrl"], GUIDE);
    for language in ["zh", "en"] {
        let share = value["access"]["share"][language].as_str().unwrap();
        assert!(
            share.contains(CODE) && share.contains("Research"),
            "{share}"
        );
    }
    assert!(
        value["localAgents"]
            .as_str()
            .unwrap()
            .contains("join --room \"Research\"")
    );
    assert!(value["people"].as_str().unwrap().contains("invite"));
    let next = value["next"].as_str().unwrap();
    assert!(next.contains(&format!(
        "--profile {} read",
        profile.invitation.session_key
    )));
}

#[tokio::test]
async fn 同一任务同名再建回到那间_再建一间还是这个_agent_换名字或换任务才另起() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = RoomsBridge::new();
    let task = uuid::Uuid::now_v7().to_string();
    let root = directory.path();

    create(
        &bridge,
        root,
        "test",
        None,
        Some(task.clone()),
        creation("Research", Some("Scout")),
    )
    .await
    .unwrap();
    // 第一次的结果没回来也一样：重试时给了别的名字，照旧用建房的人物，不然控制面会拒绝。
    create(
        &bridge,
        root,
        "test",
        None,
        Some(task.clone()),
        creation("Research", Some("Other")),
    )
    .await
    .unwrap();
    {
        let created = bridge.created.lock().unwrap();
        assert_eq!(created.len(), 2);
        assert_eq!(created[0], created[1], "同一个房间 ID、同一个人物");
    }
    assert_eq!(saved_profiles(root).len(), 1);

    // 再建一间：还是这个 Agent，建好以后进新房间，口令看的是新房间；再说一次回到同一间。
    for _ in 0..2 {
        create(
            &bridge,
            root,
            "test",
            None,
            Some(task.clone()),
            creation("Design", None),
        )
        .await
        .unwrap();
    }
    {
        let created = bridge.created.lock().unwrap();
        assert_eq!(created.len(), 4);
        assert_ne!(created[2].catalog_id, created[0].catalog_id);
        assert_eq!(created[3], created[2]);
        assert_eq!(created[2].session_key, created[0].session_key);
        assert_eq!(
            bridge.entered.lock().unwrap().as_slice(),
            [created[2].catalog_id.clone(), created[2].catalog_id.clone()]
        );
        assert_eq!(
            bridge
                .access_rooms
                .lock()
                .unwrap()
                .last()
                .cloned()
                .flatten(),
            Some(created[2].catalog_id.clone())
        );
    }
    let profiles = saved_profiles(root);
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].rooms.len(), 1);
    assert!(profiles[0].rooms[0].created);
    // 给了没用过的名字、或者换个任务，才另起一个 Agent。
    create(
        &bridge,
        root,
        "test",
        None,
        Some(task.clone()),
        creation("Notes", Some("Pilot")),
    )
    .await
    .unwrap();
    let other_task = uuid::Uuid::now_v7().to_string();
    create(
        &bridge,
        root,
        "test",
        None,
        Some(other_task),
        creation("Research", None),
    )
    .await
    .unwrap();
    let created = bridge.created.lock().unwrap();
    assert_eq!(created.len(), 6);
    assert_ne!(created[4].session_key, created[0].session_key);
    assert_ne!(created[5].catalog_id, created[0].catalog_id);
    assert_ne!(created[5].session_key, created[0].session_key);
    assert_eq!(saved_profiles(root).len(), 3);
}

#[tokio::test]
async fn 没有宿主任务时失败带上_profile_id_带着它重试回到同一间() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let bridge = RoomsBridge::new();
    *bridge.refuse_create.lock().unwrap() = Some("bridge.rooms.control_plane_unavailable");

    let error = create(
        &bridge,
        root,
        "test",
        None,
        None,
        creation("Research", None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "bridge.rooms.control_plane_unavailable");
    assert!(error.retryable);
    let key = error.details["profileId"].clone();
    let saved = saved_profiles(root);
    assert_eq!(saved.len(), 1, "建房之前人物和房间 ID 已经存好");
    assert_eq!(saved[0].invitation.session_key, key);

    *bridge.refuse_create.lock().unwrap() = None;
    create(
        &bridge,
        root,
        "test",
        Some(key.clone()),
        None,
        creation("Research", None),
    )
    .await
    .unwrap();
    let created = bridge.created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].session_key, key);
    assert_eq!(
        saved[0].invitation.catalog_id.as_deref(),
        Some(created[0].catalog_id.as_str())
    );
    // 不带 --profile 又没有宿主任务：认不出是重试，另建一间。
    create(
        &bridge,
        root,
        "test",
        None,
        None,
        creation("Research", None),
    )
    .await
    .unwrap();
    assert_eq!(saved_profiles(root).len(), 2);
    // 带着这个 --profile 再建一间：还是它，建好以后进去。
    create(
        &bridge,
        root,
        "test",
        Some(key.clone()),
        None,
        creation("Design", None),
    )
    .await
    .unwrap();
    {
        let created = bridge.created.lock().unwrap();
        let design = created.last().unwrap();
        assert_eq!(design.session_key, key);
        assert_eq!(
            bridge.entered.lock().unwrap().as_slice(),
            std::slice::from_ref(&design.catalog_id)
        );
    }
    let missing = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        create(
            &bridge,
            root,
            "test",
            Some(missing),
            None,
            creation("Design", None)
        )
        .await
        .unwrap_err()
        .code,
        "cli.profile.not_found"
    );
}

#[tokio::test]
async fn 名字不对不存人物也不碰_bridge_旧版_bridge_说要更新() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let bridge = RoomsBridge::new();
    let long = "房".repeat(129);

    for (room, name, code) in [
        ("", None, "bridge.ipc.room_name_invalid"),
        ("第一行\n第二行", None, "bridge.ipc.room_name_invalid"),
        (long.as_str(), None, "bridge.ipc.room_name_invalid"),
        (
            "Research",
            Some(" Scout"),
            "bridge.ipc.session_name_invalid",
        ),
    ] {
        let error = create(&bridge, root, "test", None, None, creation(room, name))
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.category, IpcErrorCategory::Validation);
    }
    assert!(saved_profiles(root).is_empty());
    assert!(bridge.created.lock().unwrap().is_empty());

    *bridge.refuse_create.lock().unwrap() = Some("bridge.ipc.protocol_failed");
    let error = create(
        &bridge,
        root,
        "test",
        None,
        None,
        creation("Research", None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "agent.rooms.bridge_outdated");
    assert_eq!(error.category, IpcErrorCategory::IncompatibleVersion);
    assert!(!error.retryable);
    assert!(error.hint.contains("update"), "{}", error.hint);
    assert!(error.details.contains_key("profileId"));
}

fn access_args(replace_code: bool, disable_code: bool) -> AccessArgs {
    AccessArgs {
        session: Some(uuid::Uuid::now_v7().to_string()),
        room: None,
        catalog: None,
        replace_code,
        disable_code,
    }
}

#[tokio::test]
async fn 看口令时有人敲门先说敲门_换口令和停用要明说() {
    let bridge = RoomsBridge::new();
    bridge.created.lock().unwrap().push(IpcCreateRoomRequest {
        catalog_id: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b20".into(),
        name: "Research".into(),
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: "Scout".into(),
    });
    bridge.knocks.lock().unwrap().push(IpcRoomKnock {
        agent_id: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b41".into(),
        display_name: "Visitor".into(),
        knocked_at_unix_ms: 1,
        expires_at_unix_ms: 2,
    });

    let value = access(&bridge, access_args(false, false)).await.unwrap();
    assert_eq!(value["code"]["code"], CODE);
    assert_eq!(value["knocks"][0]["displayName"], "Visitor");
    assert!(value["share"]["en"].as_str().unwrap().contains("Research"));
    assert!(value["next"].as_str().unwrap().contains("admit <agentId>"));

    bridge.knocks.lock().unwrap().clear();
    let value = access(&bridge, access_args(true, false)).await.unwrap();
    assert_eq!(value["code"]["code"], "N3WC-0DE9-ABCD");
    let value = access(&bridge, access_args(false, true)).await.unwrap();
    assert_eq!(value["code"]["state"], "missing");
    assert!(!value["share"]["zh"].as_str().unwrap().contains("N3WC"));
    assert!(value["next"].as_str().unwrap().contains("turned off"));
    *bridge.code.lock().unwrap() = IpcRoomCode::Hidden;
    let value = access(&bridge, access_args(false, false)).await.unwrap();
    assert!(value["next"].as_str().unwrap().contains("--replace-code"));
    assert_eq!(
        *bridge.actions.lock().unwrap(),
        [
            IpcRoomAccessAction::Show,
            IpcRoomAccessAction::Replace,
            IpcRoomAccessAction::Disable,
            IpcRoomAccessAction::Show,
        ]
    );
}

fn agent_args(agent: &str) -> AgentArgs {
    AgentArgs {
        session: Some(uuid::Uuid::now_v7().to_string()),
        room: None,
        catalog: None,
        agent: agent.to_owned(),
    }
}

#[tokio::test]
async fn 放行拒绝邀请移出都管会话所在的房间() {
    let bridge = RoomsBridge::new();
    let agent = "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b41";
    let account = "0199D0A8-6D8F-7B3E-9A41-5F2C8E7D1B42";

    for response in [
        answer(&bridge, agent_args(agent), true).await.unwrap(),
        answer(&bridge, agent_args(agent), false).await.unwrap(),
        invite(
            &bridge,
            InviteArgs {
                session: Some(uuid::Uuid::now_v7().to_string()),
                room: None,
                catalog: None,
                account: format!(" {account} "),
            },
        )
        .await
        .unwrap(),
        remove(&bridge, agent_args(agent)).await.unwrap(),
    ] {
        assert!(matches!(response, IpcResponse::RoomUpdated { .. }));
    }
    let managed = bridge.managed.lock().unwrap().clone();
    assert_eq!(
        managed,
        [
            IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
                agent_id: agent.into(),
                admit: true,
                catalog_id: None,
            }),
            IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
                agent_id: agent.into(),
                admit: false,
                catalog_id: None,
            }),
            // 账户 ID 照对方给的复制，大小写和前后空格不算错。
            IpcMethod::InvitePerson(IpcInvitePersonRequest {
                account_id: account.to_ascii_lowercase(),
                catalog_id: None,
            }),
            IpcMethod::RemoveAgent(IpcRemoveAgentRequest {
                agent_id: agent.into(),
                catalog_id: None,
            }),
        ]
    );
    // 不带会话不知道管哪间；agentId 不对在本机就报出来。
    let error = answer(
        &bridge,
        AgentArgs {
            session: None,
            room: None,
            catalog: None,
            agent: agent.into(),
        },
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "cli.session_required");
    let error = remove(&bridge, agent_args("Visitor")).await.unwrap_err();
    assert_eq!(error.code, "bridge.ipc.agent_id_invalid");
    assert_eq!(bridge.managed.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn 带_profile_管房间时用档案的会话() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let bridge = RoomsBridge::new();
    // 档案绑在这次运行所在的宿主任务上（设没设 CLAUDE_CODE_SESSION_ID 都行），后面的命令才认得。
    let value = create(
        &bridge,
        root,
        "test",
        None,
        host_task_id().unwrap(),
        creation("Research", None),
    )
    .await
    .unwrap();
    let key = value["profileId"].as_str().unwrap().to_owned();

    crate::access::run(
        &bridge,
        root,
        "test",
        Some(key.clone()),
        Command::Access(AccessArgs {
            session: None,
            room: None,
            catalog: None,
            replace_code: false,
            disable_code: false,
        }),
    )
    .await
    .unwrap();
    crate::access::run(
        &bridge,
        root,
        "test",
        Some(key),
        Command::Admit(AgentArgs {
            session: None,
            room: None,
            catalog: None,
            agent: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b41".into(),
        }),
    )
    .await
    .unwrap();
    // 每条命令都先接回档案的会话，再管它所在的房间。
    assert_eq!(bridge.opened.lock().unwrap().len(), 3);
    assert_eq!(bridge.actions.lock().unwrap().len(), 2);
    assert_eq!(bridge.managed.lock().unwrap().len(), 1);
}
