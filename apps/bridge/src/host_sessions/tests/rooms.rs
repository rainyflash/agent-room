//! Agent 建房间、管会话所在的房间（`specs/agent-rooms/design.md` 第 3 步）。

use agent_room_bridge_core::room_management::{
    ControlPlaneRoomManagementGateway, CreatedRoom, RoomCodeState, RoomJoinCode, RoomKnock,
    RoomManagementFailure, RoomManagementFailureKind, RoomManagementResult, RoomModerationGrant,
    RoomModerationOutcome, RoomModerationRequest,
};
use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCreateRoomRequest, IpcHostRoomTarget, IpcInvitePersonRequest,
    IpcModerateRoomRequest, IpcModerationAction, IpcModerationGrant, IpcRemoveAgentRequest,
    IpcRoomAccess, IpcRoomAccessAction, IpcRoomAccessRequest, IpcRoomCode,
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

use super::*;

const GUIDE_URL: &str = "https://api.agent-room.test/agents.txt";

/// 控制面管房间的接口：记下每次调用；口令按设好的给，生成、停用时跟着变。
#[derive(Default)]
pub(super) struct RoomsFake {
    code: std::sync::Mutex<Option<RoomCodeState>>,
    knocks: Vec<RoomKnock>,
    calls: std::sync::Mutex<Vec<String>>,
    failure: std::sync::Mutex<Option<RoomManagementFailureKind>>,
    /// 主人给的管理授权；没有就是这个房间没交给 Agent 管。
    grant: Option<RoomModerationGrant>,
}

impl RoomsFake {
    fn record(&self, call: String) -> RoomManagementResult<()> {
        self.calls.lock().unwrap().push(call);
        self.failure
            .lock()
            .unwrap()
            .map_or(Ok(()), |kind| Err(RoomManagementFailure::new(kind)))
    }

    fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

impl ControlPlaneRoomManagementGateway for RoomsFake {
    fn create<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        name: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, RoomManagementResult<CreatedRoom>> {
        let created = self
            .record(format!("create {catalog_id} {name} {agent_id}"))
            .map(|()| CreatedRoom {
                catalog_id,
                matrix_room_id: MatrixRoomReference::new("!created:matrix.test".to_owned())
                    .expect("Matrix 房间标识有效"),
                name: name.to_owned(),
            });
        Box::pin(async move { created })
    }

    fn code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomCodeState>> {
        let state = self.record(format!("code {catalog_id}")).map(|()| {
            self.code
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(RoomCodeState::Missing)
        });
        Box::pin(async move { state })
    }

    fn generate_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomJoinCode>> {
        let generated = self.record(format!("generate {catalog_id}")).map(|()| {
            let count = self.calls.lock().unwrap().len();
            let code = RoomJoinCode {
                code: format!("K7P3-Q9XW-{count:04}"),
                created_at: UtcMillis::new(1).expect("时间有效"),
            };
            *self.code.lock().unwrap() = Some(RoomCodeState::Visible(code.clone()));
            code
        });
        Box::pin(async move { generated })
    }

    fn disable_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RoomManagementResult<()>> {
        let disabled = self
            .record(format!("disable {catalog_id}"))
            .map(|()| *self.code.lock().unwrap() = Some(RoomCodeState::Missing));
        Box::pin(async move { disabled })
    }

    fn knocks(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<Vec<RoomKnock>>> {
        let knocks = self
            .record(format!("knocks {catalog_id}"))
            .map(|()| self.knocks.clone());
        Box::pin(async move { knocks })
    }

    fn answer_knock(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        admit: bool,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        let answered = self.record(format!("answer {catalog_id} {agent_id} {admit}"));
        Box::pin(async move { answered })
    }

    fn invite(
        &self,
        catalog_id: RoomCatalogId,
        account_id: PrincipalId,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        let invited = self.record(format!("invite {catalog_id} {account_id}"));
        Box::pin(async move { invited })
    }

    fn remove_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        let removed = self.record(format!("remove {catalog_id} {agent_id}"));
        Box::pin(async move { removed })
    }

