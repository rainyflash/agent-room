use super::*;
use agent_room_agent_client::{BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcHostSessionState, IpcHostSessionSummary, IpcInvitationOffer,
    IpcMessagePreviewSummary, IpcOpenHostSessionRequest, IpcRedeemJoinCodeRequest,
    IpcRoomMembership,
};
use std::{collections::BTreeMap, sync::Mutex};

struct Bridge {
    summary: Mutex<IpcSelfSummary>,
    rooms: Mutex<Vec<IpcRoomSummary>>,
    pending: Mutex<Option<IpcInvitationOffer>>,
    opened: Mutex<Vec<IpcOpenHostSessionRequest>>,
    /// 口令 `K7P3-Q9XW-2DMA` 对应的私人房间。
    code_room: Mutex<Option<IpcRoomSummary>>,
    redeemed: Mutex<Vec<IpcRedeemJoinCodeRequest>>,
    refuse_redeem: Mutex<Option<&'static str>>,
    /// 会话之外的调用按顺序记下方法名。
    calls: Mutex<Vec<&'static str>>,
    read_started: tokio::sync::Notify,
    finish_read: tokio::sync::Notify,
    /// 像真的消息库一样按游标给消息。第一次读会停在 `read_started`/`finish_read` 上，
    /// 好让用例在读的过程中插进一次确认。
    read_store: Mutex<Vec<IpcMessagePreviewSummary>>,
    first_read_done: std::sync::atomic::AtomicBool,
    read_cursors: Mutex<Vec<Option<String>>>,
    /// 收到的确认，和 Bridge 记下的确认位置（只往前走）。
    acks: Mutex<Vec<String>>,
    ack_position: Mutex<Option<String>>,
    /// 开好会话以后又进的房间、离开的房间（目录 ID）。
    entered: Mutex<Vec<IpcHostRoomTarget>>,
    left: Mutex<Vec<String>>,
}
impl Bridge {
    fn new() -> Self {
        Self {
            summary: Mutex::new(IpcSelfSummary {
                owner: None,
                room_catalog_id: Some(uuid::Uuid::now_v7().to_string()),
                agent: IpcAgentSummary {
                    agent_id: uuid::Uuid::now_v7().to_string(),
                    display_name: "Scout".into(),
                    matrix_user_id: "@scout:test.invalid".into(),
                    avatar_url: None,
                },
                instance_id: uuid::Uuid::now_v7().to_string(),
                matrix_device_id: "TEST".into(),
                room_id: "!room:test.invalid".into(),
                connection_state: IpcBridgeState::Ready,
                granted_capabilities: vec![],
            }),
            rooms: Mutex::new(Vec::new()),
            pending: Mutex::new(None),
            opened: Mutex::new(Vec::new()),
            code_room: Mutex::new(None),
            redeemed: Mutex::new(Vec::new()),
            refuse_redeem: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
            read_started: tokio::sync::Notify::new(),
            finish_read: tokio::sync::Notify::new(),
            read_store: Mutex::new(Vec::new()),
            first_read_done: std::sync::atomic::AtomicBool::new(false),
            read_cursors: Mutex::new(vec![]),
            acks: Mutex::new(Vec::new()),
            ack_position: Mutex::new(None),
            entered: Mutex::new(Vec::new()),
            left: Mutex::new(Vec::new()),
        }
    }

    /// 和 Bridge 一样：进别的房间，回答进的是哪一间；离开回答离没离开。
    fn membership(&self, method: IpcMethod) -> std::result::Result<IpcResponse, BridgeToolFailure> {
        match method {
            IpcMethod::EnterRoom(request) => {
                let room = agent_room_bridge_ipc::IpcSessionRoom {
                    catalog_id: request.room.catalog_id.clone(),
                    room_id: request
                        .room
                        .room_id
                        .clone()
                        .unwrap_or_else(|| "!entered:test.invalid".into()),
                };
                self.entered.lock().unwrap().push(request.room);
                Ok(IpcResponse::RoomEntered { room })
            }
            IpcMethod::LeaveRoom(request) => {
                self.left.lock().unwrap().push(request.catalog_id.clone());
                Ok(IpcResponse::RoomLeft {
                    catalog_id: request.catalog_id,
                    left: true,
                })
            }
            _ => Err(refusal("test.unexpected_call")),
        }
    }
}
impl BridgeToolClient for Bridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        if let IpcMethod::WithSession { method, .. } = &method
            && let IpcMethod::ReadInbox(request) | IpcMethod::WaitInbox(request) = method.as_ref()
        {
            self.read_cursors
                .lock()
                .unwrap()
                .push(request.after_event_id.clone());
            let previews = self.page(request);
            let first = !self
                .first_read_done
                .swap(true, std::sync::atomic::Ordering::SeqCst);
            return Box::pin(async move {
                if first {
                    self.read_started.notify_one();
                    self.finish_read.notified().await;
                }
                Ok(IpcResponse::MessagePreviews {
                    previews,
                    next_cursor: None,
                    typing: Vec::new(),
                    gaps: Vec::new(),
                })
            });
        }
        if let IpcMethod::WithSession { method, .. } = &method
            && let IpcMethod::AckInbox(request) = method.as_ref()
        {
            let response = self.acknowledge(&request.id);
            return Box::pin(async move { response });
        }
        if !matches!(method, IpcMethod::WithSession { .. }) {
            self.calls.lock().unwrap().push(method.name());
        }
        let response = match method {
            IpcMethod::ResolveJoinCode(request) => self.code_room(&request.code),
            IpcMethod::RedeemJoinCode(request) => {
                if let Some(code) = *self.refuse_redeem.lock().unwrap() {
                    Err(refusal(code))
                } else {
                    let room = self.code_room(&request.code);
                    if room.is_ok() {
                        self.redeemed.lock().unwrap().push(request);
                    }
                    room
                }
            }
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: self.rooms.lock().unwrap().clone(),
            }),
            IpcMethod::ReadInvitation => Ok(IpcResponse::Invitation {
                invitation: self.pending.lock().unwrap().clone().map(|invitation| {
                    agent_room_bridge_ipc::IpcPendingInvitation {
                        invitation,
                        expires_in_ms: 60_000,
                    }
                }),
            }),
            IpcMethod::OpenHostSession(request) => {
                // 和 Bridge 一样：用等待中的人物开出会话，这份邀请就用掉了。
                let mut pending = self.pending.lock().unwrap();
                if pending
                    .as_ref()
                    .is_some_and(|invitation| invitation.session_key == request.session_key)
                {
                    *pending = None;
                }
                drop(pending);
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
            IpcMethod::WithSession { method, .. } if matches!(*method, IpcMethod::GetSelf) => {
                Ok(IpcResponse::SelfSummary {
                    summary: self.summary.lock().unwrap().clone(),
                })
            }
            IpcMethod::WithSession { method, .. } => self.membership(*method),
            _ => Err(BridgeToolFailure::new(
                "test.unexpected_call",
                agent_room_bridge_ipc::IpcErrorCategory::Internal,
                false,
                BTreeMap::new(),
            )),
        };
        Box::pin(async move { response })
    }
}

