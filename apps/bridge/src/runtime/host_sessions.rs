use agent_room_bridge_core::onboarding::{
    ControlPlaneOnboardingFailureKind, HostAgentRegistrationGateway,
};
use agent_room_bridge_ipc::IpcOpenHostSessionRequest;
use agent_room_bridge_local_adapter::SecureStorageService;
use agent_room_domain::ids::AgentCreationRequestId;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};

use super::{
    AgentHandoffStores, AgentSessionTarget, BridgeConfig, BridgeExclusiveLock, BridgeRuntimeError,
    BridgeRuntimePaths, BridgeRuntimeStatus, BridgeSessionService, Clock, ControlPlaneHttpConfig,
    FoundationBridgeIpcRequestHandler, OsBridgeRuntimeSecretVault, PortFuture,
    ReqwestControlPlaneOnboardingGateway, SystemClock, compose_agent_session_runtime,
    establish_agent_online, initialize_handoff_store, initialize_matrix,
    initialize_targeted_handoff_inbox, is_reconnectable_agent_online_failure,
    maintain_agent_session,
};
use crate::{
    host_sessions::{HostSessionFactory, PreparedHostSession},
    ipc::BridgeIpcDispatchFailure,
};
use agent_room_domain::ids::AgentId;
use std::sync::Arc;
use tokio::sync::watch;

pub(super) struct HostAgentRuntimeFactory {
    config: BridgeConfig,
    paths: BridgeRuntimePaths,
    device_session: Arc<BridgeSessionService>,
    registration: Arc<dyn HostAgentRegistrationGateway>,
}

impl HostAgentRuntimeFactory {
    pub(super) fn new(
        config: BridgeConfig,
        paths: BridgeRuntimePaths,
        device_session: Arc<BridgeSessionService>,
    ) -> Result<Self, BridgeRuntimeError> {
        let registration = ReqwestControlPlaneOnboardingGateway::new(
            &ControlPlaneHttpConfig {
                base_url: config.control_plane_url.clone(),
                request_timeout: config.request_timeout,
            },
            device_session.clone(),
        )
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
        Ok(Self {
            config,
            paths,
            device_session,
            registration: Arc::new(registration),
        })
    }

    async fn prepare_runtime(
        &self,
        request: IpcOpenHostSessionRequest,
        shutdown: watch::Receiver<bool>,
    ) -> Result<PreparedHostSession, BridgeIpcDispatchFailure> {
        let lobby_catalog_id = self
            .config
            .public_lobby_catalog_id
            .ok_or_else(|| host_failure("bridge.host_session.lobby_required", false))?;
        let key = uuid::Uuid::parse_str(&request.session_key)
            .map_err(|_| host_failure("bridge.host_session.key_invalid", false))?;
        let agent = self
            .registration
            .create_host_agent(
                AgentCreationRequestId::from_uuid(key),
                &request.display_name,
            )
            .await
            .map_err(|failure| registration_failure(failure.kind()))?;
        let paths = self.paths.for_host_agent(agent.agent_id);
        paths
            .prepare()
            .map_err(BridgeRuntimeError::runtime_files)
            .map_err(BridgeIpcDispatchFailure::from)?;
        let store_lock = BridgeExclusiveLock::acquire(paths.matrix_store_lock_path())
            .map_err(BridgeRuntimeError::matrix_store_lock)
            .map_err(BridgeIpcDispatchFailure::from)?;
        // 只为子运行时选择独立存储命名空间；设备认证继续共享传入的服务，根 IPC 身份不变。
        let mut config = self.config.clone();
        config.secure_storage_service =
            host_storage_service(self.config.secure_storage_service.as_str(), agent.agent_id)
                .map_err(BridgeIpcDispatchFailure::from)?;
        let secrets = OsBridgeRuntimeSecretVault::system(config.secure_storage_service.as_str())
            .load_or_create()
            .map_err(BridgeRuntimeError::runtime_secrets)
            .map_err(BridgeIpcDispatchFailure::from)?;
        let matrix = initialize_matrix(&config, &paths, &secrets)
            .await
            .map_err(BridgeIpcDispatchFailure::from)?;
        let legacy = initialize_handoff_store(&paths, &secrets)
            .await
            .map_err(BridgeIpcDispatchFailure::from)?;
        let targeted = initialize_targeted_handoff_inbox(&paths)
            .await
            .map_err(BridgeIpcDispatchFailure::from)?;
        let mut runtime = compose_agent_session_runtime(
            &config,
            &paths,
            &secrets,
            self.device_session.clone(),
            matrix,
            AgentHandoffStores { legacy, targeted },
            AgentSessionTarget {
                agent_id: agent.agent_id,
                lobby_catalog_id,
            },
        )
        .await
        .map_err(BridgeIpcDispatchFailure::from)?;
        runtime.report_to_desktop_supervisor = false;
        let status = Arc::new(BridgeRuntimeStatus::new(SystemClock.now().value(), true));
        status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
        match establish_agent_online(&runtime).await {
            Ok(online) => {
                runtime.state.publish(&online);
                runtime.initial_session = Some(online);
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, true);
            }
            Err(failure) if is_reconnectable_agent_online_failure(failure) => {
                tracing::warn!(failure_kind = ?failure.kind(), "独立人物暂时无法上线，将在该会话中重连");
            }
            Err(failure) => {
                return Err(BridgeIpcDispatchFailure::from(
                    BridgeRuntimeError::agent_online(failure),
                ));
            }
        }
        status.finish_starting();
        let state = runtime.state.clone();
        let handler = Arc::new(FoundationBridgeIpcRequestHandler::with_agent_runtime(
            status.clone(),
            state.clone(),
            runtime.previews.clone(),
            runtime.content.clone(),
            Arc::new(SystemClock),
        ));
        Ok(PreparedHostSession {
            handler,
            run: Box::pin(async move {
                // 锁必须覆盖后台同步和全部 IPC 句柄的生命周期，不能在构造函数返回时释放。
                let _store_lock = store_lock;
                maintain_agent_session(Some(runtime), status, shutdown).await;
                state.clear();
            }),
        })
    }
}