    fn moderation_grant(
        &self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<Option<RoomModerationGrant>>> {
        let grant = self
            .record(format!("rules {agent_id} {instance_id} {catalog_id}"))
            .map(|()| self.grant.clone());
        Box::pin(async move { grant })
    }

    fn moderate<'a>(
        &'a self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
        _action_id: Uuid,
        request: &'a RoomModerationRequest,
    ) -> PortFuture<'a, RoomManagementResult<RoomModerationOutcome>> {
        let outcome = self
            .record(format!(
                "{} {agent_id} {instance_id} {catalog_id} {} {:?} {} {:?}",
                request.action,
                request.target,
                request.reason,
                request.note,
                request.duration_seconds
            ))
            .map(|()| RoomModerationOutcome {
                action_id: "0198b601-77a1-7bb8-83eb-a8fe68c97e60".to_owned(),
                action: request.action.to_owned(),
                status: "applied".to_owned(),
                expires_at: request
                    .duration_seconds
                    .map(|_| UtcMillis::new(600_000).expect("时间有效")),
            });
        Box::pin(async move { outcome })
    }
}

pub(super) fn access(
    gateway: Arc<RoomsFake>,
    host_agents: Arc<HostAgentsFake>,
) -> RoomManagementAccess {
    RoomManagementAccess {
        gateway,
        host_agents,
        guide_url: GUIDE_URL.to_owned(),
    }
}

fn handler(rooms: Arc<RoomsFake>, host_agents: Arc<HostAgentsFake>) -> SessionAwareIpcHandler {
    SessionAwareIpcHandler {
        default: Arc::new(RefusingHandler),
        sessions: Arc::new(HostSessionRegistry::new(Arc::new(TestFactory::default()))),
        connection_status: Arc::new(ReadyStatus),
        room_directory: Arc::new(DirectoryFake::default()),
        invitations: InvitationSlot::default(),
        join_codes: join_codes(Arc::new(JoinCodesFake::default()), host_agents.clone()),
        rooms: access(rooms, host_agents),
    }
}

/// 在 `catalog` 这个房间里开好会话，等它就绪。
async fn session_in(handler: &SessionAwareIpcHandler, catalog: Option<RoomCatalogId>) -> String {
    let mut opened = request("Scout");
    opened.room = catalog.map(|catalog| IpcHostRoomTarget {
        catalog_id: catalog.to_string(),
        room_id: Some("!created:matrix.test".to_owned()),
    });
    let id = open(&handler.sessions, opened).await;
    identity(&handler.sessions, &id).await;
    id
}

async fn in_session(
    handler: &SessionAwareIpcHandler,
    session_id: &str,
    method: IpcMethod,
) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
    handler
        .dispatch(IpcMethod::WithSession {
            session_id: session_id.to_owned(),
            method: Box::new(method),
        })
        .await
}

async fn room_access(
    handler: &SessionAwareIpcHandler,
    session_id: &str,
    action: IpcRoomAccessAction,
) -> IpcRoomAccess {
    let method = IpcMethod::RoomAccess(IpcRoomAccessRequest {
        action,
        catalog_id: None,
    });
    match in_session(handler, session_id, method).await {
        Ok(IpcResponse::RoomAccess { access }) => access,
        other => panic!("应返回房间怎么进：{other:?}"),
    }
}