const JOIN_CODE: &str = "K7P3-Q9XW-2DMA";

impl Bridge {
    /// 像真的消息库一样按游标给；没给位置、要从确认位置开始时，从记下的确认位置之后给。
    fn page(
        &self,
        request: &agent_room_bridge_ipc::IpcListPreviewsRequest,
    ) -> Vec<IpcMessagePreviewSummary> {
        let store = self.read_store.lock().unwrap();
        let after = request.after_event_id.clone().or_else(|| {
            request
                .from_ack
                .then(|| self.ack_position.lock().unwrap().clone())
                .flatten()
        });
        let start = after.as_ref().map_or(0, |after| {
            store
                .iter()
                .position(|preview| preview.event_id == *after)
                .map_or(store.len(), |index| index + 1)
        });
        store[start..]
            .iter()
            .take(usize::from(request.limit))
            .cloned()
            .collect()
    }

    /// 和 Bridge 一样：消息库里有这一条才能确认，位置只往前走，回还剩几条。
    fn acknowledge(&self, event: &str) -> std::result::Result<IpcResponse, BridgeToolFailure> {
        self.acks.lock().unwrap().push(event.to_owned());
        let store = self.read_store.lock().unwrap();
        let Some(index) = store.iter().position(|preview| preview.event_id == event) else {
            return Err(refusal("bridge.message_not_found"));
        };
        let mut position = self.ack_position.lock().unwrap();
        let current = position
            .as_ref()
            .and_then(|at| store.iter().position(|preview| &preview.event_id == at));
        let acknowledged = current.is_none_or(|current| index > current);
        if acknowledged {
            *position = Some(event.to_owned());
        }
        let through = current.map_or(index, |current| current.max(index));
        Ok(IpcResponse::InboxAcknowledged {
            room_id: "!room:test.invalid".into(),
            event_id: event.to_owned(),
            acknowledged,
            pending: u64::try_from(store.len() - through - 1).unwrap(),
        })
    }

    /// 和 Bridge 一样忽略大小写、空白和连字符比较口令。
    fn code_room(&self, code: &str) -> std::result::Result<IpcResponse, BridgeToolFailure> {
        let normalized: String = code
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_uppercase();
        match self.code_room.lock().unwrap().clone() {
            Some(room) if normalized == "K7P3Q9XW2DMA" => Ok(IpcResponse::JoinCodeRoom { room }),
            _ => Err(refusal("bridge.join_code.not_found")),
        }
    }
}

fn refusal(code: &str) -> BridgeToolFailure {
    BridgeToolFailure::new(
        code,
        agent_room_bridge_ipc::IpcErrorCategory::Validation,
        false,
        BTreeMap::new(),
    )
}

fn join_code(code: &str, name: Option<&str>) -> Command {
    Command::Join {
        invite: None,
        room: None,
        code: Some(code.to_owned()),
        name: name.map(str::to_owned),
    }
}

fn room(kind: IpcRoomKind, name: &str, slug: Option<&str>) -> IpcRoomSummary {
    IpcRoomSummary {
        kind,
        catalog_id: uuid::Uuid::now_v7().to_string(),
        matrix_room_id: matches!(kind, IpcRoomKind::PrivateRoom)
            .then(|| "!room:test.invalid".to_owned()),
        name: name.to_owned(),
        slug: slug.map(str::to_owned),
        membership: matches!(kind, IpcRoomKind::PrivateRoom).then_some(IpcRoomMembership::Joined),
    }
}

fn join(room: Option<&str>, name: Option<&str>) -> Command {
    Command::Join {
        invite: None,
        room: room.map(str::to_owned),
        code: None,
        name: name.map(str::to_owned),
    }
}

fn saved_profiles(root: &Path) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = std::fs::read_dir(root.join("cli-profiles"))
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
        .collect();
    profiles.sort_by(|a, b| a.invitation.session_key.cmp(&b.invitation.session_key));
    profiles
}

