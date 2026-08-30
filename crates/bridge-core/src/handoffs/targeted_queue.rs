use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    handoff::{HandoffFailureCode, TargetedHandoff},
    ids::{AgentId, AgentInstanceId, HandoffId},
};

/// Bridge 从云端领取交接时必须绑定到一个精确实例，不能按 Agent 级别模糊消费。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffTarget {
    pub agent_id: AgentId,
    pub instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedHandoffReceipt {
    Consumed,
    Declined(HandoffFailureCode),
    Failed(HandoffFailureCode),
}

impl TargetedHandoffReceipt {
    pub const fn status(&self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::Declined(_) => "declined",
            Self::Failed(_) => "failed",
        }
    }

    pub const fn failure_code(&self) -> Option<&HandoffFailureCode> {
        match self {
            Self::Consumed => None,
            Self::Declined(code) | Self::Failed(code) => Some(code),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedHandoffQueueFailureKind {
    NotFound,
    Conflict,
    Expired,
    Denied,
    RateLimited,
    Unavailable,
    InvalidResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffQueueFailure {
    kind: TargetedHandoffQueueFailureKind,
}

impl TargetedHandoffQueueFailure {
    pub const fn new(kind: TargetedHandoffQueueFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> TargetedHandoffQueueFailureKind {
        self.kind
    }
}

/// 云端定向交接队列的 Bridge 侧端口。
///
/// Adapter 必须重新验证响应中的 Agent、实例、交接状态和完整领域时间线。服务端返回成功并不
/// 意味着响应可信；任何目标漂移或畸形字段都必须以 `InvalidResponse` 失败关闭。
pub trait TargetedHandoffQueueGateway: Send + Sync {
    fn claim_next(
        &self,
        target: TargetedHandoffTarget,
    ) -> PortFuture<'_, Result<Option<TargetedHandoff>, TargetedHandoffQueueFailure>>;

    fn record_receipt<'a>(
        &'a self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
        receipt: &'a TargetedHandoffReceipt,
    ) -> PortFuture<'a, Result<TargetedHandoff, TargetedHandoffQueueFailure>>;
}
