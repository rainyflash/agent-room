use agent_room_application::ports::{MatrixRoomId, PortFuture};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, AutomationGrantId, MessageSubmissionId, RoomCatalogId},
    policy::AutomationRiskScanOutcome,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationAuthorizationRequest {
    pub grant_id: AutomationGrantId,
    pub submission_id: MessageSubmissionId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub room_catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomId,
    pub is_reply: bool,
    pub risk_scan: AutomationRiskScanOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationAuthorizationDenial {
    ControlPlaneRejected,
    GrantNotFound,
    GrantNotStarted,
    GrantRevoked,
    GrantExpired,
    AgentMismatch,
    InstanceMismatch,
    RoomMismatch,
    MessageKindNotAllowed,
    UnknownRecipientNotAllowed,
    RateLimitExceeded,
    TotalLimitExceeded,
    RiskScanRequired,
    RiskScanRejected,
    ActorMismatch,
    AuthorityChanged,
    MatrixPermissionDenied,
}

impl TryFrom<&str> for AutomationAuthorizationDenial {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "automation.grant_not_found" => Ok(Self::GrantNotFound),
            "automation.grant_not_started" => Ok(Self::GrantNotStarted),
            "automation.grant_revoked" => Ok(Self::GrantRevoked),
            "automation.grant_expired" => Ok(Self::GrantExpired),
            "automation.agent_mismatch" => Ok(Self::AgentMismatch),
            "automation.instance_mismatch" => Ok(Self::InstanceMismatch),
            "automation.room_mismatch" => Ok(Self::RoomMismatch),
            "automation.message_kind_not_allowed" => Ok(Self::MessageKindNotAllowed),
            "automation.unknown_recipient_not_allowed" => Ok(Self::UnknownRecipientNotAllowed),
            "automation.rate_limit_exceeded" => Ok(Self::RateLimitExceeded),
            "automation.total_limit_exceeded" => Ok(Self::TotalLimitExceeded),
            "automation.risk_scan_required" => Ok(Self::RiskScanRequired),
            "automation.risk_scan_rejected" => Ok(Self::RiskScanRejected),
            "automation.actor_mismatch" => Ok(Self::ActorMismatch),
            "automation.authority_changed" => Ok(Self::AuthorityChanged),
            "automation.matrix_permission_denied" => Ok(Self::MatrixPermissionDenied),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationAuthorizationFailureKind {
    Denied,
    Unavailable,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationAuthorizationFailure {
    kind: AutomationAuthorizationFailureKind,
    denial: Option<AutomationAuthorizationDenial>,
}

impl AutomationAuthorizationFailure {
    pub const fn denied(reason: AutomationAuthorizationDenial) -> Self {
        Self {
            kind: AutomationAuthorizationFailureKind::Denied,
            denial: Some(reason),
        }
    }

    pub const fn unavailable() -> Self {
        Self {
            kind: AutomationAuthorizationFailureKind::Unavailable,
            denial: None,
        }
    }

    pub const fn invalid_response() -> Self {
        Self {
            kind: AutomationAuthorizationFailureKind::InvalidResponse,
            denial: None,
        }
    }

    pub const fn internal() -> Self {
        Self {
            kind: AutomationAuthorizationFailureKind::Internal,
            denial: None,
        }
    }

    pub const fn kind(self) -> AutomationAuthorizationFailureKind {
        self.kind
    }

    pub const fn denial(self) -> Option<AutomationAuthorizationDenial> {
        self.denial
    }
}

pub type AutomationAuthorizationResult<T> = Result<T, AutomationAuthorizationFailure>;

/// 自动消息发布前的控制面权威检查。
///
/// 实现不得使用离线缓存乐观放行；控制面不可达、响应未知或无法验真时必须返回失败。
pub trait AutomationAuthorizationGateway: Send + Sync {
    fn authorize<'a>(
        &'a self,
        request: &'a AutomationAuthorizationRequest,
    ) -> PortFuture<'a, AutomationAuthorizationResult<()>>;
}
