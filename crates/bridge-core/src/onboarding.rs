use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{AgentCreationRequestId, AgentId, RoomCatalogId},
    rooms::RoomLanguage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDefaultAgent {
    pub agent_id: AgentId,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePublicLobby {
    pub catalog_id: RoomCatalogId,
    pub language: Option<RoomLanguage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapDefaultAgent {
    pub preferred_language: Option<RoomLanguage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultAgentBootstrap {
    pub agent_id: AgentId,
    pub display_name: String,
    pub public_lobby_catalog_id: RoomCatalogId,
    pub lobby_language: Option<RoomLanguage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneOnboardingFailureKind {
    AuthenticationRejected,
    Unavailable,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlaneOnboardingFailure {
    kind: ControlPlaneOnboardingFailureKind,
}

impl ControlPlaneOnboardingFailure {
    pub const fn new(kind: ControlPlaneOnboardingFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ControlPlaneOnboardingFailureKind {
        self.kind
    }
}

pub type ControlPlaneOnboardingResult<T> = Result<T, ControlPlaneOnboardingFailure>;

/// 在已认证设备所属账户下幂等注册宿主人物，不复用默认 Agent。
pub trait HostAgentRegistrationGateway: Send + Sync {
    fn create_host_agent<'a>(
        &'a self,
        session_key: AgentCreationRequestId,
        display_name: &'a str,
    ) -> PortFuture<'a, ControlPlaneOnboardingResult<BridgeDefaultAgent>>;
}

pub trait ControlPlaneOnboardingGateway: Send + Sync {
    fn ensure_default_agent(
        &self,
    ) -> PortFuture<'_, ControlPlaneOnboardingResult<BridgeDefaultAgent>>;

    fn list_public_lobbies(
        &self,
    ) -> PortFuture<'_, ControlPlaneOnboardingResult<Vec<BridgePublicLobby>>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeOnboardingFailureKind {
    NotAuthorized,
    Unavailable,
    PublicLobbyUnavailable,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeOnboardingFailure {
    operation: &'static str,
    kind: BridgeOnboardingFailureKind,
}

impl BridgeOnboardingFailure {
    const fn new(operation: &'static str, kind: BridgeOnboardingFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> BridgeOnboardingFailureKind {
        self.kind
    }
}

pub type BridgeOnboardingResult<T> = Result<T, BridgeOnboardingFailure>;

pub struct BridgeOnboardingService {
    control_plane: Arc<dyn ControlPlaneOnboardingGateway>,
}

impl BridgeOnboardingService {
    pub fn new(control_plane: Arc<dyn ControlPlaneOnboardingGateway>) -> Self {
        Self { control_plane }
    }

    /// 使用设备会话幂等恢复默认 Agent，并选择最贴近系统语言的公开大厅。
    ///
    /// # Errors
    ///
    /// 设备未授权、控制面不可用、响应畸形或没有公开大厅时返回稳定失败。
    pub async fn bootstrap(
        &self,
        request: BootstrapDefaultAgent,
    ) -> BridgeOnboardingResult<DefaultAgentBootstrap> {
        const OPERATION: &str = "bridge.onboarding.bootstrap";
        let agent = self
            .control_plane
            .ensure_default_agent()
            .await
            .map_err(|failure| map_control_plane_failure(OPERATION, failure))?;
        let lobbies = self
            .control_plane
            .list_public_lobbies()
            .await
            .map_err(|failure| map_control_plane_failure(OPERATION, failure))?;
        let lobby = select_public_lobby(lobbies, request.preferred_language.as_ref()).ok_or(
            BridgeOnboardingFailure::new(
                OPERATION,
                BridgeOnboardingFailureKind::PublicLobbyUnavailable,
            ),
        )?;

        Ok(DefaultAgentBootstrap {
            agent_id: agent.agent_id,
            display_name: agent.display_name,
            public_lobby_catalog_id: lobby.catalog_id,
            lobby_language: lobby.language,
        })
    }
}

fn select_public_lobby(
    mut lobbies: Vec<BridgePublicLobby>,
    preferred_language: Option<&RoomLanguage>,
) -> Option<BridgePublicLobby> {
    let preferred = preferred_language.map(RoomLanguage::as_str);
    let exact = preferred.and_then(|preferred| {
        lobbies.iter().position(|lobby| {
            lobby
                .language
                .as_ref()
                .is_some_and(|language| language.as_str().eq_ignore_ascii_case(preferred))
        })
    });
    let base = preferred.and_then(|preferred| {
        let base = preferred.split('-').next()?;
        lobbies.iter().position(|lobby| {
            lobby.language.as_ref().is_some_and(|language| {
                language
                    .as_str()
                    .split('-')
                    .next()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(base))
            })
        })
    });
    let selected = exact.or(base).unwrap_or(0);
    (selected < lobbies.len()).then(|| lobbies.remove(selected))
}

const fn map_control_plane_failure(
    operation: &'static str,
    failure: ControlPlaneOnboardingFailure,
) -> BridgeOnboardingFailure {
    let kind = match failure.kind() {
        ControlPlaneOnboardingFailureKind::AuthenticationRejected => {
            BridgeOnboardingFailureKind::NotAuthorized
        }
        ControlPlaneOnboardingFailureKind::Unavailable => BridgeOnboardingFailureKind::Unavailable,
        ControlPlaneOnboardingFailureKind::InvalidResponse => {
            BridgeOnboardingFailureKind::InvalidResponse
        }
        ControlPlaneOnboardingFailureKind::Internal => BridgeOnboardingFailureKind::Internal,
    };
    BridgeOnboardingFailure::new(operation, kind)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_application::ports::PortFuture;
    use agent_room_domain::{
        ids::{AgentId, RoomCatalogId},
        rooms::RoomLanguage,
    };
    use uuid::Uuid;

    use super::{
        BootstrapDefaultAgent, BridgeDefaultAgent, BridgeOnboardingFailureKind,
        BridgeOnboardingService, BridgePublicLobby, ControlPlaneOnboardingGateway,
        ControlPlaneOnboardingResult,
    };

    struct 固定控制面 {
        lobbies: Vec<BridgePublicLobby>,
    }

    impl ControlPlaneOnboardingGateway for 固定控制面 {
        fn ensure_default_agent(
            &self,
        ) -> PortFuture<'_, ControlPlaneOnboardingResult<BridgeDefaultAgent>> {
            Box::pin(async {
                Ok(BridgeDefaultAgent {
                    agent_id: AgentId::from_uuid(Uuid::now_v7()),
                    display_name: "默认 Agent".to_owned(),
                })
            })
        }

        fn list_public_lobbies(
            &self,
        ) -> PortFuture<'_, ControlPlaneOnboardingResult<Vec<BridgePublicLobby>>> {
            let lobbies = self.lobbies.clone();
            Box::pin(async move { Ok(lobbies) })
        }
    }

    #[tokio::test]
    async fn 优先精确语言再退回主语言() {
        let english = lobby("en-US");
        let chinese = lobby("zh-CN");
        let service = BridgeOnboardingService::new(Arc::new(固定控制面 {
            lobbies: vec![english.clone(), chinese.clone()],
        }));

        let exact = service
            .bootstrap(BootstrapDefaultAgent {
                preferred_language: Some(RoomLanguage::new("zh-CN").expect("语言有效")),
            })
            .await
            .expect("精确语言可选择");
        let base = service
            .bootstrap(BootstrapDefaultAgent {
                preferred_language: Some(RoomLanguage::new("zh-TW").expect("语言有效")),
            })
            .await
            .expect("主语言可退回");

        assert_eq!(exact.public_lobby_catalog_id, chinese.catalog_id);
        assert_eq!(base.public_lobby_catalog_id, chinese.catalog_id);
    }

    #[tokio::test]
    async fn 没有公开大厅时明确失败() {
        let service = BridgeOnboardingService::new(Arc::new(固定控制面 {
            lobbies: Vec::new(),
        }));

        let failure = service
            .bootstrap(BootstrapDefaultAgent {
                preferred_language: None,
            })
            .await
            .expect_err("空目录必须失败");

        assert_eq!(
            failure.kind(),
            BridgeOnboardingFailureKind::PublicLobbyUnavailable
        );
    }

    fn lobby(language: &str) -> BridgePublicLobby {
        BridgePublicLobby {
            catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
            language: Some(RoomLanguage::new(language).expect("测试语言有效")),
        }
    }
}
