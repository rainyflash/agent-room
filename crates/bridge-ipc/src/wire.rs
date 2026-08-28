use std::collections::BTreeMap;

use agent_room_bridge_core::ipc::{
    IpcCallerKind, IpcHandshakeAgreement, IpcHandshakeFailure, IpcHandshakeFailureKind,
    IpcHandshakeOffer, IpcInstallationId, IpcProtocolVersion, IpcScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::tools::{IpcMethod, IpcResponse};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcFrame {
    ClientHello {
        #[serde(rename = "installationId")]
        installation_id: String,
        caller: IpcCaller,
        #[serde(rename = "supportedVersions")]
        supported_versions: Vec<IpcVersion>,
        #[serde(rename = "requestedScopes")]
        requested_scopes: Vec<IpcScopeName>,
    },
    ServerChallenge {
        #[serde(rename = "challengeId")]
        challenge_id: Uuid,
        challenge: String,
        #[serde(rename = "selectedVersion")]
        selected_version: IpcVersion,
        #[serde(rename = "grantedScopes")]
        granted_scopes: Vec<IpcScopeName>,
    },
    ClientProof {
        #[serde(rename = "challengeId")]
        challenge_id: Uuid,
        proof: String,
    },
    ServerReady {
        #[serde(rename = "serverInstanceId")]
        server_instance_id: Uuid,
        #[serde(rename = "selectedVersion")]
        selected_version: IpcVersion,
        #[serde(rename = "grantedScopes")]
        granted_scopes: Vec<IpcScopeName>,
    },
    Request {
        #[serde(rename = "correlationId")]
        correlation_id: Uuid,
        method: IpcMethod,
    },
    Response {
        #[serde(rename = "correlationId")]
        correlation_id: Uuid,
        result: IpcResponse,
    },
    Error {
        #[serde(rename = "correlationId", skip_serializing_if = "Option::is_none")]
        correlation_id: Option<Uuid>,
        code: String,
        category: IpcErrorCategory,
        retryable: bool,
        details: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcCaller {
    McpServer,
    DesktopShell,
    DiagnosticCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpcVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcScopeName {
    BridgeStatusRead,
    SelfRead,
    AgentBootstrap,
    PreviewsRead,
    PresenceRead,
    ContentRead,
    StatusPublish,
    MessageSend,
    HandoffApprove,
    HandoffConsume,
    HandoffDecline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCategory {
    Validation,
    Authentication,
    Authorization,
    Conflict,
    DependencyUnavailable,
    IncompatibleVersion,
    Internal,
}

/// 把闭合线协议握手转换为不依赖 Serde 的核心提议。
///
/// # Errors
///
/// 帧类型错误、安装标识、版本或作用域非法时返回稳定握手错误。
pub fn client_offer_from_frame(
    frame: &IpcFrame,
) -> Result<(IpcInstallationId, IpcHandshakeOffer), IpcHandshakeFailure> {
    let IpcFrame::ClientHello {
        installation_id,
        caller,
        supported_versions,
        requested_scopes,
    } = frame
    else {
        return Err(IpcHandshakeFailure::new(
            "bridge.ipc.decode_client_hello",
            IpcHandshakeFailureKind::InvalidOffer,
        ));
    };
    let installation_id = IpcInstallationId::new(installation_id.clone())?;
    let versions = supported_versions
        .iter()
        .map(|version| IpcProtocolVersion::new(version.major, version.minor))
        .collect::<Result<Vec<_>, _>>()?;
    let scopes = requested_scopes.iter().copied().map(IpcScope::from);
    let offer = IpcHandshakeOffer::new((*caller).into(), versions, scopes)?;
    Ok((installation_id, offer))
}

pub fn server_agreement_frame(
    server_instance_id: Uuid,
    agreement: &IpcHandshakeAgreement,
) -> IpcFrame {
    IpcFrame::ServerReady {
        server_instance_id,
        selected_version: agreement.selected_version().into(),
        granted_scopes: agreement
            .granted_scopes()
            .iter()
            .copied()
            .map(IpcScopeName::from)
            .collect(),
    }
}

impl From<IpcCaller> for IpcCallerKind {
    fn from(value: IpcCaller) -> Self {
        match value {
            IpcCaller::McpServer => Self::McpServer,
            IpcCaller::DesktopShell => Self::DesktopShell,
            IpcCaller::DiagnosticCli => Self::DiagnosticCli,
        }
    }
}

impl From<IpcScopeName> for IpcScope {
    fn from(value: IpcScopeName) -> Self {
        match value {
            IpcScopeName::BridgeStatusRead => Self::BridgeStatusRead,
            IpcScopeName::SelfRead => Self::SelfRead,
            IpcScopeName::AgentBootstrap => Self::AgentBootstrap,
            IpcScopeName::PreviewsRead => Self::PreviewsRead,
            IpcScopeName::PresenceRead => Self::PresenceRead,
            IpcScopeName::ContentRead => Self::ContentRead,
            IpcScopeName::StatusPublish => Self::StatusPublish,
            IpcScopeName::MessageSend => Self::MessageSend,
            IpcScopeName::HandoffApprove => Self::HandoffApprove,
            IpcScopeName::HandoffConsume => Self::HandoffConsume,
            IpcScopeName::HandoffDecline => Self::HandoffDecline,
        }
    }
}

impl From<IpcScope> for IpcScopeName {
    fn from(value: IpcScope) -> Self {
        match value {
            IpcScope::BridgeStatusRead => Self::BridgeStatusRead,
            IpcScope::SelfRead => Self::SelfRead,
            IpcScope::AgentBootstrap => Self::AgentBootstrap,
            IpcScope::PreviewsRead => Self::PreviewsRead,
            IpcScope::PresenceRead => Self::PresenceRead,
            IpcScope::ContentRead => Self::ContentRead,
            IpcScope::StatusPublish => Self::StatusPublish,
            IpcScope::MessageSend => Self::MessageSend,
            IpcScope::HandoffApprove => Self::HandoffApprove,
            IpcScope::HandoffConsume => Self::HandoffConsume,
            IpcScope::HandoffDecline => Self::HandoffDecline,
        }
    }
}

impl From<IpcProtocolVersion> for IpcVersion {
    fn from(value: IpcProtocolVersion) -> Self {
        Self {
            major: value.major(),
            minor: value.minor(),
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::{IpcCallerKind, IpcProtocolVersion, IpcScope};

    use super::{IpcCaller, IpcFrame, IpcScopeName, IpcVersion, client_offer_from_frame};

    #[test]
    fn 客户端握手只接受闭合且经过领域校验的值() {
        let frame = IpcFrame::ClientHello {
            installation_id: "install_1".to_owned(),
            caller: IpcCaller::McpServer,
            supported_versions: vec![IpcVersion { major: 1, minor: 0 }],
            requested_scopes: vec![IpcScopeName::BridgeStatusRead],
        };

        let (installation_id, offer) = client_offer_from_frame(&frame).expect("握手帧有效");

        assert_eq!(installation_id.as_str(), "install_1");
        assert_eq!(offer.caller(), IpcCallerKind::McpServer);
        assert!(
            offer
                .supported_versions()
                .contains(&IpcProtocolVersion::V1_0)
        );
        assert!(
            offer
                .requested_scopes()
                .contains(&IpcScope::BridgeStatusRead)
        );
    }
}
