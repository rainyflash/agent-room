use std::{future::Future, pin::Pin};

use agent_room_bridge_ipc::{
    IpcBootstrapDefaultAgentRequest, IpcDefaultAgentBootstrap, IpcMethod, IpcResponse,
};
use agent_room_bridge_local_adapter::{LocalBridgeClient, LocalBridgeClientFailure};

use crate::{
    bridge_supervisor::{BridgeSupervisor, SupervisorFailure},
    desktop_config::DesktopBridgeConfig,
    runtime_target::{DesktopAgentTarget, RuntimeTargetFailure, RuntimeTargetStore},
};

type BootstrapFuture<'a> = Pin<
    Box<dyn Future<Output = Result<IpcDefaultAgentBootstrap, AgentBootstrapFailure>> + Send + 'a>,
>;

/// 桌面首次引导只依赖这个窄端口，不让 Tauri 命令认识 IPC 握手细节。
pub(crate) trait DefaultAgentBootstrapGateway: Send + Sync {
    fn bootstrap(&self, preferred_language: Option<String>) -> BootstrapFuture<'_>;
}

pub(crate) struct LocalBridgeBootstrapGateway {
    client: LocalBridgeClient,
}

impl LocalBridgeBootstrapGateway {
    pub(crate) const fn new(client: LocalBridgeClient) -> Self {
        Self { client }
    }
}

impl DefaultAgentBootstrapGateway for LocalBridgeBootstrapGateway {
    fn bootstrap(&self, preferred_language: Option<String>) -> BootstrapFuture<'_> {
        Box::pin(async move {
            let response = self
                .client
                .invoke(IpcMethod::BootstrapDefaultAgent(
                    IpcBootstrapDefaultAgentRequest { preferred_language },
                ))
                .await
                .map_err(AgentBootstrapFailure::from)?;
            match response {
                IpcResponse::DefaultAgentBootstrap { bootstrap } => Ok(bootstrap),
                _ => Err(AgentBootstrapFailure::new(
                    "desktop.bridge.agent_bootstrap_failed",
                    false,
                )),
            }
        })
    }
}

/// 通过 Bridge 恢复幂等默认 Agent，并把响应收敛为桌面 Runtime 目标。
pub(crate) async fn bootstrap_agent_target(
    gateway: &dyn DefaultAgentBootstrapGateway,
    preferred_language: Option<String>,
) -> Result<DesktopAgentTarget, AgentBootstrapFailure> {
    let bootstrap = gateway.bootstrap(preferred_language.clone()).await?;
    let language = bootstrap
        .lobby_language
        .as_deref()
        .or(preferred_language.as_deref())
        .unwrap_or("en");
    DesktopAgentTarget::new(
        &bootstrap.agent_id,
        &bootstrap.public_lobby_catalog_id,
        language,
    )
    .map_err(|failure| AgentBootstrapFailure::new(failure.code(), failure.retryable()))
}

/// Runtime 绑定端口把持久化与进程生命周期放在同一个用例边界内。
pub(crate) trait AgentRuntimeBindingPort {
    fn ensure_reconfigurable(&self) -> Result<(), AgentRuntimeBindingFailure>;
    fn persist_target(&self, target: DesktopAgentTarget) -> Result<(), AgentRuntimeBindingFailure>;
    fn reconfigure_bridge(
        &self,
        config: DesktopBridgeConfig,
    ) -> Result<(), AgentRuntimeBindingFailure>;
}

pub(crate) struct DesktopAgentRuntimeBinding<'a> {
    bridge: &'a BridgeSupervisor,
    targets: &'a RuntimeTargetStore,
}

impl<'a> DesktopAgentRuntimeBinding<'a> {
    pub(crate) const fn new(bridge: &'a BridgeSupervisor, targets: &'a RuntimeTargetStore) -> Self {
        Self { bridge, targets }
    }
}

impl AgentRuntimeBindingPort for DesktopAgentRuntimeBinding<'_> {
    fn ensure_reconfigurable(&self) -> Result<(), AgentRuntimeBindingFailure> {
        self.bridge
            .ensure_reconfigurable()
            .map_err(AgentRuntimeBindingFailure::from)
    }

    fn persist_target(&self, target: DesktopAgentTarget) -> Result<(), AgentRuntimeBindingFailure> {
        self.targets.persist(target).map_err(|failure| {
            AgentRuntimeBindingFailure::new(
                "desktop.bridge.target_persist_failed",
                failure.retryable(),
            )
        })
    }

    fn reconfigure_bridge(
        &self,
        config: DesktopBridgeConfig,
    ) -> Result<(), AgentRuntimeBindingFailure> {
        self.bridge
            .reconfigure(config)
            .map_err(AgentRuntimeBindingFailure::from)
    }
}