#[tokio::test]
async fn 按房间名接入会合成邀请_同一档案重跑复用身份_默认进公开大厅() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    let private = room(IpcRoomKind::PrivateRoom, "game dev", Some("game-dev"));
    let catalog = private.catalog_id.clone();
    bridge
        .rooms
        .lock()
        .unwrap()
        .extend([room(IpcRoomKind::PublicLobby, "Lobby", None), private]);
    run(
        &bridge,
        directory.path(),
        "test",
        None,
        join(Some("Game-Dev"), Some("Scout")),
    )
    .await
    .unwrap();
    let profiles = saved_profiles(directory.path());
    assert_eq!(profiles.len(), 1);
    let first = &profiles[0];
    assert_eq!(
        first.invitation.catalog_id.as_deref(),
        Some(catalog.as_str())
    );
    assert_eq!(
        first.invitation.room_id.as_deref(),
        Some("!room:test.invalid")
    );
    assert_eq!(first.invitation.display_name, "Scout");
    assert_eq!(first.room_id.as_deref(), Some("!room:test.invalid"));
    let key = first.invitation.session_key.clone();
    // 带 --profile 重跑同一房间：复用保存的邀请，不看默认显示名。
    run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        join(Some("game dev"), None),
    )
    .await
    .unwrap();
    assert_eq!(saved_profiles(directory.path()).len(), 1);
    {
        let opened = bridge.opened.lock().unwrap();
        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0], opened[1]);
        assert_eq!(opened[0].session_key, key);
    }
    // 同一档案进别的房间：还是这个 Agent，会话照旧开在原来的房间，开好以后再进。
    run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        join(Some("Lobby"), None),
    )
    .await
    .unwrap();
    {
        let opened = bridge.opened.lock().unwrap();
        assert_eq!(opened.len(), 3);
        assert_eq!(opened[2], opened[0]);
        let entered = bridge.entered.lock().unwrap();
        assert_eq!(entered.len(), 1);
        assert_ne!(entered[0].catalog_id, catalog);
    }
    let profiles = saved_profiles(directory.path());
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].rooms.len(), 1);
    assert_eq!(profiles[0].rooms[0].name.as_deref(), Some("Lobby"));
    assert_eq!(
        profiles[0].current_room.as_deref(),
        Some("!entered:test.invalid")
    );
    // 没有等待中的邀请、这个任务也没用过人物时，不指定房间就让 Bridge 选默认公开大厅。
    let fresh = tempfile::tempdir().unwrap();
    run(&bridge, fresh.path(), "test", None, join(None, None))
        .await
        .unwrap();
    let opened = bridge.opened.lock().unwrap();
    assert_eq!(opened.len(), 4);
    assert!(opened[3].room.is_none());
    assert!(!opened[3].display_name.is_empty());
}

#[tokio::test]
async fn 进了别的房间说话默认是它_离开它以后回到第一个房间_第一个房间离不开() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    let weekly = IpcRoomSummary {
        matrix_room_id: Some("!weekly:test.invalid".into()),
        ..room(IpcRoomKind::PrivateRoom, "周报", None)
    };
    bridge.rooms.lock().unwrap().extend([
        room(IpcRoomKind::PrivateRoom, "game dev", Some("game-dev")),
        weekly.clone(),
    ]);
    let root = directory.path();
    run(
        &bridge,
        root,
        "test",
        None,
        join(Some("game dev"), Some("Scout")),
    )
    .await
    .unwrap();
    let key = saved_profiles(root)[0].invitation.session_key.clone();
    run(
        &bridge,
        root,
        "test",
        Some(key.clone()),
        join(Some("周报"), None),
    )
    .await
    .unwrap();
    let mut saved = load(root, &key);
    assert_eq!(saved.room_name.as_deref(), Some("game dev"));
    assert_eq!(saved.current_room.as_deref(), Some("!weekly:test.invalid"));

    // 说话、看前后不给 --room 就是最近进的房间；也能按名字说第一个房间。
    let mut send = <crate::cli::Cli as clap::Parser>::try_parse_from([
        "agent-room",
        "send",
        "--text",
        "hi",
        "--submission-id",
        "x",
        "--authorized",
    ])
    .unwrap()
    .command;
    apply_context(&mut send, &saved).unwrap();
    let Command::Send(args) = send else {
        panic!("是发消息");
    };
    assert_eq!(args.room.as_deref(), Some("!weekly:test.invalid"));
    let mut asked = Some("GAME DEV".to_owned());
    let mut session = None;
    set_scope(&mut session, Some(&mut asked), RoomScope::Current, &saved).unwrap();
    assert_eq!(asked.as_deref(), Some("!room:test.invalid"));
    asked = None;
    set_scope(&mut session, Some(&mut asked), RoomScope::Every, &saved).unwrap();
    assert_eq!(asked, None, "等消息不给房间就是所有房间");

    // 第一个房间离不开；离开周报以后回到第一个房间。
    let leave = |room: &str| Command::Leave {
        room: Some(room.to_owned()),
    };
    assert_eq!(
        run(&bridge, root, "test", Some(key.clone()), leave("game dev"))
            .await
            .unwrap_err()
            .code,
        "cli.leave_session_room"
    );
    run(
        &bridge,
        root,
        "test",
        Some(key.clone()),
        leave("!weekly:test.invalid"),
    )
    .await
    .unwrap();
    assert_eq!(
        bridge.left.lock().unwrap().as_slice(),
        std::slice::from_ref(&weekly.catalog_id)
    );
    saved = load(root, &key);
    assert!(saved.rooms.is_empty());
    assert_eq!(saved.current_room, None);
    assert_eq!(
        saved.current().room_id.as_deref(),
        Some("!room:test.invalid")
    );
}

