//! 建房间、管房间的工具（`specs/agent-rooms/design.md` 第 3 步），走真实的 MCP 传输。

use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCreateRoomRequest, IpcInvitePersonRequest, IpcParentRoom,
    IpcPublicRoomJoinMode, IpcRemoveAgentRequest, IpcRoomAccess, IpcRoomAccessAction,
    IpcRoomAccessRequest, IpcRoomCode, IpcRoomKnock,
};

use super::*;

const TASK: &str = "01990d9e-8400-7000-8000-0000000000a1";
const OTHER_TASK: &str = "01990d9e-8400-7000-8000-0000000000a2";
const CREATED_ROOM: &str = "!created:test.invalid";
const GUIDE: &str = "https://api.agent-room.test/agents.txt";
const KNOCKER: &str = "01990d9e-8400-7000-8000-0000000000b1";
/// 账号在的整间房“Project”，下面已经有一间子房间“Ship log”。
const PROJECT: &str = "01990d9e-8400-7000-8000-0000000000e1";
const SHIP_LOG: &str = "01990d9e-8400-7000-8000-0000000000e2";

/// 建房、开会话、看口令都记下来；口令总是 [`JOIN_CODE`]。
#[derive(Default)]
struct RoomsBridge {
    created: Mutex<Vec<IpcCreateRoomRequest>>,
    opened: Mutex<Vec<IpcOpenHostSessionRequest>>,
    access: Mutex<Vec<IpcRoomAccessRequest>>,
    entered: Mutex<Vec<IpcHostRoomTarget>>,
    refuse_create: Mutex<Option<BridgeToolFailure>>,
}

impl RoomsBridge {
    fn last_created(&self) -> IpcCreateRoomRequest {
        self.created
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("建过房")
    }

    /// 建过房就在最后建的那间；没建过、按名单进的就在会话开的那间。
    fn current_room(&self) -> Option<(String, String)> {
        if let Some(created) = self.created.lock().unwrap().last() {
            return Some((created.catalog_id.clone(), CREATED_ROOM.into()));
        }
        let opened = self.opened.lock().unwrap();
        let room = opened.last()?.room.as_ref()?;
        Some((room.catalog_id.clone(), room.room_id.clone()?))
    }

    fn in_session(&self, method: IpcMethod) -> Result<IpcResponse, BridgeToolFailure> {
        match method {
            IpcMethod::GetSelf => Ok(IpcResponse::SelfSummary {
                summary: IpcSelfSummary {
                    owner: None,
                    room_catalog_id: self.current_room().map(|(catalog_id, _)| catalog_id),
                    agent: IpcAgentSummary {
                        agent_id: uuid::Uuid::now_v7().to_string(),
                        display_name: "Scout".into(),
                        matrix_user_id: "@scout:test.invalid".into(),
                        avatar_url: None,
                    },
                    instance_id: uuid::Uuid::now_v7().to_string(),
                    matrix_device_id: "TEST".into(),
                    room_id: self
                        .current_room()
                        .map_or_else(|| CREATED_ROOM.into(), |(_, room_id)| room_id),
                    connection_state: IpcBridgeState::Ready,
                    granted_capabilities: vec![],
                },
            }),
            IpcMethod::EnterRoom(request) => {
                let catalog_id = request.room.catalog_id.clone();
                self.entered.lock().unwrap().push(request.room);
                Ok(IpcResponse::RoomEntered {
                    room: IpcSessionRoom {
                        catalog_id,
                        room_id: CREATED_ROOM.into(),
                    },
                })
            }
            IpcMethod::RoomAccess(request) => {
                self.access.lock().unwrap().push(request);
                Ok(IpcResponse::RoomAccess {
                    access: IpcRoomAccess {
                        catalog_id: self.last_created().catalog_id,
                        code: IpcRoomCode::Visible {
                            code: JOIN_CODE.into(),
                        },
                        knocks: vec![],
                        guide_url: GUIDE.into(),
                    },
                })
            }
            other => Err(unexpected(&other)),
        }
    }
}

/// 名单：整间的“Project”和它下面的“Ship log”。
fn project_rooms() -> Vec<IpcRoomSummary> {
    vec![
        IpcRoomSummary {
            catalog_id: PROJECT.into(),
            matrix_room_id: Some("!project:test.invalid".into()),
            ..room(IpcRoomKind::PrivateRoom, "Project", None)
        },
        IpcRoomSummary {
            catalog_id: SHIP_LOG.into(),
            matrix_room_id: Some("!ship-log:test.invalid".into()),
            parent: Some(IpcParentRoom {
                catalog_id: PROJECT.into(),
                name: "Project".into(),
            }),
            category: Some("Ship".into()),
            access: Some(IpcPublicRoomJoinMode::Open),
            ..room(IpcRoomKind::PrivateRoom, "Ship log", None)
        },
    ]
}