/// 绑定顺序不可交换：先确认进程可控，再落盘，最后触发受控重启。
pub(crate) fn bind_agent_runtime(
    port: &dyn AgentRuntimeBindingPort,
    target: DesktopAgentTarget,
    config: DesktopBridgeConfig,
) -> Result<DesktopAgentTarget, AgentRuntimeBindingFailure> {
    port.ensure_reconfigurable()?;
    port.persist_target(target.clone())?;
    port.reconfigure_bridge(config)?;
    Ok(target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentBootstrapFailure {
    code: String,
    retryable: bool,
}

impl AgentBootstrapFailure {
    fn new(code: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            retryable,
        }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) const fn retryable(&self) -> bool {
        self.retryable
    }
}

impl From<LocalBridgeClientFailure> for AgentBootstrapFailure {
    fn from(failure: LocalBridgeClientFailure) -> Self {
        Self::new(failure.code(), failure.retryable())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRuntimeBindingFailure {
    code: String,
    retryable: bool,
}

impl AgentRuntimeBindingFailure {
    fn new(code: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            retryable,
        }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) const fn retryable(&self) -> bool {
        self.retryable
    }
}

impl From<SupervisorFailure> for AgentRuntimeBindingFailure {
    fn from(failure: SupervisorFailure) -> Self {
        Self::new(failure.code, failure.retryable)
    }
}

impl From<RuntimeTargetFailure> for AgentRuntimeBindingFailure {
    fn from(failure: RuntimeTargetFailure) -> Self {
        Self::new(failure.code(), failure.retryable())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use agent_room_bridge_ipc::IpcDefaultAgentBootstrap;

    use super::{
        AgentBootstrapFailure, AgentRuntimeBindingFailure, AgentRuntimeBindingPort,
        BootstrapFuture, DefaultAgentBootstrapGateway, bind_agent_runtime, bootstrap_agent_target,
    };
    use crate::{desktop_config::DesktopBridgeConfig, runtime_target::DesktopAgentTarget};

    const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const LOBBY_ID: &str = "0198b601-77a2-7f41-b4f4-940f291951b8";

    struct 固定引导网关;

    impl DefaultAgentBootstrapGateway for 固定引导网关 {
        fn bootstrap(&self, _preferred_language: Option<String>) -> BootstrapFuture<'_> {
            Box::pin(async {
                Ok(IpcDefaultAgentBootstrap {
                    agent_id: AGENT_ID.to_owned(),
                    display_name: "默认 Agent".to_owned(),
                    public_lobby_catalog_id: LOBBY_ID.to_owned(),
                    lobby_language: None,
                })
            })
        }
    }

    #[tokio::test]
    async fn 引导响应在大厅无语言时复用系统语言() {
        let target = bootstrap_agent_target(&固定引导网关, Some("zh-CN".to_owned()))
            .await
            .expect("引导目标有效");

        assert_eq!(target.agent_id(), AGENT_ID);
        assert_eq!(target.public_lobby_catalog_id(), LOBBY_ID);
        assert_eq!(target.lobby_language(), "zh-CN");
    }

    struct 记录绑定端口 {
        calls: Mutex<Vec<&'static str>>,
    }

    impl AgentRuntimeBindingPort for 记录绑定端口 {
        fn ensure_reconfigurable(&self) -> Result<(), AgentRuntimeBindingFailure> {
            self.calls.lock().expect("调用记录可用").push("ensure");
            Ok(())
        }

        fn persist_target(
            &self,
            _target: DesktopAgentTarget,
        ) -> Result<(), AgentRuntimeBindingFailure> {
            self.calls.lock().expect("调用记录可用").push("persist");
            Ok(())
        }

        fn reconfigure_bridge(
            &self,
            _config: DesktopBridgeConfig,
        ) -> Result<(), AgentRuntimeBindingFailure> {
            self.calls.lock().expect("调用记录可用").push("reconfigure");
            Ok(())
        }
    }

    #[test]
    fn 绑定按确认可控_持久化_受控重启的顺序执行() {
        let port = 记录绑定端口 {
            calls: Mutex::new(Vec::new()),
        };
        let target = DesktopAgentTarget::new(AGENT_ID, LOBBY_ID, "en").expect("目标有效");
        let config = DesktopBridgeConfig::from_environment()
            .expect("桌面配置有效")
            .with_agent_target(&target);

        bind_agent_runtime(&port, target, config).expect("绑定成功");

        assert_eq!(
            *port.calls.lock().expect("调用记录可用"),
            vec!["ensure", "persist", "reconfigure"]
        );
    }

    #[test]
    fn 引导故障保留稳定码与重试语义() {
        let failure = AgentBootstrapFailure::new("bridge.onboarding.unavailable", true);
        assert_eq!(failure.code(), "bridge.onboarding.unavailable");
        assert!(failure.retryable());
    }
}
