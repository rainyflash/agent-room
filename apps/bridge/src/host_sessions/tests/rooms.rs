//! Agent 建房间、管会话所在的房间（`specs/agent-rooms/design.md` 第 3 步）。

use agent_room_bridge_core::room_management::{
    ControlPlaneRoomManagementGateway, CreatedRoom, RoomCodeState, RoomJoinCode, RoomKnock,
    RoomManagementFailure, RoomManagementFailureKind, RoomManagementResult,
};
use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCreateRoomRequest, IpcHostRoomTarget, IpcInvitePersonRequest,
    IpcRemoveAgentRequest, IpcRoomAccess, IpcRoomAccessAction, IpcRoomAccessRequest, IpcRoomCode,
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId},
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
        room_directory: Arc::new(DirectoryFake { rooms: vec![] }),
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
    let method = IpcMethod::RoomAccess(IpcRoomAccessRequest { action });
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
        }),
        IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
            agent_id: agent.to_string(),
            admit: false,
        }),
        IpcMethod::InvitePerson(IpcInvitePersonRequest {
            account_id: account.to_string(),
        }),
        IpcMethod::RemoveAgent(IpcRemoveAgentRequest {
            agent_id: agent.to_string(),
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
