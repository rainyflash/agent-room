use std::sync::Arc;

use agent_room_application::ports::{MatrixDeviceId, MatrixEvent, MatrixEventType, MatrixUserId};
use agent_room_domain::{
    DomainResult,
    handoff::{ContextHandoff, HandoffFailureCode, HandoffStatus},
    ids::{AgentId, AgentInstanceId, HandoffId, PrincipalId},
};

use crate::agent_identity::BridgeAgentIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffRequestError {
    SourceActorMismatch,
    InvalidEventContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteHandoffReceiptStatus {
    Delivered,
    Consumed,
    Declined,
    Revoked,
    Expired,
    Failed,
}

impl RemoteHandoffReceiptStatus {
    pub const fn as_handoff_status(self) -> HandoffStatus {
        match self {
            Self::Delivered => HandoffStatus::Delivered,
            Self::Consumed => HandoffStatus::Consumed,
            Self::Declined => HandoffStatus::Declined,
            Self::Revoked => HandoffStatus::Revoked,
            Self::Expired => HandoffStatus::Expired,
            Self::Failed => HandoffStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffReceiptRecord {
    handoff_id: HandoffId,
    target_agent_id: AgentId,
    target_instance_id: AgentInstanceId,
    requester_instance_id: AgentInstanceId,
    status: RemoteHandoffReceiptStatus,
    failure_code: Option<HandoffFailureCode>,
    occurred_at: agent_room_domain::time::UtcMillis,
}

impl HandoffReceiptRecord {
    pub const fn new(
        handoff_id: HandoffId,
        target_agent_id: AgentId,
        target_instance_id: AgentInstanceId,
        requester_instance_id: AgentInstanceId,
        status: RemoteHandoffReceiptStatus,
        failure_code: Option<HandoffFailureCode>,
        occurred_at: agent_room_domain::time::UtcMillis,
    ) -> Self {
        Self {
            handoff_id,
            target_agent_id,
            target_instance_id,
            requester_instance_id,
            status,
            failure_code,
            occurred_at,
        }
    }

    pub const fn handoff_id(&self) -> HandoffId {
        self.handoff_id
    }

    pub const fn target_agent_id(&self) -> AgentId {
        self.target_agent_id
    }

    pub const fn target_instance_id(&self) -> AgentInstanceId {
        self.target_instance_id
    }

    pub const fn requester_instance_id(&self) -> AgentInstanceId {
        self.requester_instance_id
    }

    pub const fn status(&self) -> RemoteHandoffReceiptStatus {
        self.status
    }

    pub const fn failure_code(&self) -> Option<&HandoffFailureCode> {
        self.failure_code.as_ref()
    }

    pub const fn occurred_at(&self) -> agent_room_domain::time::UtcMillis {
        self.occurred_at
    }

    /// 把已认证回执映射为领域状态迁移。
    ///
    /// # Errors
    ///
    /// 回执目标不匹配、状态顺序非法、时间越界或失败码缺失时返回领域错误。
    pub fn apply_to(&self, handoff: &mut ContextHandoff) -> DomainResult<()> {
        if handoff.fields().id != self.handoff_id
            || handoff.fields().target_agent_id != self.target_agent_id
            || handoff.fields().target_instance_id != self.target_instance_id
            || handoff.fields().requester_instance_id != self.requester_instance_id
        {
            return Err(agent_room_domain::DomainError::Forbidden {
                action: "把回执应用到其他上下文交付",
            });
        }
        if (self.status == RemoteHandoffReceiptStatus::Failed) != self.failure_code.is_some() {
            return Err(agent_room_domain::DomainError::InvariantViolation {
                entity: "handoff_receipt",
                rule: "只有失败回执能够携带且必须携带失败码",
            });
        }
        match self.status {
            RemoteHandoffReceiptStatus::Delivered => handoff.mark_delivered(self.occurred_at),
            RemoteHandoffReceiptStatus::Consumed => {
                if handoff.status() == HandoffStatus::Approved {
                    handoff.mark_delivered(self.occurred_at)?;
                }
                handoff.consume(self.occurred_at)
            }
            RemoteHandoffReceiptStatus::Declined => handoff.decline(self.occurred_at),
            RemoteHandoffReceiptStatus::Revoked => handoff.revoke(self.occurred_at),
            RemoteHandoffReceiptStatus::Expired => handoff.expire(self.occurred_at),
            RemoteHandoffReceiptStatus::Failed => {
                let failure_code = self.failure_code.clone().ok_or(
                    agent_room_domain::DomainError::InvariantViolation {
                        entity: "handoff_receipt",
                        rule: "失败回执必须携带失败码",
                    },
                )?;
                handoff.fail(failure_code, self.occurred_at)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffReceiptOutcome {
    handoff_id: HandoffId,
    status: HandoffStatus,
}

impl HandoffReceiptOutcome {
    pub const fn new(handoff_id: HandoffId, status: HandoffStatus) -> Self {
        Self { handoff_id, status }
    }

    pub const fn handoff_id(self) -> HandoffId {
        self.handoff_id
    }

    pub const fn status(self) -> HandoffStatus {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedHandoffToDeviceEvent {
    sender: MatrixUserId,
    event_type: MatrixEventType,
    content: serde_json::Value,
}

impl DecryptedHandoffToDeviceEvent {
    /// 接受由 Matrix 加密会话解密后的 To-Device 事件。
    ///
    /// # Errors
    ///
    /// 内容不是对象或超过协议事件大小上限时返回错误。
    pub fn new(
        sender: MatrixUserId,
        event_type: MatrixEventType,
        content: serde_json::Value,
    ) -> Result<Self, HandoffRequestError> {
        let serialized =
            serde_json::to_vec(&content).map_err(|_| HandoffRequestError::InvalidEventContent)?;
        if !content.is_object() || serialized.len() > 64 * 1_024 {
            return Err(HandoffRequestError::InvalidEventContent);
        }
        Ok(Self {
            sender,
            event_type,
            content,
        })
    }

    pub const fn sender(&self) -> &MatrixUserId {
        &self.sender
    }

    pub const fn event_type(&self) -> &MatrixEventType {
        &self.event_type
    }

    pub const fn content(&self) -> &serde_json::Value {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproveHandoffRequest {
    handoff: ContextHandoff,
    source_identity: BridgeAgentIdentity,
    principal_id: PrincipalId,
}

impl ApproveHandoffRequest {
    /// 创建等待用户批准的精确交付请求。
    ///
    /// # Errors
    ///
    /// 来源身份与正文所属消息的签名身份不一致时返回错误。
    pub fn new(
        handoff: ContextHandoff,
        source_identity: BridgeAgentIdentity,
        principal_id: PrincipalId,
    ) -> Result<Self, HandoffRequestError> {
        let source_actor = handoff.fields().source.actor();
        if source_actor.agent_id() != source_identity.agent_id()
            || source_actor.instance_id() != source_identity.agent_instance_id()
        {
            return Err(HandoffRequestError::SourceActorMismatch);
        }
        Ok(Self {
            handoff,
            source_identity,
            principal_id,
        })
    }

    pub const fn handoff(&self) -> &ContextHandoff {
        &self.handoff
    }

    pub(super) const fn handoff_mut(&mut self) -> &mut ContextHandoff {
        &mut self.handoff
    }

    pub const fn source_identity(&self) -> &BridgeAgentIdentity {
        &self.source_identity
    }

    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffDeviceAddress {
    agent: AgentId,
    instance: AgentInstanceId,
    matrix_user: MatrixUserId,
    matrix_device: MatrixDeviceId,
}

impl HandoffDeviceAddress {
    pub const fn new(
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        matrix_user_id: MatrixUserId,
        matrix_device_id: MatrixDeviceId,
    ) -> Self {
        Self {
            agent: agent_id,
            instance: instance_id,
            matrix_user: matrix_user_id,
            matrix_device: matrix_device_id,
        }
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent
    }

    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance
    }

    pub const fn matrix_user_id(&self) -> &MatrixUserId {
        &self.matrix_user
    }

    pub const fn matrix_device_id(&self) -> &MatrixDeviceId {
        &self.matrix_device
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedHandoffToDeviceRequest {
    target: HandoffDeviceAddress,
    event: MatrixEvent,
}

impl EncryptedHandoffToDeviceRequest {
    pub const fn new(target: HandoffDeviceAddress, event: MatrixEvent) -> Self {
        Self { target, event }
    }

    pub const fn target(&self) -> &HandoffDeviceAddress {
        &self.target
    }

    pub const fn event(&self) -> &MatrixEvent {
        &self.event
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneShotHandoffPackage {
    handoff_id: HandoffId,
    body: Arc<[u8]>,
}

impl OneShotHandoffPackage {
    pub const fn new(handoff_id: HandoffId, body: Arc<[u8]>) -> Self {
        Self { handoff_id, body }
    }

    pub const fn handoff_id(&self) -> HandoffId {
        self.handoff_id
    }

    pub const fn body(&self) -> &Arc<[u8]> {
        &self.body
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedHandoffContext {
    handoff: ContextHandoff,
    body: Arc<[u8]>,
}

impl ConsumedHandoffContext {
    pub const fn new(handoff: ContextHandoff, body: Arc<[u8]>) -> Self {
        Self { handoff, body }
    }

    pub const fn handoff(&self) -> &ContextHandoff {
        &self.handoff
    }

    pub const fn body(&self) -> &Arc<[u8]> {
        &self.body
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffDeliveryOutcome {
    Submitted {
        handoff_id: HandoffId,
        reused: bool,
    },
    DeliveryUncertain {
        handoff_id: HandoffId,
    },
    AlreadyResolved {
        handoff_id: HandoffId,
        status: HandoffStatus,
    },
    Failed {
        handoff_id: HandoffId,
        code: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffReceiptDelivery {
    Confirmed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffReceptionOutcome {
    Delivered {
        handoff_id: HandoffId,
        replayed: bool,
        receipt: HandoffReceiptDelivery,
    },
    AlreadyResolved {
        handoff_id: HandoffId,
        status: HandoffStatus,
        receipt: HandoffReceiptDelivery,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffConsumptionOutcome {
    context: ConsumedHandoffContext,
    receipt: HandoffReceiptDelivery,
}

impl HandoffConsumptionOutcome {
    pub const fn new(context: ConsumedHandoffContext, receipt: HandoffReceiptDelivery) -> Self {
        Self { context, receipt }
    }

    pub const fn context(&self) -> &ConsumedHandoffContext {
        &self.context
    }

    pub const fn receipt(&self) -> HandoffReceiptDelivery {
        self.receipt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffResolutionOutcome {
    handoff_id: HandoffId,
    status: HandoffStatus,
    receipt: HandoffReceiptDelivery,
}

impl HandoffResolutionOutcome {
    pub const fn new(
        handoff_id: HandoffId,
        status: HandoffStatus,
        receipt: HandoffReceiptDelivery,
    ) -> Self {
        Self {
            handoff_id,
            status,
            receipt,
        }
    }

    pub const fn handoff_id(self) -> HandoffId {
        self.handoff_id
    }

    pub const fn status(self) -> HandoffStatus {
        self.status
    }

    pub const fn receipt(self) -> HandoffReceiptDelivery {
        self.receipt
    }
}