#[tokio::test]
async fn 凭口令接入先查看房间_存好身份后兑换再开会话_同一档案重跑回到同一人物() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    let project = IpcRoomSummary {
        kind: IpcRoomKind::PrivateRoom,
        catalog_id: uuid::Uuid::now_v7().to_string(),
        matrix_room_id: Some("!room:test.invalid".to_owned()),
        name: "项目室".to_owned(),
        slug: None,
        membership: None,
    };
    *bridge.code_room.lock().unwrap() = Some(project.clone());
    run(
        &bridge,
        directory.path(),
        "test",
        None,
        join_code("k7p3 q9xw 2dma", Some("Scout")),
    )
    .await
    .unwrap();
    let profiles = saved_profiles(directory.path());
    assert_eq!(profiles.len(), 1);
    let saved = &profiles[0].invitation;
    assert_eq!(
        saved.catalog_id.as_deref(),
        Some(project.catalog_id.as_str())
    );
    assert_eq!(saved.room_id.as_deref(), Some("!room:test.invalid"));
    assert_eq!(saved.display_name, "Scout");
    assert_eq!(
        bridge.redeemed.lock().unwrap().as_slice(),
        &[IpcRedeemJoinCodeRequest {
            session_key: saved.session_key.clone(),
            display_name: "Scout".to_owned(),
            code: "k7p3 q9xw 2dma".to_owned(),
        }]
    );
    assert_eq!(bridge.opened.lock().unwrap().as_slice(), &[saved.request()]);
    assert_eq!(
        bridge.calls.lock().unwrap().as_slice(),
        &["resolve_join_code", "redeem_join_code", "open_host_session"]
    );
    // 同一档案再凭口令接入：还是这个人物，再兑换一次也没关系。
    run(
        &bridge,
        directory.path(),
        "test",
        Some(saved.session_key.clone()),
        join_code(JOIN_CODE, None),
    )
    .await
    .unwrap();
    assert_eq!(saved_profiles(directory.path()).len(), 1);
    assert_eq!(bridge.opened.lock().unwrap()[1], saved.request());
    assert_eq!(
        bridge.redeemed.lock().unwrap()[1].session_key,
        saved.session_key
    );
}

#[tokio::test]
async fn 口令不对时什么都不留_兑换被拒时身份已存好_重试回到同一人物() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    *bridge.code_room.lock().unwrap() = Some(IpcRoomSummary {
        kind: IpcRoomKind::PrivateRoom,
        catalog_id: uuid::Uuid::now_v7().to_string(),
        matrix_room_id: Some("!room:test.invalid".to_owned()),
        name: "项目室".to_owned(),
        slug: None,
        membership: None,
    });
    let wrong = run(
        &bridge,
        directory.path(),
        "test",
        None,
        join_code("0000-0000-0000", Some("Scout")),
    )
    .await
    .unwrap_err();
    assert_eq!(wrong.code, "bridge.join_code.not_found");
    assert!(wrong.hint.contains("Do not guess"), "{}", wrong.hint);
    assert!(!directory.path().join("cli-profiles").exists());
    *bridge.refuse_redeem.lock().unwrap() = Some("bridge.join_code.forbidden");
    let refused = run(
        &bridge,
        directory.path(),
        "test",
        None,
        join_code(JOIN_CODE, Some("Scout")),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, "bridge.join_code.forbidden");
    assert!(
        bridge.opened.lock().unwrap().is_empty(),
        "兑换被拒就不开会话"
    );
    let saved = saved_profiles(directory.path());
    assert_eq!(saved.len(), 1);
    *bridge.refuse_redeem.lock().unwrap() = None;
    run(
        &bridge,
        directory.path(),
        "test",
        Some(saved[0].invitation.session_key.clone()),
        join_code(JOIN_CODE, Some("Scout")),
    )
    .await
    .unwrap();
    assert_eq!(saved_profiles(directory.path()).len(), 1);
    assert_eq!(
        bridge.opened.lock().unwrap()[0].session_key,
        saved[0].invitation.session_key
    );
}

#[tokio::test]
async fn 房间名不存在或有歧义时不接入并列出候选() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    bridge.rooms.lock().unwrap().extend([
        room(IpcRoomKind::PublicLobby, "Lobby", Some("lobby")),
        room(IpcRoomKind::PrivateRoom, "ops", None),
        room(IpcRoomKind::PrivateRoom, "Ops", None),
    ]);
    let missing = run(
        &bridge,
        directory.path(),
        "test",
        None,
        join(Some("missing"), None),
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "cli.room_not_found");
    assert!(missing.details["available"].contains("Lobby [lobby] (public lobby"));
    let ambiguous = run(
        &bridge,
        directory.path(),
        "test",
        None,
        join(Some("OPS"), None),
    )
    .await
    .unwrap_err();
    assert_eq!(ambiguous.code, "cli.room_ambiguous");
    assert!(ambiguous.details["candidates"].contains("ops (private room"));
    assert!(ambiguous.details["candidates"].contains("Ops (private room"));
    assert!(bridge.opened.lock().unwrap().is_empty());
    assert!(!directory.path().join("cli-profiles").exists());
}

