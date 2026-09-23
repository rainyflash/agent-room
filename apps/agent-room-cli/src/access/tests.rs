use super::*;
use agent_room_agent_client::{BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcAgentSummary, IpcHostSessionState, IpcHostSessionSummary, IpcOpenHostSessionRequest,
    IpcRoomMembership,
};
use std::{collections::BTreeMap, sync::Mutex};

struct Bridge {
    summary: Mutex<IpcSelfSummary>,
    rooms: Mutex<Vec<IpcRoomSummary>>,
    opened: Mutex<Vec<IpcOpenHostSessionRequest>>,
    read_started: tokio::sync::Notify,
    finish_read: tokio::sync::Notify,
    read_pages:
        Mutex<std::collections::VecDeque<Vec<agent_room_bridge_ipc::IpcMessagePreviewSummary>>>,
    read_cursors: Mutex<Vec<Option<String>>>,
}
impl Bridge {
    fn new() -> Self {
        Self {
            summary: Mutex::new(IpcSelfSummary {
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
            opened: Mutex::new(Vec::new()),
            read_started: tokio::sync::Notify::new(),
            finish_read: tokio::sync::Notify::new(),
            read_pages: Mutex::new(std::collections::VecDeque::new()),
            read_cursors: Mutex::new(vec![]),
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
            return Box::pin(async move {
                self.read_started.notify_one();
                self.finish_read.notified().await;
                Ok(IpcResponse::MessagePreviews {
                    previews: self
                        .read_pages
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_default(),
                    next_cursor: None,
                })
            });
        }
        let response = match method {
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: self.rooms.lock().unwrap().clone(),
            }),
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
            IpcMethod::WithSession { method, .. } if matches!(*method, IpcMethod::GetSelf) => {
                Ok(IpcResponse::SelfSummary {
                    summary: self.summary.lock().unwrap().clone(),
                })
            }
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
    // 同一档案换房间不能悄悄换掉身份。
    assert_eq!(
        run(
            &bridge,
            directory.path(),
            "test",
            Some(key.clone()),
            join(Some("Lobby"), None),
        )
        .await
        .unwrap_err()
        .code,
        "cli.profile.invitation_mismatch"
    );
    // 不指定房间就让 Bridge 选默认公开大厅。
    run(&bridge, directory.path(), "test", None, join(None, None))
        .await
        .unwrap();
    let opened = bridge.opened.lock().unwrap();
    assert_eq!(opened.len(), 3);
    assert!(opened[2].room.is_none());
    assert!(!opened[2].display_name.is_empty());
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

#[test]
fn 同一宿主任务为同一房间保存过的身份会被找回() {
    let directory = tempfile::tempdir().unwrap();
    let target = RoomTarget {
        catalog_id: Some(uuid::Uuid::now_v7().to_string()),
        room_id: Some("!room:test.invalid".into()),
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
async fn 重启产生新会话句柄但保留人物与消息进度() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    let store = ProfileStore::open(directory.path(), &saved.invitation.session_key).unwrap();
    let bridge = Bridge::new();
    let first = connect(&bridge, &store, &mut saved).await.unwrap();
    let first_session = saved.session_id.clone();
    saved
        .record_delivery(["$one".into(), "$two".into()])
        .unwrap();
    saved.acknowledge("$one").unwrap();
    store.save(&saved).unwrap();
    let mut restored = store.load().unwrap().unwrap();
    let second = connect(&bridge, &store, &mut restored).await.unwrap();
    assert_eq!(first.agent, second.agent);
    assert_ne!(first_session, restored.session_id);
    assert_eq!(restored.after_event_id.as_deref(), Some("$one"));
    assert_eq!(restored.delivered, ["$two"]);
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
    set_scope(&mut session, Some(&mut room), &saved).unwrap();
    assert_eq!(session, saved.session_id);
    assert_eq!(room, saved.room_id);
    session = Some(uuid::Uuid::now_v7().to_string());
    assert!(set_scope(&mut session, Some(&mut room), &saved).is_err());
    session = None;
    room = Some("!other:test.invalid".into());
    assert!(set_scope(&mut session, Some(&mut room), &saved).is_err());
}

#[tokio::test]
async fn 等待消息期间可确认已处理批次且返回空批次不覆盖并发确认() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    saved.record_delivery(["$handled".into()]).unwrap();
    let key = saved.invitation.session_key.clone();
    ProfileStore::open(directory.path(), &key)
        .unwrap()
        .save(&saved)
        .unwrap();
    let bridge = Bridge::new();
    let reading = Box::pin(run(
        &bridge,
        directory.path(),
        "test",
        Some(key.clone()),
        Command::Read(ReadArgs {
            session: None,
            room: None,
            after: None,
            limit: 20,
            wait: Some(0),
        }),
    ));
    let acknowledging = async {
        bridge.read_started.notified().await;
        let store = ProfileStore::open(directory.path(), &key).expect("取信等待不能占用进度写锁");
        let mut current = store.load().unwrap().unwrap();
        current.acknowledge("$handled").unwrap();
        store.save(&current).unwrap();
        drop(store);
        bridge.finish_read.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(reading, acknowledging)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(
        ProfileStore::open(directory.path(), &key)
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .after_event_id
            .as_deref(),
        Some("$handled")
    );
}

#[tokio::test]
async fn 并发确认吞掉整批消息时默认read会接着等下一批() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = profile();
    saved.session_id = Some(uuid::Uuid::now_v7().to_string());
    saved.record_delivery(["$handled".into()]).unwrap();
    let key = saved.invitation.session_key.clone();
    ProfileStore::open(directory.path(), &key)
        .unwrap()
        .save(&saved)
        .unwrap();
    let args = ReadArgs {
        session: saved.session_id.clone(),
        room: None,
        after: None,
        limit: 20,
        wait: None,
    };
    let bridge = Bridge::new();
    bridge
        .read_pages
        .lock()
        .unwrap()
        .extend([vec![preview("$handled")], vec![preview("$new")]]);
    let reading = read_batch(&bridge, directory.path(), &mut saved, args);
    let delivering = async {
        bridge.read_started.notified().await;
        let store = ProfileStore::open(directory.path(), &key).unwrap();
        let mut current = store.load().unwrap().unwrap();
        current.acknowledge("$handled").unwrap();
        store.save(&current).unwrap();
        drop(store);
        bridge.finish_read.notify_one();
        bridge.read_started.notified().await;
        assert_eq!(
            *bridge.read_cursors.lock().unwrap(),
            [None, Some("$handled".into())]
        );
        bridge.finish_read.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(reading, delivering)
    })
    .await
    .unwrap();
    let IpcResponse::MessagePreviews { previews, .. } = result.unwrap().unwrap() else {
        panic!("message page")
    };
    assert_eq!(previews.len(), 1);
    assert_eq!(previews[0].event_id, "$new");
    assert_eq!(saved.after_event_id.as_deref(), Some("$handled"));
    assert_eq!(saved.delivered, ["$new"]);
}

#[tokio::test]
async fn 并发确认后旧批次不能重新进入待处理队列或倒退游标() {
    for events in [vec!["$one"], vec!["$one", "$two", "$three"], vec!["$three"]] {
        let directory = tempfile::tempdir().unwrap();
        let mut before = profile();
        before
            .record_delivery(["$one".into(), "$two".into()])
            .unwrap();
        let mut current = before.clone();
        current.acknowledge("$two").unwrap();
        ProfileStore::open(directory.path(), &before.invitation.session_key)
            .unwrap()
            .save(&current)
            .unwrap();
        let mut response = IpcResponse::MessagePreviews {
            previews: events.iter().map(|id| preview(id)).collect(),
            next_cursor: None,
        };
        let cursor = persist_delivery(directory.path(), &mut before, &mut response)
            .await
            .unwrap();
        let IpcResponse::MessagePreviews { previews, .. } = response else {
            panic!("消息响应")
        };
        let has_new = events.contains(&"$three");
        assert_eq!(
            previews
                .iter()
                .map(|message| message.event_id.as_str())
                .collect::<Vec<_>>(),
            if has_new { vec!["$three"] } else { vec![] }
        );
        assert_eq!(
            cursor.as_deref(),
            Some(if has_new { "$three" } else { "$two" })
        );
        assert_eq!(before.after_event_id.as_deref(), Some("$two"));
        assert!(before.acknowledge("$one").is_err());
        let restored = ProfileStore::open(directory.path(), &before.invitation.session_key)
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(restored.delivered, before.delivered);
        assert_eq!(restored.after_event_id, before.after_event_id);
    }
}

fn preview(event: &str) -> agent_room_bridge_ipc::IpcMessagePreviewSummary {
    use agent_room_bridge_ipc::{IpcActorSummary, IpcContentReference, IpcMessageSensitivity};
    agent_room_bridge_ipc::IpcMessagePreviewSummary {
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
    }
}
