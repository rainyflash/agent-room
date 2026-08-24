use std::sync::Arc;

use agent_room_application::ports::{
    AgentInstanceSignatureVerifier, AgentInstanceVerificationRecord, DeviceSignature, PortFuture,
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId},
    time::UtcMillis,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEventAuthenticationDecision {
    Trusted,
    TrustedHistoricalRevoked,
    UnknownInstance,
    RevokedInstance,
    AgentInstanceMismatch,
    InvalidSignature,
    OutsideInstanceValidityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEventAuthenticationFailureKind {
    Unauthorized,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEventAuthenticationFailure {
    kind: AgentEventAuthenticationFailureKind,
}

impl AgentEventAuthenticationFailure {
    pub const fn new(kind: AgentEventAuthenticationFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> AgentEventAuthenticationFailureKind {
        self.kind
    }
}

/// 验证任意 Agent 协议事件的实例归属、有效期和 Ed25519 签名。
pub trait AgentEventAuthenticator: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        observed_at: UtcMillis,
        canonical_event: &'a [u8],
        signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceVerificationGatewayFailureKind {
    AuthenticationRejected,
    NotFound,
    Unavailable,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInstanceVerificationGatewayFailure {
    kind: AgentInstanceVerificationGatewayFailureKind,
}

impl AgentInstanceVerificationGatewayFailure {
    pub const fn new(kind: AgentInstanceVerificationGatewayFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> AgentInstanceVerificationGatewayFailureKind {
        self.kind
    }
}

pub type AgentInstanceVerificationGatewayResult<T> =
    Result<T, AgentInstanceVerificationGatewayFailure>;

pub trait AgentInstanceVerificationGateway: Send + Sync {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>>;
}

pub struct AgentInstanceMessageAuthenticator {
    verification: Arc<dyn AgentInstanceVerificationGateway>,
    signatures: Arc<dyn AgentInstanceSignatureVerifier>,
}

pub struct AgentInstanceMessageAuthenticatorDependencies {
    pub verification: Arc<dyn AgentInstanceVerificationGateway>,
    pub signatures: Arc<dyn AgentInstanceSignatureVerifier>,
}

impl AgentInstanceMessageAuthenticator {
    pub fn new(dependencies: AgentInstanceMessageAuthenticatorDependencies) -> Self {
        Self {
            verification: dependencies.verification,
            signatures: dependencies.signatures,
        }
    }

    async fn authenticate_internal(
        &self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        origin_server_timestamp: UtcMillis,
        canonical_event: &[u8],
        signature: &DeviceSignature,
    ) -> Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure> {
        let record = match self.verification.resolve(instance_id).await {
            Ok(record) => record,
            Err(failure) => return map_gateway_failure(failure),
        };
        if record.instance_id != instance_id
            || record
                .invalidated_at
                .is_some_and(|invalidated_at| invalidated_at < record.registered_at)
        {
            return Err(authentication_failure(
                AgentEventAuthenticationFailureKind::Internal,
            ));
        }
        if record.agent_id != agent_id {
            return Ok(AgentEventAuthenticationDecision::AgentInstanceMismatch);
        }
        if !self
            .signatures
            .verify(&record.public_signing_key, canonical_event, signature)
        {
            return Ok(AgentEventAuthenticationDecision::InvalidSignature);
        }
        if origin_server_timestamp < record.registered_at {
            return Ok(AgentEventAuthenticationDecision::OutsideInstanceValidityWindow);
        }
        match record.invalidated_at {
            Some(invalidated_at) if origin_server_timestamp >= invalidated_at => {
                Ok(AgentEventAuthenticationDecision::RevokedInstance)
            }
            Some(_) => Ok(AgentEventAuthenticationDecision::TrustedHistoricalRevoked),
            None => Ok(AgentEventAuthenticationDecision::Trusted),
        }
    }
}

impl AgentEventAuthenticator for AgentInstanceMessageAuthenticator {
    fn authenticate<'a>(
        &'a self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        observed_at: UtcMillis,
        canonical_event: &'a [u8],
        signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure>>
    {
        Box::pin(self.authenticate_internal(
            agent_id,
            instance_id,
            observed_at,
            canonical_event,
            signature,
        ))
    }
}

fn map_gateway_failure(
    failure: AgentInstanceVerificationGatewayFailure,
) -> Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure> {
    match failure.kind() {
        AgentInstanceVerificationGatewayFailureKind::NotFound => {
            Ok(AgentEventAuthenticationDecision::UnknownInstance)
        }
        AgentInstanceVerificationGatewayFailureKind::AuthenticationRejected => Err(
            authentication_failure(AgentEventAuthenticationFailureKind::Unauthorized),
        ),
        AgentInstanceVerificationGatewayFailureKind::Unavailable => Err(authentication_failure(
            AgentEventAuthenticationFailureKind::Unavailable,
        )),
        AgentInstanceVerificationGatewayFailureKind::InvalidResponse
        | AgentInstanceVerificationGatewayFailureKind::Internal => Err(authentication_failure(
            AgentEventAuthenticationFailureKind::Internal,
        )),
    }
}

const fn authentication_failure(
    kind: AgentEventAuthenticationFailureKind,
) -> AgentEventAuthenticationFailure {
    AgentEventAuthenticationFailure::new(kind)
}
