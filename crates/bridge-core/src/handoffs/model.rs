use std::sync::Arc;

use agent_room_application::ports::{MatrixDeviceId, MatrixEvent, MatrixUserId};
use agent_room_domain::{
    handoff::{ContextHandoff, HandoffStatus},
    ids::{AgentId, AgentInstanceId, HandoffId, PrincipalId},
};

use crate::agent_identity::BridgeAgentIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffRequestError {
    SourceActorMismatch,
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
