//! 网络 Agent 的在线状态：用本机 Bridge 同一个状态发布服务写房间状态。长轮询期间发“等待消息”
//! （`listeningUntil` 最多为当前时间加 15 秒），停止轮询后按租约转为离线；停用时先发“已离线”。
//! 每个网络 Agent 一个发布服务，续租节奏记在进程里：控制面重启后第一次发布就是新的开始。

use std::{collections::HashMap, sync::Arc};

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        Clock, MatrixEventId, MatrixResult, MatrixRoomId, MatrixStateEvent,
        NetworkAgentMatrixGateway, PortFuture, SecretValue,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    ports::{AgentStatusStatePublisher, StatusEventIdentifierFactory},
    status::{
        AgentStatusIntent, AgentStatusLeasePolicy, AgentStatusPublicationDependencies,
        AgentStatusPublicationService, AgentStatusRoomTarget,
    },
};
use agent_room_domain::{
    agent_status::AgentStatusVisibility, ids::NetworkAgentId, time::DurationMillis,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::speaking::SessionSigner;

/// 与本机 Bridge 相同：租约 5 分钟，约 2 分钟续一次。
const LEASE_LIFETIME_MILLIS: u64 = 300_000;
const RENEWAL_INTERVAL_MILLIS: u64 = 120_000;
const RENEWAL_JITTER_MILLIS: u64 = 15_000;

/// 以 Agent 自己的 Matrix 会话写状态事件。
struct SessionStatePublisher {
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    access_token: SecretValue,
}

impl AgentStatusStatePublisher for SessionStatePublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        self.matrix
            .send_state_event(&self.access_token, room_id, event)
    }
}

struct UuidV7Identifiers;

impl StatusEventIdentifierFactory for UuidV7Identifiers {
    fn event_id(&self) -> Uuid {
        Uuid::now_v7()
    }

    fn correlation_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

type SharedService = Arc<Mutex<AgentStatusPublicationService>>;

#[derive(Default)]
pub(super) struct Presence {
    services: Mutex<HashMap<NetworkAgentId, SharedService>>,
}

impl Presence {
    /// 在这个 Agent 所在的每个房间按需发布状态；失败只记日志，不影响收发。
    pub(super) async fn publish(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
        intent: &AgentStatusIntent,
    ) {
        let Some(service) = self.service_for(matrix, clock, session).await else {
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                "网络 Agent 的在线状态服务建不起来，这次不发布"
            );
            return;
        };
        let mut service = service.lock().await;
        for room in &session.rooms {
            let Ok(room_id) = MatrixRoomId::new(room.matrix_room_id.as_str()) else {
                continue;
            };
            let target = AgentStatusRoomTarget::new(room_id, AgentStatusVisibility::Coarse);
            if let Err(failure) = service.publish_if_due(&target, intent, entropy()).await {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    failure = ?failure.kind(),
                    "网络 Agent 的在线状态没发出去"
                );
            }
        }
    }

    /// 停用后不再续租。
    pub(super) async fn forget(&self, id: NetworkAgentId) {
        self.services.lock().await.remove(&id);
    }

    async fn service_for(
        &self,
        matrix: &Arc<dyn NetworkAgentMatrixGateway>,
        clock: &Arc<dyn Clock>,
        session: &NetworkAgentSession,
    ) -> Option<SharedService> {
        let mut services = self.services.lock().await;
        if let Some(service) = services.get(&session.network_agent_id) {
            return Some(service.clone());
        }
        let identity = BridgeAgentIdentity::new(
            session.agent_id,
            session.display_name.clone(),
            session.agent_matrix_user_id.clone(),
            session.agent_instance_id,
        )
        .ok()?;
        let policy = AgentStatusLeasePolicy::new(
            DurationMillis::new(LEASE_LIFETIME_MILLIS).ok()?,
            DurationMillis::new(RENEWAL_INTERVAL_MILLIS).ok()?,
            DurationMillis::new(RENEWAL_JITTER_MILLIS).ok()?,
        )
        .ok()?;
        let service = Arc::new(Mutex::new(AgentStatusPublicationService::new(
            AgentStatusPublicationDependencies {
                identity,
                signer: Arc::new(SessionSigner::from_seed(&session.instance_signing_seed)?),
                publisher: Arc::new(SessionStatePublisher {
                    matrix: matrix.clone(),
                    access_token: session.matrix_access_token.clone(),
                }),
                identifiers: Arc::new(UuidV7Identifiers),
                clock: clock.clone(),
            },
            policy,
        )));
        services.insert(session.network_agent_id, service.clone());
        Some(service)
    }
}

/// 续租时间的随机抖动，免得大家同时续租。
fn entropy() -> u64 {
    let bytes = Uuid::new_v4().into_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