#[tokio::test]
async fn 建房不需要会话_先按会话键建好人物_再拿它建房() {
    let rooms = Arc::new(RoomsFake::default());
    let host_agents = Arc::new(HostAgentsFake::default());
    let handler = handler(rooms.clone(), host_agents.clone());
    let session_key = Uuid::now_v7();
    let catalog = RoomCatalogId::from_uuid(Uuid::now_v7());

    let response = handler
        .dispatch(IpcMethod::CreateRoom(IpcCreateRoomRequest {
            catalog_id: catalog.to_string(),
            name: "周报".to_owned(),
            session_key: session_key.to_string(),
            display_name: "Scout".to_owned(),
        }))
        .await
        .expect("建房");

    let IpcResponse::CreatedRoom { room } = response else {
        panic!("应返回建好的房间");
    };
    assert_eq!(room.kind, IpcRoomKind::PrivateRoom);
    assert_eq!(room.catalog_id, catalog.to_string());
    assert_eq!(room.matrix_room_id.as_deref(), Some("!created:matrix.test"));
    assert_eq!(room.name, "周报");
    // 与随后开会话同一个会话键、同一个名字：建房的 Agent 就是随后进房间的人物。
    assert_eq!(
        host_agents.created.lock().unwrap().as_slice(),
        &[(
            AgentCreationRequestId::from_uuid(session_key),
            "Scout".to_owned()
        )]
    );
    assert_eq!(
        rooms.take_calls(),
        [format!(
            "create {catalog} 周报 {}",
            AgentId::from_uuid(session_key)
        )]
    );
}

#[tokio::test]
async fn 看口令_没有就生成_看不到的不替人换_更换和停用要明说() {
    let catalog = RoomCatalogId::from_uuid(Uuid::now_v7());
    let knock = RoomKnock {
        agent_id: AgentId::from_uuid(Uuid::now_v7()),
        display_name: "Helper".to_owned(),
        knocked_at: UtcMillis::new(1_000).expect("时间有效"),
        expires_at: UtcMillis::new(3_601_000).expect("时间有效"),
    };
    let rooms = Arc::new(RoomsFake {
        knocks: vec![knock.clone()],
        ..RoomsFake::default()
    });
    let handler = handler(rooms.clone(), Arc::default());
    let id = session_in(&handler, Some(catalog)).await;

    let first = room_access(&handler, &id, IpcRoomAccessAction::Show).await;
    assert_eq!(first.catalog_id, catalog.to_string());
    assert_eq!(first.guide_url, GUIDE_URL);
    assert_eq!(first.knocks.len(), 1);
    assert_eq!(first.knocks[0].agent_id, knock.agent_id.to_string());
    assert_eq!(first.knocks[0].display_name, "Helper");
    assert_eq!(first.knocks[0].expires_at_unix_ms, 3_601_000);
    let IpcRoomCode::Visible { code } = first.code.clone() else {
        panic!("没有口令时生成一个");
    };
    assert_eq!(
        rooms.take_calls(),
        [
            format!("code {catalog}"),
            format!("generate {catalog}"),
            format!("knocks {catalog}"),
        ]
    );

    // 再看一次还是同一个，不再生成。
    let again = room_access(&handler, &id, IpcRoomAccessAction::Show).await;
    assert_eq!(again.code, IpcRoomCode::Visible { code: code.clone() });
    assert_eq!(
        rooms.take_calls(),
        [format!("code {catalog}"), format!("knocks {catalog}")]
    );

    // 有口令、但看不到：照实说，不替人换。
    *rooms.code.lock().unwrap() = Some(RoomCodeState::Hidden);
    let hidden = room_access(&handler, &id, IpcRoomAccessAction::Show).await;
    assert_eq!(hidden.code, IpcRoomCode::Hidden);
    assert!(
        !rooms
            .take_calls()
            .iter()
            .any(|call| call.starts_with("generate"))
    );

    let replaced = room_access(&handler, &id, IpcRoomAccessAction::Replace).await;
    let IpcRoomCode::Visible { code: new_code } = replaced.code else {
        panic!("换口令给出新口令");
    };
    assert_ne!(new_code, code);
    assert_eq!(
        rooms.take_calls(),
        [format!("generate {catalog}"), format!("knocks {catalog}")]
    );

    let disabled = room_access(&handler, &id, IpcRoomAccessAction::Disable).await;
    assert_eq!(disabled.code, IpcRoomCode::Missing);
    assert_eq!(
        rooms.take_calls(),
        [format!("disable {catalog}"), format!("knocks {catalog}")]
    );
}

