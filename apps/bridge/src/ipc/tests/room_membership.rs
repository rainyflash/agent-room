//! 会话的 Agent 再进一个房间、离开一个房间（`specs/agent-identity/design.md` 第 3 步）。

use std::sync::{Arc, Mutex};

use agent_room_application::ports::{MatrixRoomId, MatrixRoomSyncKind, PortFuture};
use agent_room_bridge_core::lobby_session::{
    AgentLobbyEntryIntent, AgentLobbyExit, AgentLobbySessionConfig, AgentLobbySessionService,
    ControlPlaneLobbyEntryGateway, ControlPlaneLobbyEntryOutcome, ControlPlaneLobbyEntryResult,
    JoinedAgentLobby, LobbyAssignmentKind,
};
use agent_room_bridge_ipc::{
    IpcEnterRoomRequest, IpcHostRoomTarget, IpcLeaveRoomRequest, IpcMethod, IpcResponse,
    IpcSessionRoom,
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomLanguage},
    time::UtcMillis,
};
use uuid::Uuid;

use super::super::BridgeIpcRequestHandler as _;
use super::viewing::{ROOM, store_with};
use super::{
    BridgeAgentRuntimeSnapshot, FoundationBridgeIpcRequestHandler, 固定Agent运行时, 固定时钟,
    固定状态, 测试_agent_身份, 空正文服务,
};
use crate::{ipc::AgentRoomEntry, joined_rooms::JoinedRooms};

const PRIVATE_ROOM: &str = "!private:matrix.test";

/// 控制面：进房间按顺序给几个结果，离开一律回答离开了；都记下请求。
struct 进出控制面 {
    outcomes: Mutex<Vec<ControlPlaneLobbyEntryOutcome>>,
    intents: Mutex<Vec<AgentLobbyEntryIntent>>,
    exits: Mutex<Vec<AgentLobbyExit>>,
}

impl ControlPlaneLobbyEntryGateway for 进出控制面 {
    fn enter<'a>(
        &'a self,
        intent: &'a AgentLobbyEntryIntent,
    ) -> PortFuture<'a, ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>> {
        self.intents.lock().expect("锁可用").push(intent.clone());
        let outcome = self.outcomes.lock().expect("锁可用").remove(0);
        Box::pin(async move { Ok(outcome) })
    }

    fn leave<'a>(
        &'a self,
        exit: &'a AgentLobbyExit,
    ) -> PortFuture<'a, ControlPlaneLobbyEntryResult<bool>> {
        self.exits.lock().expect("锁可用").push(*exit);
        Box::pin(async { Ok(true) })
    }
}

fn catalog(index: u8) -> RoomCatalogId {
    RoomCatalogId::from_uuid(
        Uuid::parse_str(&format!("0198b601-77a1-7bb8-83eb-a8fe68c97e{index:02x}"))
            .expect("目录标识有效"),
    )
}

/// 只用来看 Agent 在哪些房间的入口：控制面不会被问到。
pub(super) fn entry_with(joined: Arc<JoinedRooms>) -> AgentRoomEntry {
    AgentRoomEntry::new(
        Arc::new(AgentLobbySessionService::new(Arc::new(进出控制面 {
            outcomes: Mutex::new(Vec::new()),
            intents: Mutex::new(Vec::new()),
            exits: Mutex::new(Vec::new()),
        }))),
        AgentLobbySessionConfig::new(catalog(1), None, None),
        catalog(1),
        joined,
    )
}

fn joined_private() -> ControlPlaneLobbyEntryOutcome {
    ControlPlaneLobbyEntryOutcome::Joined(JoinedAgentLobby::new(
        catalog(2),
        RoomInstanceId::from_uuid(Uuid::now_v7()),
        MatrixRoomReference::new(PRIVATE_ROOM).expect("房间有效"),
        RoomReservationId::from_uuid(Uuid::now_v7()),
        LobbyAssignmentKind::New,
    ))
}

#[tokio::test]
async fn 再进一个房间照开会话时的偏好_开会话的那个房间离不开() {
    let control_plane = Arc::new(进出控制面 {
        outcomes: Mutex::new(vec![
            ControlPlaneLobbyEntryOutcome::ProvisioningBusy {
                retry_at: UtcMillis::new(1).expect("时间有效"),
            },
            joined_private(),
        ]),
        intents: Mutex::new(Vec::new()),
        exits: Mutex::new(Vec::new()),
    });
    // 本机已经同步到这个房间：不用等。
    let joined = Arc::new(JoinedRooms::new());
    let private = MatrixRoomId::new(PRIVATE_ROOM).expect("房间有效");
    joined.record([(&private, MatrixRoomSyncKind::Joined)]);
    let entry = AgentRoomEntry::new(
        Arc::new(AgentLobbySessionService::new(control_plane.clone())),
        AgentLobbySessionConfig::new(
            catalog(1),
            Some(RoomLanguage::new("zh-CN".to_owned()).expect("语言有效")),
            None,
        ),
        catalog(1),
        joined,
    );
    let (_directory, store) = store_with(&[]).await;
    let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
        super::super::AgentRuntimeConsumer::HostSession,
        Arc::new(固定状态),
        Arc::new(固定Agent运行时(
            BridgeAgentRuntimeSnapshot::new(
                测试_agent_身份(),
                "DEVICE-1",
                MatrixRoomId::new(ROOM).expect("房间标识有效"),
                ["previews.read"],
            )
            .with_room_entry(Arc::new(entry)),
        )),
        store.clone(),
        空正文服务(store),
        Arc::new(固定时钟),
    );
    let enter = IpcMethod::EnterRoom(IpcEnterRoomRequest {
        room: IpcHostRoomTarget {
            catalog_id: catalog(2).to_string(),
            room_id: Some(PRIVATE_ROOM.to_owned()),
        },
    });

    let busy = handler
        .dispatch(enter.clone())
        .await
        .expect_err("公共大厅正在开新分片");
    assert_eq!(busy.code(), "bridge.lobby_provisioning_busy");
    assert!(busy.retryable());
    let entered = handler.dispatch(enter).await.expect("进了私人房间");
    assert_eq!(
        entered,
        IpcResponse::RoomEntered {
            room: IpcSessionRoom {
                catalog_id: catalog(2).to_string(),
                room_id: PRIVATE_ROOM.to_owned(),
            },
        }
    );
    let intent = control_plane.intents.lock().expect("锁可用")[1].clone();
    assert_eq!(intent.catalog_id(), catalog(2));
    assert_eq!(
        intent.target_room().map(MatrixRoomReference::as_str),
        Some(PRIVATE_ROOM)
    );
    assert_eq!(
        intent.preferred_language().map(RoomLanguage::as_str),
        Some("zh-CN"),
        "照开会话时的偏好"
    );

    let leave = |catalog_id: RoomCatalogId| {
        IpcMethod::LeaveRoom(IpcLeaveRoomRequest {
            catalog_id: catalog_id.to_string(),
        })
    };
    let refused = handler
        .dispatch(leave(catalog(1)))
        .await
        .expect_err("开会话的那个房间离不开");
    assert_eq!(refused.code(), "bridge.leave_session_room");
    assert!(control_plane.exits.lock().expect("锁可用").is_empty());
    assert_eq!(
        handler.dispatch(leave(catalog(2))).await.expect("离开"),
        IpcResponse::RoomLeft {
            catalog_id: catalog(2).to_string(),
            left: true,
        }
    );
    assert_eq!(
        control_plane.exits.lock().expect("锁可用")[0].catalog_id(),
        catalog(2)
    );
}
