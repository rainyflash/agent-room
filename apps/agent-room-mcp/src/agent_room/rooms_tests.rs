//! 建房间、管房间的工具（`specs/agent-rooms/design.md` 第 3 步），走真实的 MCP 传输。

use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCreateRoomRequest, IpcInvitePersonRequest, IpcRemoveAgentRequest,
    IpcRoomAccess, IpcRoomAccessAction, IpcRoomAccessRequest, IpcRoomCode, IpcRoomKnock,
};

use super::*;

const TASK: &str = "01990d9e-8400-7000-8000-0000000000a1";
const OTHER_TASK: &str = "01990d9e-8400-7000-8000-0000000000a2";
const CREATED_ROOM: &str = "!created:test.invalid";
const GUIDE: &str = "https://api.agent-room.test/agents.txt";
const KNOCKER: &str = "01990d9e-8400-7000-8000-0000000000b1";

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

    fn in_session(&self, method: IpcMethod) -> Result<IpcResponse, BridgeToolFailure> {
        match method {
            IpcMethod::GetSelf => Ok(IpcResponse::SelfSummary {
                summary: IpcSelfSummary {
                    owner: None,
                    room_catalog_id: Some(self.last_created().catalog_id),
                    agent: IpcAgentSummary {
                        agent_id: uuid::Uuid::now_v7().to_string(),
                        display_name: "Scout".into(),
                        matrix_user_id: "@scout:test.invalid".into(),
                        avatar_url: None,
                    },
                    instance_id: uuid::Uuid::now_v7().to_string(),
                    matrix_device_id: "TEST".into(),
                    room_id: CREATED_ROOM.into(),
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
                        let room = IpcRoomSummary {
                            kind: IpcRoomKind::PrivateRoom,
                            catalog_id: request.catalog_id.clone(),
                            matrix_room_id: Some(CREATED_ROOM.into()),
                            name: request.name.clone(),
                            slug: None,
                            membership: None,
                        };
                        self.created.lock().unwrap().push(request);
                        Ok(IpcResponse::CreatedRoom { room })
                    },
                    Err,
                )
            }
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