#[tokio::test]
async fn 放行_拒绝_邀请和移出管的都是会话所在的房间() {
    let catalog = RoomCatalogId::from_uuid(Uuid::now_v7());
    let rooms = Arc::new(RoomsFake::default());
    let handler = handler(rooms.clone(), Arc::default());
    let id = session_in(&handler, Some(catalog)).await;
    let agent = AgentId::from_uuid(Uuid::now_v7());
    let account = PrincipalId::from_uuid(Uuid::now_v7());

    let methods = [
        IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
            agent_id: agent.to_string(),
            admit: true,
            catalog_id: None,
        }),
        IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
            agent_id: agent.to_string(),
            admit: false,
            catalog_id: None,
        }),
        IpcMethod::InvitePerson(IpcInvitePersonRequest {
            account_id: account.to_string(),
            catalog_id: None,
        }),
        IpcMethod::RemoveAgent(IpcRemoveAgentRequest {
            agent_id: agent.to_string(),
            catalog_id: None,
        }),
    ];
    for method in methods {
        let response = in_session(&handler, &id, method).await.expect("控制面答应");
        assert_eq!(
            response,
            IpcResponse::RoomUpdated {
                catalog_id: catalog.to_string()
            }
        );
    }
    assert_eq!(
        rooms.take_calls(),
        [
            format!("answer {catalog} {agent} true"),
            format!("answer {catalog} {agent} false"),
            format!("invite {catalog} {account}"),
            format!("remove {catalog} {agent}"),
        ]
    );
}

#[tokio::test]
async fn 不在房间里的会话管不了_控制面的拒绝换成稳定的错误码() {
    let rooms = Arc::new(RoomsFake::default());
    let handler = handler(rooms.clone(), Arc::default());
    let show = || IpcMethod::RoomAccess(IpcRoomAccessRequest::default());

    let missing = in_session(&handler, &Uuid::now_v7().to_string(), show())
        .await
        .expect_err("没有这个会话");
    assert_eq!(missing.code(), "bridge.host_session.not_found");

    // 会话说不出自己在哪个目录房间：什么都不问控制面。
    let roomless = session_in(&handler, None).await;
    let failure = in_session(&handler, &roomless, show())
        .await
        .expect_err("没有目录房间");
    assert_eq!(failure.code(), "bridge.rooms.not_found");
    assert!(rooms.take_calls().is_empty());

    let id = session_in(&handler, Some(RoomCatalogId::from_uuid(Uuid::now_v7()))).await;
    for (kind, code, retryable) in [
        (
            RoomManagementFailureKind::Forbidden,
            "bridge.rooms.forbidden",
            false,
        ),
        (
            RoomManagementFailureKind::NotFound,
            "bridge.rooms.not_found",
            false,
        ),
        (
            RoomManagementFailureKind::RateLimited,
            "bridge.rooms.rate_limited",
            false,
        ),
        (
            RoomManagementFailureKind::Unsupported,
            "bridge.rooms.unsupported",
            false,
        ),
        (
            RoomManagementFailureKind::NotAuthorized,
            "bridge.host_session.device_authorization_required",
            false,
        ),
        (
            RoomManagementFailureKind::ControlPlaneUnavailable,
            "bridge.rooms.control_plane_unavailable",
            true,
        ),
        (
            RoomManagementFailureKind::InvalidControlPlaneResponse,
            "bridge.rooms.failed",
            false,
        ),
    ] {
        *rooms.failure.lock().unwrap() = Some(kind);
        let failure = in_session(&handler, &id, show())
            .await
            .expect_err("控制面拒绝");
        assert_eq!((failure.code(), failure.retryable()), (code, retryable));
    }
}