fn unexpected(method: &IpcMethod) -> BridgeToolFailure {
    BridgeToolFailure::new(
        "test.unexpected_call",
        IpcErrorCategory::Internal,
        false,
        BTreeMap::from([("method".to_owned(), format!("{method:?}"))]),
    )
}

impl BridgeToolClient for RoomsBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let response = match method {
            IpcMethod::CreateRoom(request) => {
                let refused = self.refuse_create.lock().unwrap().clone();
                refused.map_or_else(
                    || {
                        // 分类只认得“Ship”，别的都放进“未分类”。
                        let known = request
                            .category
                            .clone()
                            .filter(|category| category == "Ship");
                        let room = IpcRoomSummary {
                            kind: IpcRoomKind::PrivateRoom,
                            catalog_id: request.catalog_id.clone(),
                            matrix_room_id: Some(CREATED_ROOM.into()),
                            name: request.name.clone(),
                            slug: None,
                            membership: None,
                            parent: request.parent_catalog_id.clone().map(|catalog_id| {
                                IpcParentRoom {
                                    catalog_id,
                                    name: "Project".into(),
                                }
                            }),
                            category: known.clone(),
                            access: request
                                .parent_catalog_id
                                .as_ref()
                                .map(|_| request.access.unwrap_or(IpcPublicRoomJoinMode::Open)),
                        };
                        let category_missing = request.category.is_some() && known.is_none();
                        self.created.lock().unwrap().push(request);
                        Ok(IpcResponse::CreatedRoom {
                            room,
                            category_missing,
                        })
                    },
                    Err,
                )
            }
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: project_rooms(),
            }),
            IpcMethod::OpenHostSession(request) => {
                self.opened.lock().unwrap().push(request);
                Ok(IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: SESSION_A.into(),
                        state: IpcHostSessionState::Ready,
                        agent_id: None,
                        error_code: None,
                    },
                })
            }
            IpcMethod::WithSession { method, .. } => self.in_session(*method),
            other => Err(unexpected(&other)),
        };
        Box::pin(async move { response })
    }
}

async fn create(harness: &mut McpHarness, arguments: Value, task: &str) -> Value {
    harness
        .send(
            json!({"jsonrpc":"2.0","id":900,"method":"tools/call","params":{
                "name":"agent_room_create_room","arguments":arguments,"_meta":{"threadId":task}
            }}),
        )
        .await;
    harness.receive().await["result"].clone()
}

#[tokio::test]
async fn 建房后就在里面_拿到口令和要转给别的_agent_的话() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;

    let result = create(
        &mut harness,
        json!({"name": " 周报 ", "displayName": "Scout"}),
        TASK,
    )
    .await;

    assert_ne!(result["isError"], true, "{result}");
    let created = bridge.last_created();
    assert_eq!(created.name, "周报");
    assert_eq!(created.display_name, "Scout");
    assert!(uuid::Uuid::parse_str(&created.catalog_id).is_ok());
    // 建房的人物就是随后开会话的人物，开会话直接进建好的房间。
    let opened = bridge
        .opened
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("开了会话");
    assert_eq!(opened.session_key, created.session_key);
    let target = opened.room.expect("进建好的房间");
    assert_eq!(target.catalog_id, created.catalog_id);
    assert_eq!(target.room_id.as_deref(), Some(CREATED_ROOM));
    assert_eq!(
        *bridge.access.lock().unwrap(),
        [IpcRoomAccessRequest {
            action: IpcRoomAccessAction::Show,
            catalog_id: Some(created.catalog_id.clone()),
        }]
    );
    assert!(
        bridge.entered.lock().unwrap().is_empty(),
        "会话就开在建好的房间"
    );

    let content = &result["structuredContent"];
    assert_eq!(content["sessionId"], SESSION_A);
    assert_eq!(content["identity"], "created");
    assert_eq!(content["room"]["name"], "周报");
    assert_eq!(content["access"]["code"]["state"], "visible");
    assert_eq!(content["access"]["catalogId"], created.catalog_id.as_str());
    for language in ["zh", "en"] {
        let share = content["access"]["share"][language]
            .as_str()
            .expect("中英文各一段");
        for part in [JOIN_CODE, created.catalog_id.as_str(), GUIDE, "周报"] {
            assert!(share.contains(part), "{language}: {share}");
        }
    }
    assert!(content["localAgents"].as_str().unwrap().contains("周报"));
    assert!(content["people"].as_str().unwrap().contains("account ID"));
    harness.stop().await;
}