#[tokio::test]
async fn 只说接入时接上桌面面板正在等的人物_用掉后不再重复接() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    let waiting = IpcInvitationOffer {
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: Some("面板里起的名字".into()),
        room: Some(IpcHostRoomTarget {
            catalog_id: uuid::Uuid::now_v7().to_string(),
            room_id: Some("!room:test.invalid".into()),
        }),
    };
    *bridge.pending.lock().unwrap() = Some(waiting.clone());
    run(&bridge, directory.path(), "test", None, join(None, None))
        .await
        .unwrap();
    assert_eq!(
        bridge.opened.lock().unwrap().as_slice(),
        &[waiting.open_request(|| unreachable!("面板定了名字"))]
    );
    let saved = saved_profiles(directory.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].invitation.session_key, waiting.session_key);
    assert_eq!(saved[0].invitation.display_name, "面板里起的名字");
    assert_eq!(
        saved[0].invitation.catalog_id,
        waiting.room.as_ref().map(|room| room.catalog_id.clone())
    );
    assert!(bridge.pending.lock().unwrap().is_none());
    // 面板没定名字：带上自己起的名字说“接入”，接上的就是这份邀请，名字用 Agent 起的。
    let unnamed = IpcInvitationOffer {
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: None,
        room: None,
    };
    *bridge.pending.lock().unwrap() = Some(unnamed.clone());
    run(
        &bridge,
        directory.path(),
        "test",
        None,
        join(None, Some("Scout")),
    )
    .await
    .unwrap();
    let opened = bridge.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[1].session_key, unnamed.session_key);
    assert_eq!(opened[1].display_name, "Scout");
    assert!(bridge.pending.lock().unwrap().is_none());
    // 指了房间就是按名字接入，不去接面板的邀请。
    bridge
        .rooms
        .lock()
        .unwrap()
        .push(room(IpcRoomKind::PublicLobby, "Agent Room Global", None));
    *bridge.pending.lock().unwrap() = Some(unnamed.clone());
    run(
        &bridge,
        directory.path(),
        "test",
        None,
        join(Some("Agent Room Global"), Some("Pilot")),
    )
    .await
    .unwrap();
    let opened = bridge.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 3);
    assert_ne!(opened[2].session_key, unnamed.session_key);
    assert_eq!(opened[2].display_name, "Pilot");
    assert!(bridge.pending.lock().unwrap().is_some());
}

#[tokio::test]
async fn 带名字接入时这个任务已有同名人物就回到它_面板的邀请留给别的_agent() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = Bridge::new();
    let task = uuid::Uuid::now_v7().to_string();
    let saved = Profile::new(
        Invitation::for_room(
            uuid::Uuid::now_v7().to_string(),
            "Scout".into(),
            &RoomTarget::DEFAULT_LOBBY,
        ),
        "test",
        Some(task.clone()),
    );
    ProfileStore::open(directory.path(), &saved.invitation.session_key)
        .unwrap()
        .save(&saved)
        .unwrap();
    let waiting = IpcInvitationOffer {
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: None,
        room: None,
    };
    *bridge.pending.lock().unwrap() = Some(waiting.clone());
    let select = |name: Option<&str>| {
        let command = join(None, name);
        let bridge = &bridge;
        let root = directory.path();
        let task = task.clone();
        async move { select_identity(bridge, root, "test", None, Some(&task), &command).await }
    };
    let returning = select(Some("Scout")).await.unwrap();
    assert_eq!(returning.key, saved.invitation.session_key);
    assert!(returning.invitation.is_none() && returning.join.is_none());
    // 换个名字就是新人物：接上面板的邀请，名字用自己起的。
    let invited = select(Some("Pilot")).await.unwrap();
    assert_eq!(invited.key, waiting.session_key);
    assert_eq!(invited.invitation.unwrap().display_name, "Pilot");
    // 不带名字、任务里已经有 Agent：面板没说房间时回到它自己，邀请留给别的 Agent。
    let bare = select(None).await.unwrap();
    assert_eq!(bare.key, saved.invitation.session_key);
    assert!(bare.invitation.is_none() && bare.join.is_none());
    // 面板在某个房间等：还是这个 Agent，带它进面板的房间。
    let weekly = IpcHostRoomTarget {
        catalog_id: uuid::Uuid::now_v7().to_string(),
        room_id: Some("!weekly:test.invalid".into()),
    };
    *bridge.pending.lock().unwrap() = Some(IpcInvitationOffer {
        room: Some(weekly.clone()),
        ..waiting.clone()
    });
    let entering = select(None).await.unwrap();
    assert_eq!(entering.key, saved.invitation.session_key);
    assert!(entering.invitation.is_none());
    let target = entering.join.unwrap().target;
    assert_eq!(
        target.catalog_id.as_deref(),
        Some(weekly.catalog_id.as_str())
    );
    assert_eq!(target.room_id, weekly.room_id);
    *bridge.pending.lock().unwrap() = None;
    assert_eq!(
        select(None).await.unwrap().key,
        saved.invitation.session_key
    );
}

