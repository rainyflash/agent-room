use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    content::ContentMediaType,
    handoff::{ContextHandoff, HandoffFailureCode, HandoffStatus},
    ids::{AgentId, AgentInstanceId, HandoffId, PrincipalId},
    time::UtcMillis,
};

use super::{
    ConsumedHandoffContext, EncryptedHandoffToDeviceRequest, HandoffDeviceAddress,
    OneShotHandoffPackage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffAuthorizationDecision {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffAuthorizationRequest {
    pub principal_id: PrincipalId,
    pub requester_agent_id: AgentId,
    pub requester_instance_id: AgentInstanceId,
    pub target_agent_id: AgentId,
    pub target_instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffAuthorizationFailureKind {
    Unavailable,
    InvalidResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffAuthorizationFailure {
    kind: HandoffAuthorizationFailureKind,
}

impl HandoffAuthorizationFailure {
    pub const fn new(kind: HandoffAuthorizationFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> HandoffAuthorizationFailureKind {
        self.kind
    }
}

pub trait HandoffAuthorizationGateway: Send + Sync {
    fn authorize<'a>(
        &'a self,
        request: &'a HandoffAuthorizationRequest,
    ) -> PortFuture<'a, Result<HandoffAuthorizationDecision, HandoffAuthorizationFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffDirectoryFailureKind {
    NotFound,
    Unavailable,
    InvalidResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffDirectoryFailure {
    kind: HandoffDirectoryFailureKind,
}

impl HandoffDirectoryFailure {
    pub const fn new(kind: HandoffDirectoryFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> HandoffDirectoryFailureKind {
        self.kind
    }
}

pub trait HandoffInstanceDirectory: Send + Sync {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, Result<HandoffDeviceAddress, HandoffDirectoryFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffTransportFailureKind {
    Rejected,
    Unavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffTransportFailure {
    kind: HandoffTransportFailureKind,
}

impl HandoffTransportFailure {
    pub const fn new(kind: HandoffTransportFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> HandoffTransportFailureKind {
        self.kind
    }
}

/// 只接受由 Matrix 端到端加密会话发送的设备命令。
///
/// 适配器不得降级为未加密的房间事件或普通 HTTP 回调。
pub trait EncryptedHandoffToDeviceGateway: Send + Sync {
    fn send<'a>(
        &'a self,
        request: &'a EncryptedHandoffToDeviceRequest,
    ) -> PortFuture<'a, Result<(), HandoffTransportFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffStoreFailureKind {
    Conflict,
    NotFound,
    Expired,
    AlreadyResolved,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffStoreFailure {
    kind: HandoffStoreFailureKind,
}

impl HandoffStoreFailure {
    pub const fn new(kind: HandoffStoreFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> HandoffStoreFailureKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffRecordOutcome {
    Created(ContextHandoff),
    Existing(ContextHandoff),
}

impl HandoffRecordOutcome {
    pub const fn handoff(&self) -> &ContextHandoff {
        match self {
            Self::Created(handoff) | Self::Existing(handoff) => handoff,
        }
    }

    pub const fn reused(&self) -> bool {
        matches!(self, Self::Existing(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffStoreCommand {
    MarkDelivered {
        occurred_at: UtcMillis,
    },
    Consume {
        target_instance_id: AgentInstanceId,
        occurred_at: UtcMillis,
    },
    Decline {
        target_instance_id: AgentInstanceId,
        occurred_at: UtcMillis,
    },
    Revoke {
        occurred_at: UtcMillis,
    },
    Expire {
        occurred_at: UtcMillis,
    },
    Fail {
        code: HandoffFailureCode,
        occurred_at: UtcMillis,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffStoreCommandOutcome {
    Updated(ContextHandoff),
    Consumed(ConsumedHandoffContext),
}

impl HandoffStoreCommandOutcome {
    pub const fn status(&self) -> HandoffStatus {
        match self {
            Self::Updated(handoff) => handoff.status(),
            Self::Consumed(context) => context.handoff().status(),
        }
    }
}

/// 持久化交付状态和一次性正文的单一事务边界。
///
/// `accept_incoming` 必须原子写入交付记录与正文；`Consume` 必须原子返回并删除正文，
/// 其他终态命令必须在同一事务中删除正文。实现不得把正文写入日志或错误详情。
pub trait HandoffStore: Send + Sync {
    fn find(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<ContextHandoff>, HandoffStoreFailure>>;

    fn record_outgoing<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>>;

    fn accept_incoming<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
        package: &'a OneShotHandoffPackage,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>>;

    fn apply(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
    ) -> PortFuture<'_, Result<HandoffStoreCommandOutcome, HandoffStoreFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffContentFailureKind {
    Denied,
    NotFound,
    Unavailable,
    InvalidResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffContentFailure {
    kind: HandoffContentFailureKind,
}

impl HandoffContentFailure {
    pub const fn new(kind: HandoffContentFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> HandoffContentFailureKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffContentRead {
    pub body: std::sync::Arc<[u8]>,
    pub media_type: ContentMediaType,
}

pub trait HandoffContentGateway: Send + Sync {
    /// 读取与交付记录精确绑定的正文。
    ///
    /// 实现必须同时验证房间、来源事件、消息、内容标识和调用方授权；不得接受载荷中的 URL
    /// 或绕过内容服务绑定关系直接下载任意地址。
    fn read<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffContentRead, HandoffContentFailure>>;
}