#[tokio::test]
async fn 同一任务同名再建回到同一间_人物也不换_换个名字或换个任务才是新的() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;
    create(
        &mut harness,
        json!({"name": "周报", "displayName": "Scout"}),
        TASK,
    )
    .await;
    let first = bridge.last_created();

    // 结果没回来时重试，或者之后又说一次：同一间、同一个人物，给了别的名字也照旧。
    let again = create(
        &mut harness,
        json!({"name": "周报", "displayName": "Other"}),
        TASK,
    )
    .await;
    assert_eq!(again["structuredContent"]["identity"], "reused");
    let second = bridge.last_created();
    assert_eq!(second.catalog_id, first.catalog_id);
    assert_eq!(second.session_key, first.session_key);
    assert_eq!(second.display_name, "Scout");

    // 同一任务再建一间：还是这个 Agent，会话照旧开在第一间，开好以后进新的这间。
    let meeting = create(&mut harness, json!({"name": "周会"}), TASK).await;
    assert_ne!(meeting["isError"], true, "{meeting}");
    assert_eq!(meeting["structuredContent"]["identity"], "reused");
    let third = bridge.last_created();
    assert_ne!(third.catalog_id, first.catalog_id);
    assert_eq!(third.session_key, first.session_key);
    let opened = bridge.opened.lock().unwrap().last().cloned().unwrap();
    assert_eq!(opened.room.unwrap().catalog_id, first.catalog_id);
    assert_eq!(
        bridge.entered.lock().unwrap().last().unwrap().catalog_id,
        third.catalog_id
    );
    create(&mut harness, json!({"name": "周报"}), OTHER_TASK).await;
    assert_ne!(bridge.last_created().catalog_id, first.catalog_id);
    harness.stop().await;
}

#[tokio::test]
async fn 建房的名字不合规不去问_bridge_旧版_bridge_提示更新桌面端() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;
    for name in ["   ", "周\n报"] {
        let invalid = create(&mut harness, json!({ "name": name }), TASK).await;
        assert_eq!(invalid["isError"], true);
        assert_eq!(
            invalid["structuredContent"]["code"],
            "bridge.ipc.room_name_invalid"
        );
    }
    assert!(bridge.created.lock().unwrap().is_empty());

    *bridge.refuse_create.lock().unwrap() = Some(failure("bridge.ipc.protocol_failed", true));
    let outdated = create(&mut harness, json!({"name": "周报"}), TASK).await;
    assert_eq!(
        outdated["structuredContent"]["code"],
        "agent.rooms.bridge_outdated"
    );
    assert_eq!(outdated["structuredContent"]["retryable"], false);

    *bridge.refuse_create.lock().unwrap() = Some(failure("bridge.rooms.rate_limited", false));
    let limited = create(&mut harness, json!({"name": "周报"}), TASK).await;
    assert_eq!(
        limited["structuredContent"]["code"],
        "bridge.rooms.rate_limited"
    );
    assert!(bridge.opened.lock().unwrap().is_empty(), "没建成就不开会话");
    harness.stop().await;
}

fn access_call(
    action: IpcRoomAccessAction,
    code: IpcRoomCode,
    knocks: Vec<IpcRoomKnock>,
) -> [ExpectedCall; 2] {
    let catalog_id = "01990d9e-8400-7000-8000-0000000000c1";
    [
        ExpectedCall {
            method: IpcMethod::WithSession {
                session_id: SESSION_A.into(),
                method: Box::new(IpcMethod::RoomAccess(IpcRoomAccessRequest {
                    action,
                    catalog_id: None,
                })),
            },
            response: Ok(IpcResponse::RoomAccess {
                access: IpcRoomAccess {
                    catalog_id: catalog_id.into(),
                    code,
                    knocks,
                    guide_url: GUIDE.into(),
                },
            }),
        },
        ExpectedCall {
            method: IpcMethod::ListRooms,
            response: Ok(IpcResponse::Rooms {
                rooms: vec![IpcRoomSummary {
                    catalog_id: catalog_id.into(),
                    ..room(IpcRoomKind::PrivateRoom, "周报", None)
                }],
            }),
        },
    ]
}