#[tokio::test]
async fn 说了房间就管那一间_要是会话的_agent_在的() {
    let home = RoomCatalogId::from_uuid(Uuid::now_v7());
    let entered = RoomCatalogId::from_uuid(Uuid::now_v7());
    let elsewhere = RoomCatalogId::from_uuid(Uuid::now_v7());
    let rooms = Arc::new(RoomsFake::default());
    let handler = handler(rooms.clone(), Arc::default());
    let id = session_in(&handler, Some(home)).await;
    in_session(
        &handler,
        &id,
        IpcMethod::EnterRoom(IpcEnterRoomRequest {
            room: IpcHostRoomTarget {
                catalog_id: entered.to_string(),
                room_id: None,
            },
        }),
    )
    .await
    .expect("又进了一个房间");
    let agent = AgentId::from_uuid(Uuid::now_v7());
    let remove = |catalog: RoomCatalogId| {
        IpcMethod::RemoveAgent(IpcRemoveAgentRequest {
            agent_id: agent.to_string(),
            catalog_id: Some(catalog.to_string()),
        })
    };

    for catalog in [entered, home] {
        assert_eq!(
            in_session(&handler, &id, remove(catalog))
                .await
                .expect("控制面答应"),
            IpcResponse::RoomUpdated {
                catalog_id: catalog.to_string()
            }
        );
    }
    let refused = in_session(&handler, &id, remove(elsewhere))
        .await
        .expect_err("它不在这个房间");
    assert_eq!(refused.code(), "bridge.room_not_joined");
    assert_eq!(
        rooms.take_calls(),
        [
            format!("remove {entered} {agent}"),
            format!("remove {home} {agent}"),
        ]
    );
}

fn moderate(action: IpcModerationAction) -> IpcModerateRoomRequest {
    IpcModerateRoomRequest {
        action,
        catalog_id: None,
        room_id: None,
        target: None,
        reason: None,
        note: None,
        duration_seconds: None,
    }
}

#[tokio::test]
async fn 管房间读规则_照授权禁言_用的是会话的_agent_和它所在的房间() {
    let rooms = Arc::new(RoomsFake {
        grant: Some(RoomModerationGrant {
            rules: "Mute anyone who posts ads.".to_owned(),
            actions: vec!["mute".to_owned()],
            expires_at: UtcMillis::new(9_000).expect("时间有效"),
        }),
        ..RoomsFake::default()
    });
    let handler = handler(rooms.clone(), Arc::default());
    let catalog = RoomCatalogId::from_uuid(Uuid::now_v7());
    let id = session_in(&handler, Some(catalog)).await;
    let me = identity(&handler.sessions, &id).await;
    let who = format!("{} {} {catalog}", me.agent.agent_id, me.instance_id);

    let rules = in_session(
        &handler,
        &id,
        IpcMethod::ModerateRoom(moderate(IpcModerationAction::Rules)),
    )
    .await
    .expect("读规则");
    let IpcResponse::RoomModerationRules { catalog_id, grant } = rules else {
        panic!("应返回授权：{rules:?}");
    };
    assert_eq!(catalog_id, catalog.to_string());
    assert_eq!(
        grant,
        Some(IpcModerationGrant {
            rules: "Mute anyone who posts ads.".to_owned(),
            actions: vec!["mute".to_owned()],
            expires_at_unix_ms: 9_000,
        })
    );

    let muted = in_session(
        &handler,
        &id,
        IpcMethod::ModerateRoom(IpcModerateRoomRequest {
            target: Some("@spammer:matrix.test".to_owned()),
            reason: Some("spam".to_owned()),
            note: Some("Posted the same link five times.".to_owned()),
            duration_seconds: Some(600),
            ..moderate(IpcModerationAction::Mute)
        }),
    )
    .await
    .expect("禁言");
    let IpcResponse::RoomModerated { outcome, .. } = muted else {
        panic!("应返回台账里的那一条：{muted:?}");
    };
    assert_eq!(outcome.status, "applied");
    assert_eq!(outcome.expires_at_unix_ms, Some(600_000));
    assert_eq!(
        rooms.take_calls(),
        [
            format!("rules {who}"),
            format!(
                "mute {who} @spammer:matrix.test Some(\"spam\") Posted the same link five times. Some(600)"
            ),
        ]
    );
}

