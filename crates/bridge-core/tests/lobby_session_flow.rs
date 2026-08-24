use std::sync::{Arc, Mutex};

use agent_room_application::ports::{MatrixUserId, PortFuture};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    lobby_session::{
        AgentLobbyEntryIntent, AgentLobbySessionConfig, AgentLobbySessionFailureKind,
        AgentLobbySessionService, ControlPlaneLobbyEntryFailure, ControlPlaneLobbyEntryFailureKind,
        ControlPlaneLobbyEntryGateway, ControlPlaneLobbyEntryOutcome, ControlPlaneLobbyEntryResult,
        JoinedAgentLobby, LobbyAssignmentKind,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomLanguage, RoomRegion},
    time::UtcMillis,
};
use uuid::Uuid;

const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const OTHER_CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const ROOM_INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const RESERVATION_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

struct 固定大厅控制面 {
    outcome: Mutex<Option<ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>>>,
    intent: Mutex<Option<AgentLobbyEntryIntent>>,
}

impl 固定大厅控制面 {
    fn returning(outcome: ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>) -> Self {
        Self {
            outcome: Mutex::new(Some(outcome)),
            intent: Mutex::new(None),
        }
    }
}

impl ControlPlaneLobbyEntryGateway for 固定大厅控制面 {
    fn enter<'a>(
        &'a self,
        intent: &'a AgentLobbyEntryIntent,
    ) -> PortFuture<'a, ControlPlaneLobbyEntryResult<ControlPlaneLobbyEntryOutcome>> {
        *self.intent.lock().expect("大厅意图锁可用") = Some(intent.clone());
        let outcome = self
            .outcome
            .lock()
            .expect("大厅结果锁可用")
            .take()
            .expect("测试已配置一个结果");
        Box::pin(async move { outcome })
    }
}

#[tokio::test]
async fn 使用登记身份进入大厅且保留语言地区偏好() {
    let gateway = Arc::new(固定大厅控制面::returning(Ok(joined(catalog_id()))));
    let service = AgentLobbySessionService::new(gateway.clone());
    let config = config();

    let outcome = service
        .enter(&identity(), &config)
        .await
        .expect("权威身份可进入大厅");

    assert!(matches!(outcome, ControlPlaneLobbyEntryOutcome::Joined(_)));
    let intent = gateway
        .intent
        .lock()
        .expect("大厅意图锁可用")
        .clone()
        .expect("大厅意图已发送");
    assert_eq!(intent.agent_id(), agent_id());
    assert_eq!(intent.agent_instance_id(), agent_instance_id());
    assert_eq!(intent.catalog_id(), catalog_id());
    assert_eq!(
        intent.preferred_language().map(RoomLanguage::as_str),
        Some("zh-CN")
    );
    assert_eq!(
        intent.preferred_region().map(RoomRegion::as_str),
        Some("ap-southeast")
    );
}

#[tokio::test]
async fn 控制面返回其他目录时拒绝建立本地会话() {
    let gateway = Arc::new(固定大厅控制面::returning(
        Ok(joined(other_catalog_id())),
    ));
    let service = AgentLobbySessionService::new(gateway);

    let failure = service
        .enter(&identity(), &config())
        .await
        .expect_err("错配目录不能伪装成功");

    assert_eq!(
        failure.kind(),
        AgentLobbySessionFailureKind::InvalidControlPlaneResponse
    );
}

#[tokio::test]
async fn 未知提交保持独立错误语义供运行时先对账再重试() {
    let gateway = Arc::new(固定大厅控制面::returning(Err(
        ControlPlaneLobbyEntryFailure::new(ControlPlaneLobbyEntryFailureKind::UnknownCommit),
    )));
    let service = AgentLobbySessionService::new(gateway);

    let failure = service
        .enter(&identity(), &config())
        .await
        .expect_err("未知提交不能伪装为未加入");

    assert_eq!(
        failure.kind(),
        AgentLobbySessionFailureKind::EntryOutcomeUnknown
    );
}

#[tokio::test]
async fn 供给繁忙保留服务端重试时间() {
    let retry_at = UtcMillis::new(1_700_000_030_000).expect("测试时间有效");
    let gateway = Arc::new(固定大厅控制面::returning(Ok(
        ControlPlaneLobbyEntryOutcome::ProvisioningBusy { retry_at },
    )));
    let service = AgentLobbySessionService::new(gateway);

    let outcome = service
        .enter(&identity(), &config())
        .await
        .expect("供给繁忙是可重试结果");

    assert_eq!(
        outcome,
        ControlPlaneLobbyEntryOutcome::ProvisioningBusy { retry_at }
    );
}

fn config() -> AgentLobbySessionConfig {
    AgentLobbySessionConfig::new(
        catalog_id(),
        Some(RoomLanguage::new("zh-CN".to_owned()).expect("测试语言有效")),
        Some(RoomRegion::new("ap-southeast".to_owned()).expect("测试地区有效")),
    )
}

fn identity() -> BridgeAgentIdentity {
    let user_id = MatrixUserId::new("@agent:matrix.agent-room.test".to_owned())
        .expect("测试 Matrix 用户有效");
    BridgeAgentIdentity::new(
        agent_id(),
        "Codex Builder",
        user_id.as_str(),
        agent_instance_id(),
    )
    .expect("测试 Agent 身份有效")
}

fn joined(catalog_id: RoomCatalogId) -> ControlPlaneLobbyEntryOutcome {
    ControlPlaneLobbyEntryOutcome::Joined(JoinedAgentLobby::new(
        catalog_id,
        RoomInstanceId::from_uuid(uuid(ROOM_INSTANCE_ID)),
        MatrixRoomReference::new("!public:matrix.agent-room.test".to_owned())
            .expect("测试 Matrix 房间有效"),
        RoomReservationId::from_uuid(uuid(RESERVATION_ID)),
        LobbyAssignmentKind::New,
    ))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_ID))
}

fn agent_instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid(INSTANCE_ID))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(CATALOG_ID))
}

fn other_catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(OTHER_CATALOG_ID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}