#[tokio::test]
async fn 看口令带上要转发的话_有人敲门先说_口令看不到时只给敲门的说法() {
    let knock = IpcRoomKnock {
        agent_id: KNOCKER.into(),
        display_name: "Helper".into(),
        knocked_at_unix_ms: 1_000,
        expires_at_unix_ms: 3_601_000,
    };
    let visible = IpcRoomCode::Visible {
        code: JOIN_CODE.into(),
    };
    let mut calls = Vec::new();
    calls.extend(access_call(IpcRoomAccessAction::Show, visible, vec![knock]));
    calls.extend(access_call(
        IpcRoomAccessAction::Replace,
        IpcRoomCode::Hidden,
        vec![],
    ));
    let bridge = Arc::new(ScriptedBridge::new(calls));
    let mut harness = McpHarness::start(bridge.clone()).await;

    let shown = harness
        .call("agent_room_room_access", json!({ "sessionId": SESSION_A }))
        .await;
    assert_ne!(shown["isError"], true, "{shown}");
    let content = &shown["structuredContent"];
    assert_eq!(content["knocks"][0]["displayName"], "Helper");
    assert!(
        content["next"]
            .as_str()
            .unwrap()
            .contains("agent_room_answer_knock")
    );
    let share = content["share"]["zh"].as_str().unwrap();
    assert!(
        share.contains(JOIN_CODE) && share.contains("「周报」"),
        "{share}"
    );

    let hidden = harness
        .call(
            "agent_room_room_access",
            json!({ "sessionId": SESSION_A, "action": "replace" }),
        )
        .await;
    let content = &hidden["structuredContent"];
    assert_eq!(content["code"]["state"], "hidden");
    assert!(!content["share"]["en"].as_str().unwrap().contains(JOIN_CODE));
    assert!(content["share"]["en"].as_str().unwrap().contains("knock"));
    assert!(content["next"].as_str().unwrap().contains("replace"));
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 放行_邀请和移出都在会话里_账户_id_统一成小写() {
    let account = "01990D9E-8400-7000-8000-0000000000D1";
    let catalog_id = "01990d9e-8400-7000-8000-0000000000c1";
    let in_session = |method| ExpectedCall {
        method: IpcMethod::WithSession {
            session_id: SESSION_A.into(),
            method: Box::new(method),
        },
        response: Ok(IpcResponse::RoomUpdated {
            catalog_id: catalog_id.into(),
        }),
    };
    let bridge = Arc::new(ScriptedBridge::new(vec![
        in_session(IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
            agent_id: KNOCKER.into(),
            admit: true,
            catalog_id: None,
        })),
        in_session(IpcMethod::InvitePerson(IpcInvitePersonRequest {
            account_id: account.to_ascii_lowercase(),
            catalog_id: None,
        })),
        in_session(IpcMethod::RemoveAgent(IpcRemoveAgentRequest {
            agent_id: KNOCKER.into(),
            catalog_id: None,
        })),
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    for (tool, arguments) in [
        (
            "agent_room_answer_knock",
            json!({ "sessionId": SESSION_A, "agentId": KNOCKER, "admit": true }),
        ),
        (
            "agent_room_invite_person",
            json!({ "sessionId": SESSION_A, "accountId": account }),
        ),
        (
            "agent_room_remove_agent",
            json!({ "sessionId": SESSION_A, "agentId": KNOCKER }),
        ),
    ] {
        let result = harness.call(tool, arguments).await;
        assert_ne!(result["isError"], true, "{tool}: {result}");
        assert_eq!(result["structuredContent"]["catalogId"], catalog_id);
    }
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 建子房间先找到父房间_带上门和分类_回答说在哪间房下面_不去拿口令() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;

    let result = create(
        &mut harness,
        json!({"name": "Release", "parent": "Project", "access": "request", "category": "Design"}),
        TASK,
    )
    .await;

    assert_ne!(result["isError"], true, "{result}");
    let created = bridge.last_created();
    assert_eq!(created.parent_catalog_id.as_deref(), Some(PROJECT));
    assert_eq!(created.access, Some(IpcPublicRoomJoinMode::Request));
    assert_eq!(created.category.as_deref(), Some("Design"));
    assert!(bridge.access.lock().unwrap().is_empty(), "子房间没有口令");
    let content = &result["structuredContent"];
    assert_eq!(content["room"]["parent"]["name"], "Project");
    assert_eq!(content["room"]["access"], "request");
    assert_eq!(content["categoryMissing"], true);
    assert!(
        content["categoryNote"]
            .as_str()
            .unwrap()
            .contains("Uncategorized")
    );
    assert!(content.get("access").is_none(), "{content}");
    assert!(
        content["localAgents"]
            .as_str()
            .unwrap()
            .contains("\"Project/Release\""),
        "{content}"
    );
    assert!(content["people"].as_str().unwrap().contains("sub-room"));

    // 同一个任务在同一间房下面再说一次是同一间；没带父房间的同名房间是另一间。
    create(
        &mut harness,
        json!({"name": "Release", "parent": PROJECT}),
        TASK,
    )
    .await;
    assert_eq!(bridge.last_created().catalog_id, created.catalog_id);
    create(&mut harness, json!({"name": "Release"}), TASK).await;
    assert_ne!(bridge.last_created().catalog_id, created.catalog_id);
    harness.stop().await;
}

#[tokio::test]
async fn 子房间的选项不合规_或者父房间找不到_都不去建() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;
    for (arguments, code) in [
        (
            json!({"name": "Release", "access": "open"}),
            "bridge.ipc.sub_room_options_invalid",
        ),
        (
            json!({"name": "Release", "category": "Ship"}),
            "bridge.ipc.sub_room_options_invalid",
        ),
        (
            json!({"name": "Re/lease", "parent": "Project"}),
            "bridge.ipc.room_name_invalid",
        ),
        (
            json!({"name": "Release", "parent": "Project", "category": " Ship"}),
            "bridge.ipc.sub_room_category_invalid",
        ),
        (
            json!({"name": "Release", "parent": "Nowhere"}),
            "agent.join.room_not_found",
        ),
    ] {
        let refused = create(&mut harness, arguments.clone(), TASK).await;
        assert_eq!(refused["isError"], true, "{arguments}");
        assert_eq!(refused["structuredContent"]["code"], code, "{arguments}");
    }
    assert!(bridge.created.lock().unwrap().is_empty());

    *bridge.refuse_create.lock().unwrap() = Some(failure(
        "bridge.rooms.sub_room_agents_may_not_create",
        false,
    ));
    let refused = create(
        &mut harness,
        json!({"name": "Release", "parent": "Project"}),
        TASK,
    )
    .await;
    assert_eq!(
        refused["structuredContent"]["code"],
        "bridge.rooms.sub_room_agents_may_not_create"
    );
    assert!(
        refused.to_string().contains("doesn't let agents"),
        "{refused}"
    );
    harness.stop().await;
}

#[tokio::test]
async fn 列房间默认不列子房间_带上父房间才列_和公开房间一起要不行() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;

    let all = harness.call("agent_room_list_rooms", json!({})).await;
    let names: Vec<_> = all["structuredContent"]["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|room| room["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Project"], "{all}");

    for parent in ["Project", PROJECT] {
        let listed = harness
            .call("agent_room_list_rooms", json!({ "parent": parent }))
            .await;
        let rooms = listed["structuredContent"]["rooms"].as_array().unwrap();
        assert_eq!(rooms.len(), 1, "{listed}");
        assert_eq!(rooms[0]["name"], "Ship log");
        assert_eq!(rooms[0]["category"], "Ship");
        assert_eq!(rooms[0]["access"], "open");
    }

    let mixed = harness
        .call(
            "agent_room_list_rooms",
            json!({ "parent": "Project", "publicRooms": true }),
        )
        .await;
    assert_eq!(mixed["isError"], true);
    assert_eq!(
        mixed["structuredContent"]["code"],
        "agent.rooms.parent_with_public"
    );
    let missing = harness
        .call("agent_room_list_rooms", json!({ "parent": "Nowhere" }))
        .await;
    assert_eq!(
        missing["structuredContent"]["code"],
        "agent.join.room_not_found"
    );
    harness.stop().await;
}

#[tokio::test]
async fn 进子房间写父房间名斜杠子房间名_只写子房间名找不到() {
    let bridge = Arc::new(RoomsBridge::default());
    let mut harness = McpHarness::start(bridge.clone()).await;

    let alone = harness
        .call("agent_room_join", json!({ "room": "Ship log" }))
        .await;
    assert_eq!(
        alone["structuredContent"]["code"], "agent.join.room_not_found",
        "{alone}"
    );
    assert!(bridge.opened.lock().unwrap().is_empty());

    let joined = harness
        .call("agent_room_join", json!({ "room": "Project / Ship log" }))
        .await;
    assert_ne!(joined["isError"], true, "{joined}");
    let opened = bridge.opened.lock().unwrap().last().cloned().unwrap();
    assert_eq!(opened.room.unwrap().catalog_id, SHIP_LOG);
    harness.stop().await;
}
