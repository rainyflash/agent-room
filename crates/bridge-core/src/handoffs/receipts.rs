use std::sync::Arc;

use agent_room_application::ports::Clock;

use crate::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{AgentEventAuthenticationDecision, AgentEventAuthenticator},
};

use super::{
    DecryptedHandoffToDeviceEvent, HandoffReceiptOutcome, HandoffStore, HandoffStoreFailure,
    receipt_incoming_wire::parse_receipt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffReceiptFailureKind {
    InvalidEnvelope,
    WrongRequester,
    UntrustedSender,
    AuthenticationUnavailable,
    Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffReceiptFailure {
    kind: HandoffReceiptFailureKind,
    store: Option<HandoffStoreFailure>,
}

impl HandoffReceiptFailure {
    const fn simple(kind: HandoffReceiptFailureKind) -> Self {
        Self { kind, store: None }
    }

    const fn store(failure: HandoffStoreFailure) -> Self {
        Self {
            kind: HandoffReceiptFailureKind::Store,
            store: Some(failure),
        }
    }

    pub const fn kind(self) -> HandoffReceiptFailureKind {
        self.kind
    }

    pub const fn store_failure(self) -> Option<HandoffStoreFailure> {
        self.store
    }
}

pub struct HandoffReceiptDependencies {
    pub identity: BridgeAgentIdentity,
    pub clock: Arc<dyn Clock>,
    pub authenticator: Arc<dyn AgentEventAuthenticator>,
    pub store: Arc<dyn HandoffStore>,
}

pub struct HandoffReceiptService {
    identity: BridgeAgentIdentity,
    clock: Arc<dyn Clock>,
    authenticator: Arc<dyn AgentEventAuthenticator>,
    store: Arc<dyn HandoffStore>,
}

impl HandoffReceiptService {
    pub fn new(dependencies: HandoffReceiptDependencies) -> Self {
        Self {
            identity: dependencies.identity,
            clock: dependencies.clock,
            authenticator: dependencies.authenticator,
            store: dependencies.store,
        }
    }

    /// 验证目标实例签名并把远端回执原子应用到本地发送记录。
    ///
    /// # Errors
    ///
    /// 信封非法、请求实例不匹配、签名不可信或存储无法推进状态时返回错误。
    pub async fn apply(
        &self,
        event: &DecryptedHandoffToDeviceEvent,
    ) -> Result<HandoffReceiptOutcome, HandoffReceiptFailure> {
        let parsed = parse_receipt(event).map_err(|_| {
            HandoffReceiptFailure::simple(HandoffReceiptFailureKind::InvalidEnvelope)
        })?;
        if parsed.record.requester_instance_id() != self.identity.agent_instance_id() {
            return Err(HandoffReceiptFailure::simple(
                HandoffReceiptFailureKind::WrongRequester,
            ));
        }
        let decision = self
            .authenticator
            .authenticate(
                parsed.record.target_agent_id(),
                parsed.record.target_instance_id(),
                self.clock.now(),
                &parsed.canonical_event,
                &parsed.signature,
            )
            .await
            .map_err(|_| {
                HandoffReceiptFailure::simple(HandoffReceiptFailureKind::AuthenticationUnavailable)
            })?;
        if decision != AgentEventAuthenticationDecision::Trusted {
            return Err(HandoffReceiptFailure::simple(
                HandoffReceiptFailureKind::UntrustedSender,
            ));
        }
        let existing = self
            .store
            .find(parsed.record.handoff_id())
            .await
            .map_err(HandoffReceiptFailure::store)?
            .ok_or_else(|| {
                HandoffReceiptFailure::simple(HandoffReceiptFailureKind::InvalidEnvelope)
            })?;
        if existing.fields().requester_agent_id != self.identity.agent_id()
            || existing.fields().requester_instance_id != self.identity.agent_instance_id()
            || existing.fields().target_agent_id != parsed.record.target_agent_id()
            || existing.fields().target_instance_id != parsed.record.target_instance_id()
        {
            return Err(HandoffReceiptFailure::simple(
                HandoffReceiptFailureKind::InvalidEnvelope,
            ));
        }
        let handoff = self
            .store
            .apply_receipt(&parsed.record)
            .await
            .map_err(HandoffReceiptFailure::store)?;
        Ok(HandoffReceiptOutcome::new(
            handoff.fields().id,
            handoff.status(),
        ))
    }
}