#[test]
fn 只说接入且没有等待中的邀请时回到这个任务最近用过的人物() {
    let directory = tempfile::tempdir().unwrap();
    let task = uuid::Uuid::now_v7().to_string();
    let target = RoomTarget {
        catalog_id: Some(uuid::Uuid::now_v7().to_string()),
        room_id: Some("!room:test.invalid".into()),
        name: None,
    };
    let mut profiles = Vec::new();
    for (name, room) in [("Lobby", &RoomTarget::DEFAULT_LOBBY), ("Game", &target)] {
        let profile = Profile::new(
            Invitation::for_room(uuid::Uuid::now_v7().to_string(), name.into(), room),
            "test",
            Some(task.clone()),
        );
        ProfileStore::open(directory.path(), &profile.invitation.session_key)
            .unwrap()
            .save(&profile)
            .unwrap();
        profiles.push(profile);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        ProfileStore::latest_bound(directory.path(), "test", &task, None),
        Some(profiles[1].invitation.session_key.clone())
    );
    // 早先的人物刚被用过（例如 ack），就轮到它。
    std::thread::sleep(Duration::from_millis(20));
    ProfileStore::open(directory.path(), &profiles[0].invitation.session_key)
        .unwrap()
        .save(&profiles[0])
        .unwrap();
    assert_eq!(
        ProfileStore::latest_bound(directory.path(), "test", &task, None),
        Some(profiles[0].invitation.session_key.clone())
    );
    // 带了名字只找叫这个名字的人物。
    assert_eq!(
        ProfileStore::latest_bound(directory.path(), "test", &task, Some("Game")),
        Some(profiles[1].invitation.session_key.clone())
    );
    assert_eq!(
        ProfileStore::latest_bound(directory.path(), "test", &task, Some("Other")),
        None
    );
    assert_eq!(
        ProfileStore::latest_bound(directory.path(), "other", &task, None),
        None
    );
    assert_eq!(
        ProfileStore::latest_bound(
            directory.path(),
            "test",
            &uuid::Uuid::now_v7().to_string(),
            None
        ),
        None
    );
}

#[test]
fn 同一宿主任务为同一房间保存过的身份会被找回() {
    let directory = tempfile::tempdir().unwrap();
    let target = RoomTarget {
        catalog_id: Some(uuid::Uuid::now_v7().to_string()),
        room_id: Some("!room:test.invalid".into()),
        name: None,
    };
    let task = uuid::Uuid::now_v7().to_string();
    let mut older = Profile::new(
        Invitation::for_room(uuid::Uuid::now_v7().to_string(), "Scout".into(), &target),
        "test",
        Some(task.clone()),
    );
    older.session_id = None;
    let newer = Profile::new(
        Invitation::for_room(uuid::Uuid::now_v7().to_string(), "Scout".into(), &target),
        "test",
        Some(task.clone()),
    );
    let other_task = Profile::new(
        Invitation::for_room(uuid::Uuid::now_v7().to_string(), "Scout".into(), &target),
        "test",
        Some(uuid::Uuid::now_v7().to_string()),
    );
    let lobby = Profile::new(
        Invitation::for_room(
            uuid::Uuid::now_v7().to_string(),
            "Scout".into(),
            &RoomTarget::DEFAULT_LOBBY,
        ),
        "test",
        Some(task.clone()),
    );
    for profile in [&older, &newer, &other_task, &lobby] {
        ProfileStore::open(directory.path(), &profile.invitation.session_key)
            .unwrap()
            .save(profile)
            .unwrap();
    }
    std::fs::write(
        directory.path().join("cli-profiles").join("broken.json"),
        b"not a profile",
    )
    .unwrap();
    assert_eq!(
        ProfileStore::find_bound(directory.path(), "test", &task, &target, None),
        Some(newer.invitation.session_key.clone())
    );
    assert_eq!(
        ProfileStore::find_bound(directory.path(), "test", &task, &target, Some("Scout")),
        Some(newer.invitation.session_key.clone())
    );
    assert_eq!(
        ProfileStore::find_bound(directory.path(), "test", &task, &target, Some("Other")),
        None
    );
    assert_eq!(
        ProfileStore::find_bound(directory.path(), "other", &task, &target, None),
        None
    );
    assert_eq!(
        ProfileStore::find_bound(
            directory.path(),
            "test",
            &task,
            &RoomTarget::DEFAULT_LOBBY,
            None
        ),
        Some(lobby.invitation.session_key.clone())
    );
    assert_eq!(
        ProfileStore::find_bound(
            directory.path(),
            "test",
            &other_task.task_id.clone().unwrap(),
            &target,
            None
        ),
        Some(other_task.invitation.session_key.clone())
    );
}

fn profile() -> Profile {
    Profile::new(
        Invitation {
            catalog_id: None,
            version: 1,
            session_key: uuid::Uuid::now_v7().to_string(),
            display_name: "Scout".into(),
            room_id: Some("!room:test.invalid".into()),
        },
        "test",
        None,
    )
}

#[tokio::test]
async fn 重启产生新会话句柄但保留人物() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    let store = ProfileStore::open(directory.path(), &saved.invitation.session_key).unwrap();
    let bridge = Bridge::new();
    let first = connect(&bridge, &store, &mut saved).await.unwrap();
    let first_session = saved.session_id.clone();
    let mut restored = store.load().unwrap().unwrap();
    let second = connect(&bridge, &store, &mut restored).await.unwrap();
    assert_eq!(first.agent, second.agent);
    assert_ne!(first_session, restored.session_id);
    assert_eq!(
        bridge.opened.lock().unwrap().as_slice(),
        [saved.invitation.request(), saved.invitation.request()]
    );
}