impl HostSessionFactory for HostAgentRuntimeFactory {
    fn prepare(
        &self,
        request: IpcOpenHostSessionRequest,
        shutdown: watch::Receiver<bool>,
    ) -> PortFuture<'_, Result<PreparedHostSession, BridgeIpcDispatchFailure>> {
        Box::pin(self.prepare_runtime(request, shutdown))
    }
}

fn host_storage_service(
    parent_service: &str,
    agent_id: AgentId,
) -> Result<SecureStorageService, BridgeRuntimeError> {
    let digest = Sha256::digest(format!("{parent_service}\0{agent_id}").as_bytes());
    SecureStorageService::new(format!(
        "dev.agent-room.host.{}.v1",
        URL_SAFE_NO_PAD.encode(digest)
    ))
    .map_err(|_| BridgeRuntimeError::configuration("独立 Agent 存储命名空间无效".to_owned()))
}

impl From<BridgeRuntimeError> for BridgeIpcDispatchFailure {
    fn from(error: BridgeRuntimeError) -> Self {
        host_failure(error.code(), false)
    }
}

fn registration_failure(kind: ControlPlaneOnboardingFailureKind) -> BridgeIpcDispatchFailure {
    let code = match kind {
        ControlPlaneOnboardingFailureKind::AuthenticationRejected => {
            "bridge.host_session.device_authorization_required"
        }
        ControlPlaneOnboardingFailureKind::Unavailable => {
            "bridge.host_session.registration_unavailable"
        }
        ControlPlaneOnboardingFailureKind::InvalidResponse => {
            "bridge.host_session.registration_invalid"
        }
        ControlPlaneOnboardingFailureKind::Internal => "bridge.host_session.registration_failed",
    };
    host_failure(code, kind == ControlPlaneOnboardingFailureKind::Unavailable)
}

fn host_failure(code: &'static str, retryable: bool) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(
        code,
        agent_room_bridge_ipc::IpcErrorCategory::DependencyUnavailable,
        retryable,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 人物存储命名空间稳定且隔离安装与身份() {
        let first = AgentId::from_uuid(uuid::Uuid::now_v7());
        let second = AgentId::from_uuid(uuid::Uuid::now_v7());
        assert_eq!(
            host_storage_service("installation-a", first).unwrap(),
            host_storage_service("installation-a", first).unwrap()
        );
        assert_ne!(
            host_storage_service("installation-a", first).unwrap(),
            host_storage_service("installation-a", second).unwrap()
        );
        assert_ne!(
            host_storage_service("installation-a", first).unwrap(),
            host_storage_service("installation-b", first).unwrap()
        );
        assert!(host_storage_service(&"a".repeat(128), first).is_ok());
    }
}