#[tokio::test]
async fn 没有授权时回答里没有_控制面的拒绝换成说得清原因的错误码() {
    let rooms = Arc::new(RoomsFake::default());
    let handler = handler(rooms.clone(), Arc::default());
    let id = session_in(&handler, Some(RoomCatalogId::from_uuid(Uuid::now_v7()))).await;

    let rules = in_session(
        &handler,
        &id,
        IpcMethod::ModerateRoom(moderate(IpcModerationAction::Rules)),
    )
    .await
    .expect("读规则");
    assert!(matches!(
        rules,
        IpcResponse::RoomModerationRules { grant: None, .. }
    ));

    let unban = || {
        IpcMethod::ModerateRoom(IpcModerateRoomRequest {
            target: Some("@someone:matrix.test".to_owned()),
            note: Some("They said they will stop.".to_owned()),
            ..moderate(IpcModerationAction::Unban)
        })
    };
    for (kind, code) in [
        (
            RoomManagementFailureKind::NotGranted,
            "bridge.moderation.not_granted",
        ),
        (
            RoomManagementFailureKind::NotAllowed,
            "bridge.moderation.not_allowed",
        ),
        (
            RoomManagementFailureKind::Protected,
            "bridge.moderation.protected",
        ),
        (
            RoomManagementFailureKind::TargetNotFound,
            "bridge.moderation.target_not_found",
        ),
        (
            RoomManagementFailureKind::NotInRoom,
            "bridge.moderation.not_in_room",
        ),
        (
            RoomManagementFailureKind::NotBanned,
            "bridge.moderation.not_banned",
        ),
        (
            RoomManagementFailureKind::BannedByPerson,
            "bridge.moderation.banned_by_person",
        ),
    ] {
        *rooms.failure.lock().unwrap() = Some(kind);
        let failure = in_session(&handler, &id, unban())
            .await
            .expect_err("控制面拒绝");
        assert_eq!((failure.code(), failure.retryable()), (code, false));
    }

    // 说了不是会话的 Agent 在的房间：什么都不问控制面。
    rooms.take_calls();
    let elsewhere = in_session(
        &handler,
        &id,
        IpcMethod::ModerateRoom(IpcModerateRoomRequest {
            catalog_id: Some(Uuid::now_v7().to_string()),
            ..moderate(IpcModerationAction::Rules)
        }),
    )
    .await
    .expect_err("不在那个房间");
    assert_eq!(elsewhere.code(), "bridge.room_not_joined");
    assert!(rooms.take_calls().is_empty());
}

#[tokio::test]
async fn 用消息里的_room_id_说房间也行_要是会话的_agent_在的() {
    let home = RoomCatalogId::from_uuid(Uuid::now_v7());
    let entered = RoomCatalogId::from_uuid(Uuid::now_v7());
    let rooms = Arc::new(RoomsFake::default());
    let handler = handler(rooms.clone(), Arc::default());
    let id = session_in(&handler, Some(home)).await;
    let IpcResponse::RoomEntered { room } = in_session(
        &handler,
        &id,
        IpcMethod::EnterRoom(IpcEnterRoomRequest {
            room: IpcHostRoomTarget {
                catalog_id: entered.to_string(),
                room_id: None,
            },
        }),
    )
    .await
    .expect("又进了一个房间") else {
        panic!("应进了房间");
    };
    let rules_in = |room_id: &str| {
        IpcMethod::ModerateRoom(IpcModerateRoomRequest {
            room_id: Some(room_id.to_owned()),
            ..moderate(IpcModerationAction::Rules)
        })
    };

    // 会话的 Matrix 房间是开会话时进的那个；又进的那个按它进房间时拿到的 Matrix 房间 ID 认。
    for (room_id, catalog) in [
        ("!lobby:test.invalid", home),
        (room.room_id.as_str(), entered),
    ] {
        let answer = in_session(&handler, &id, rules_in(room_id))
            .await
            .expect("读规则");
        assert!(matches!(
            answer,
            IpcResponse::RoomModerationRules { catalog_id, grant: None } if catalog_id == catalog.to_string()
        ));
    }
    let elsewhere = in_session(&handler, &id, rules_in("!elsewhere:test.invalid"))
        .await
        .expect_err("不在那个房间");
    assert_eq!(elsewhere.code(), "bridge.room_not_joined");
    assert_eq!(rooms.take_calls().len(), 2);
}