#[tokio::test]
async fn 默认房间不同不能报邀请成功() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    let store = ProfileStore::open(directory.path(), &saved.invitation.session_key).unwrap();
    let bridge = Bridge::new();
    bridge.summary.lock().unwrap().room_id = "!other:test.invalid".into();
    let error = connect(&bridge, &store, &mut saved).await.unwrap_err();
    assert_eq!(error.code, "cli.invitation.room_mismatch");
    assert_eq!(error.details["actualRoomId"], "!other:test.invalid");
    assert!(saved.room_id.is_none());
}

#[tokio::test]
async fn 账号切换返回不同人物时拒绝覆盖原身份() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    let store = ProfileStore::open(directory.path(), &saved.invitation.session_key).unwrap();
    let bridge = Bridge::new();
    connect(&bridge, &store, &mut saved).await.unwrap();
    let original = saved.agent_id.clone();
    bridge.summary.lock().unwrap().agent.agent_id = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        connect(&bridge, &store, &mut saved).await.unwrap_err().code,
        "cli.profile.identity_changed"
    );
    assert_eq!(store.load().unwrap().unwrap().agent_id, original);
}

#[test]
fn 自动上下文不能覆盖显式的其他会话或房间() {
    let mut saved = profile();
    saved.session_id = Some(uuid::Uuid::now_v7().to_string());
    saved.room_id = saved.invitation.room_id.clone();
    let mut session = None;
    let mut room = None;
    set_scope(&mut session, Some(&mut room), RoomScope::Current, &saved).unwrap();
    assert_eq!(session, saved.session_id);
    assert_eq!(room, saved.room_id);
    // 等消息不给房间就是所有房间。
    room = None;
    set_scope(&mut session, Some(&mut room), RoomScope::Every, &saved).unwrap();
    assert_eq!(room, None);
    session = Some(uuid::Uuid::now_v7().to_string());
    assert!(set_scope(&mut session, Some(&mut room), RoomScope::Current, &saved).is_err());
    // 给了别的 Matrix 房间照原样交给 Bridge（主人带它进去的房间档案里没记）；名字要是它在的房间。
    session = None;
    room = Some("!other:test.invalid".into());
    set_scope(&mut session, Some(&mut room), RoomScope::Current, &saved).unwrap();
    assert_eq!(room.as_deref(), Some("!other:test.invalid"));
    room = Some("别处".into());
    assert_eq!(
        set_scope(&mut session, Some(&mut room), RoomScope::Current, &saved)
            .unwrap_err()
            .code,
        "cli.room_not_found"
    );
}

#[test]
fn 按需查看的命令带上档案的会话和房间() {
    use clap::Parser as _;
    let parse = |args: &[&str]| {
        crate::cli::Cli::try_parse_from(std::iter::once("agent-room").chain(args.iter().copied()))
            .map(|cli| cli.command)
    };
    let mut saved = profile();
    saved.session_id = Some(uuid::Uuid::now_v7().to_string());
    saved.room_id = saved.invitation.room_id.clone();

    let mut show = parse(&["show", "--id", "$one", "--id", "$two"]).unwrap();
    apply_context(&mut show, &saved).unwrap();
    let Command::Show(args) = show else {
        panic!("show 是按 ID 取");
    };
    let (session, IpcMethod::GetMessages(request)) = args.into_request() else {
        panic!("show 是按 ID 取");
    };
    assert_eq!(session, saved.session_id);
    assert_eq!(request.ids, ["$one", "$two"]);

    let mut around = parse(&["around", "--id", "$one", "--before", "3"]).unwrap();
    apply_context(&mut around, &saved).unwrap();
    let Command::Around(args) = around else {
        panic!("around 是看前后");
    };
    let (_, IpcMethod::MessagesAround(request)) = args.into_request() else {
        panic!("around 是看前后");
    };
    assert_eq!(request.room_id, saved.room_id);
    assert_eq!((request.before, request.after), (3, 10), "后面默认 10 条");

    let mut history =
        parse(&["history", "--before", "$one", "--from", "Ada", "--mentions"]).unwrap();
    apply_context(&mut history, &saved).unwrap();
    let Command::History(args) = history else {
        panic!("history 是往前翻");
    };
    let (_, IpcMethod::RoomHistory(request)) = args.into_request() else {
        panic!("history 是往前翻");
    };
    assert_eq!(request.room_id, saved.room_id);
    assert_eq!(request.before.as_deref(), Some("$one"));
    assert_eq!(request.from.as_deref(), Some("Ada"));
    assert!(request.mentions_me);
    assert_eq!(request.limit, 20, "默认 20 条");

    // 后来进的房间按名字找；不认得的名字说找不到。往哪个方向翻只能选一个。
    saved.remember(EnteredRoom {
        catalog_id: uuid::Uuid::now_v7().to_string(),
        room_id: Some("!weekly:test.invalid".into()),
        name: Some("周报".into()),
        created: false,
    });
    let mut weekly = parse(&["history", "--room", "周报"]).unwrap();
    apply_context(&mut weekly, &saved).unwrap();
    let Command::History(args) = weekly else {
        panic!("history 是往前翻");
    };
    assert_eq!(args.scope.room.as_deref(), Some("!weekly:test.invalid"));
    let mut elsewhere = parse(&["history", "--room", "别处"]).unwrap();
    assert_eq!(
        apply_context(&mut elsewhere, &saved).unwrap_err().code,
        "cli.room_not_found"
    );
    assert!(parse(&["history", "--before", "$a", "--after", "$b"]).is_err());
    assert!(parse(&["around", "--id", "$a", "--after", "21"]).is_err());
}

fn read_args(wait: Option<u32>) -> ReadArgs {
    ReadArgs {
        session: None,
        room: None,
        after: None,
        limit: 20,
        wait,
        wake: None,
        from: Vec::new(),
        wait_for: Vec::new(),
        reply_to: None,
        settle: Some(0),
        digest: None,
        mentions_only: false,
    }
}

/// 存好档案，消息库里放上这几条，第一次读不要停。
fn prepared(saved: &Profile, events: &[&str]) -> (tempfile::TempDir, Bridge) {
    let directory = tempfile::tempdir().unwrap();
    ProfileStore::open(directory.path(), &saved.invitation.session_key)
        .unwrap()
        .save(saved)
        .unwrap();
    let bridge = Bridge::new();
    bridge
        .first_read_done
        .store(true, std::sync::atomic::Ordering::SeqCst);
    bridge
        .read_store
        .lock()
        .unwrap()
        .extend(events.iter().map(|event| preview(event)));
    (directory, bridge)
}

fn load(directory: &Path, key: &str) -> Profile {
    ProfileStore::open(directory, key)
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn 旧版档案里的确认位置第一次连上时交给_bridge_之后从那里接着读() {
    let mut saved = profile();
    saved.after_event_id = Some("$one".into());
    saved.delivered = vec!["$two".into()];
    let key = saved.invitation.session_key.clone();
    let (directory, bridge) = prepared(&saved, &["$one", "$two", "$three"]);

    run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        Command::Read(read_args(Some(0))),
    )
    .await
    .unwrap();

    assert_eq!(bridge.acks.lock().unwrap().as_slice(), ["$one"]);
    // 读的时候没带位置：Bridge 从确认位置之后给，没确认的那条照样在。
    assert_eq!(bridge.read_cursors.lock().unwrap().first(), Some(&None));
    let moved = load(directory.path(), &key);
    assert_eq!(moved.after_event_id, None);
    assert!(moved.delivered.is_empty());

    // 搬过一次就不再搬。
    run(
        &bridge,
        directory.path(),
        "test",
        Some(key),
        Command::Read(read_args(Some(0))),
    )
    .await
    .unwrap();
    assert_eq!(bridge.acks.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 旧版位置在消息库里找不到时当没确认过_不挡读消息() {
    let mut saved = profile();
    saved.after_event_id = Some("$gone".into());
    let key = saved.invitation.session_key.clone();
    let (directory, bridge) = prepared(&saved, &["$one"]);

    run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        Command::Read(read_args(Some(0))),
    )
    .await
    .unwrap();

    assert_eq!(bridge.acks.lock().unwrap().as_slice(), ["$gone"]);
    assert_eq!(load(directory.path(), &key).after_event_id, None);
}

#[tokio::test]
async fn 确认交给_bridge_只往前走_找不到的报出来() {
    let saved = profile();
    let key = saved.invitation.session_key.clone();
    let (directory, bridge) = prepared(&saved, &["$one", "$two", "$three"]);
    let ack = |event: &str| {
        run(
            &bridge,
            directory.path(),
            "test",
            Some(key.clone()),
            Command::Ack {
                event: event.to_owned(),
            },
        )
    };

    ack("$two").await.unwrap();
    ack("$one").await.unwrap();
    assert_eq!(
        bridge.ack_position.lock().unwrap().as_deref(),
        Some("$two"),
        "往回确认不动位置"
    );
    assert_eq!(
        ack("$nowhere").await.unwrap_err().code,
        "bridge.message_not_found"
    );
    assert_eq!(
        bridge.acks.lock().unwrap().as_slice(),
        ["$two", "$one", "$nowhere"]
    );
}

#[tokio::test]
async fn 等消息的时候可以确认_确认不占档案的写锁() {
    let saved = profile();
    let key = saved.invitation.session_key.clone();
    let (directory, bridge) = prepared(&saved, &["$one"]);
    bridge
        .first_read_done
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let reading = Box::pin(run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        Command::Read(read_args(Some(0))),
    ));
    let acknowledging = async {
        bridge.read_started.notified().await;
        Box::pin(run(
            &bridge,
            directory.path(),
            "test",
            Some(key.clone()),
            Command::Ack {
                event: "$one".into(),
            },
        ))
        .await
        .unwrap();
        bridge.finish_read.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(reading, acknowledging)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(bridge.ack_position.lock().unwrap().as_deref(), Some("$one"));
}

fn preview(event: &str) -> IpcMessagePreviewSummary {
    use agent_room_bridge_ipc::{IpcActorSummary, IpcContentReference, IpcMessageSensitivity};
    IpcMessagePreviewSummary {
        event_id: event.into(),
        message_id: uuid::Uuid::now_v7().to_string(),
        room_id: "!room:test.invalid".into(),
        conversation: None,
        reply_to_message_id: None,
        actor: IpcActorSummary::Human {
            principal_id: uuid::Uuid::now_v7().to_string(),
            display_name: "Owner".into(),
            matrix_user_id: "@owner:test.invalid".into(),
            avatar_url: None,
        },
        created_at_unix_ms: 1,
        title: "message".into(),
        summary: "message".into(),
        content: IpcContentReference {
            content_id: uuid::Uuid::now_v7().to_string(),
            digest_sha256: "0".repeat(64),
            media_type: "text/plain".into(),
            size_bytes: 7,
        },
        language: None,
        sensitivity: IpcMessageSensitivity::Normal,
        risk_flags: vec![],
        reply_to: None,
        from_me: false,
        mentions_me: false,
        mentions_everyone: false,
        room_name: None,
        before_join: false,
    }
}
