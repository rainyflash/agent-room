use std::sync::Arc;

use agent_room_application::ports::Clock;
use agent_room_domain::{
    handoff::{ContextHandoff, HandoffStatus},
    ids::HandoffId,
};
use sha2::{Digest as _, Sha256};

use crate::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{AgentEventAuthenticationDecision, AgentEventAuthenticator},
    ports::DeviceSigningIdentity,
};

use super::{
    DecryptedHandoffToDeviceEvent, EncryptedHandoffToDeviceGateway,
    EncryptedHandoffToDeviceRequest, HandoffAuthorizationDecision, HandoffAuthorizationGateway,
    HandoffAuthorizationRequest, HandoffConsumptionOutcome, HandoffContentFailure,
    HandoffContentGateway, HandoffInstanceDirectory, HandoffReceiptDelivery,
    HandoffReceptionOutcome, HandoffRecordOutcome, HandoffResolutionOutcome, HandoffStore,
    HandoffStoreCommand, HandoffStoreCommandOutcome, HandoffStoreFailure, HandoffStoreFailureKind,
    OneShotHandoffPackage,
    incoming_wire::{HandoffEnvelopeFailure, parse_request},
    receipt_wire::receipt_event,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffReceptionFailureKind {
    InvalidEnvelope,
    WrongTarget,
    UntrustedSender,
    AuthenticationUnavailable,
    Unauthorized,
    AuthorizationUnavailable,
    Expired,
    Content,
    IntegrityMismatch,
    Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffReceptionFailure {
    kind: HandoffReceptionFailureKind,
    content: Option<HandoffContentFailure>,
    store: Option<HandoffStoreFailure>,
}

impl HandoffReceptionFailure {
    const fn simple(kind: HandoffReceptionFailureKind) -> Self {
        Self {
            kind,
            content: None,
            store: None,
        }
    }

    const fn content(failure: HandoffContentFailure) -> Self {
        Self {
            kind: HandoffReceptionFailureKind::Content,
            content: Some(failure),
            store: None,
        }
    }

    const fn store(failure: HandoffStoreFailure) -> Self {
        Self {
            kind: HandoffReceptionFailureKind::Store,
            content: None,
            store: Some(failure),
        }
    }

    pub const fn kind(self) -> HandoffReceptionFailureKind {
        self.kind
    }

    pub const fn content_failure(self) -> Option<HandoffContentFailure> {
        self.content
    }

    pub const fn store_failure(self) -> Option<HandoffStoreFailure> {
        self.store
    }
}

pub struct HandoffReceptionDependencies {
    pub identity: BridgeAgentIdentity,
    pub signer: Arc<dyn DeviceSigningIdentity>,
    pub clock: Arc<dyn Clock>,
    pub authenticator: Arc<dyn AgentEventAuthenticator>,
    pub authorization: Arc<dyn HandoffAuthorizationGateway>,
    pub directory: Arc<dyn HandoffInstanceDirectory>,
    pub transport: Arc<dyn EncryptedHandoffToDeviceGateway>,
    pub content: Arc<dyn HandoffContentGateway>,
    pub store: Arc<dyn HandoffStore>,
}

pub struct HandoffReceptionService {
    identity: BridgeAgentIdentity,
    signer: Arc<dyn DeviceSigningIdentity>,
    clock: Arc<dyn Clock>,
    authenticator: Arc<dyn AgentEventAuthenticator>,
    authorization: Arc<dyn HandoffAuthorizationGateway>,
    directory: Arc<dyn HandoffInstanceDirectory>,
    transport: Arc<dyn EncryptedHandoffToDeviceGateway>,
    content: Arc<dyn HandoffContentGateway>,
    store: Arc<dyn HandoffStore>,
}

impl HandoffReceptionService {
    pub fn new(dependencies: HandoffReceptionDependencies) -> Self {
        Self {
            identity: dependencies.identity,
            signer: dependencies.signer,
            clock: dependencies.clock,
            authenticator: dependencies.authenticator,
            authorization: dependencies.authorization,
            directory: dependencies.directory,
            transport: dependencies.transport,
            content: dependencies.content,
            store: dependencies.store,
        }
    }

    /// 验证并接收一个加密 To-Device 上下文交付请求。
    ///
    /// 正文只有在签名、目标、用户归属和时效全部通过后才会下载；落库后才发送回执。
    ///
    /// # Errors
    ///
    /// 信封、身份、授权、正文完整性或原子存储失败时返回阶段化错误。
    pub async fn receive(
        &self,
        event: &DecryptedHandoffToDeviceEvent,
    ) -> Result<HandoffReceptionOutcome, HandoffReceptionFailure> {
        let parsed = parse_request(event).map_err(map_envelope_failure)?;
        self.verify_target(&parsed.handoff)?;
        let observed_at = self.clock.now();
        if observed_at >= parsed.handoff.fields().expires_at {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Expired,
            ));
        }
        let decision = self
            .authenticator
            .authenticate(
                parsed.handoff.fields().requester_agent_id,
                parsed.handoff.fields().requester_instance_id,
                observed_at,
                &parsed.canonical_event,
                &parsed.signature,
            )
            .await
            .map_err(|_| {
                HandoffReceptionFailure::simple(
                    HandoffReceptionFailureKind::AuthenticationUnavailable,
                )
            })?;
        if decision != AgentEventAuthenticationDecision::Trusted {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::UntrustedSender,
            ));
        }

        if let Some(existing) = self
            .store
            .find(parsed.handoff.fields().id)
            .await
            .map_err(HandoffReceptionFailure::store)?
        {
            if !same_approved_request(&existing, &parsed.handoff) {
                return Err(HandoffReceptionFailure::simple(
                    HandoffReceptionFailureKind::InvalidEnvelope,
                ));
            }
            return Ok(self.replayed_outcome(existing).await);
        }

        self.authorize(&parsed.handoff).await?;
        let content = self
            .content
            .read(&parsed.handoff)
            .await
            .map_err(HandoffReceptionFailure::content)?;
        validate_content(&parsed.handoff, &content)?;
        let delivered_at = self.clock.now();
        if delivered_at >= parsed.handoff.fields().expires_at {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Expired,
            ));
        }
        let mut delivered = parsed.handoff;
        delivered.mark_delivered(delivered_at).map_err(|_| {
            HandoffReceptionFailure::simple(HandoffReceptionFailureKind::InvalidEnvelope)
        })?;
        let package = OneShotHandoffPackage::new(delivered.fields().id, content.body);
        let record = self
            .store
            .accept_incoming(&delivered, &package)
            .await
            .map_err(HandoffReceptionFailure::store)?;
        let stored = record.handoff().clone();
        if !same_approved_request(&stored, &delivered) {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::InvalidEnvelope,
            ));
        }
        Ok(self
            .outcome_for(stored, matches!(record, HandoffRecordOutcome::Existing(_)))
            .await)
    }

    /// 在破坏性消费前读取并校验当前实例的一次性交接元数据。
    ///
    /// # Errors
    ///
    /// 包不存在、已到期、已经终结、目标不匹配或存储不可用时返回错误。
    pub async fn inspect_pending(
        &self,
        handoff_id: HandoffId,
    ) -> Result<ContextHandoff, HandoffReceptionFailure> {
        let handoff = self
            .store
            .find(handoff_id)
            .await
            .map_err(HandoffReceptionFailure::store)?
            .ok_or_else(|| {
                HandoffReceptionFailure::store(HandoffStoreFailure::new(
                    HandoffStoreFailureKind::NotFound,
                ))
            })?;
        self.verify_target(&handoff)?;
        if self.clock.now() >= handoff.fields().expires_at {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Expired,
            ));
        }
        if handoff.status() != HandoffStatus::Delivered {
            return Err(HandoffReceptionFailure::store(HandoffStoreFailure::new(
                HandoffStoreFailureKind::AlreadyResolved,
            )));
        }
        Ok(handoff)
    }

    /// 原子领取并删除一个只属于当前实例的一次性上下文包。
    ///
    /// # Errors
    ///
    /// 包不存在、已到期、已被消费、目标不匹配或存储不可用时返回错误。
    pub async fn consume(
        &self,
        handoff_id: HandoffId,
    ) -> Result<HandoffConsumptionOutcome, HandoffReceptionFailure> {
        let occurred_at = self.clock.now();
        let outcome = self
            .store
            .apply(
                handoff_id,
                HandoffStoreCommand::Consume {
                    target_instance_id: self.identity.agent_instance_id(),
                    occurred_at,
                },
            )
            .await
            .map_err(HandoffReceptionFailure::store)?;
        let HandoffStoreCommandOutcome::Consumed(context) = outcome else {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Store,
            ));
        };
        let receipt = self.send_receipt(context.handoff(), occurred_at).await;
        Ok(HandoffConsumptionOutcome::new(context, receipt))
    }

    /// 拒绝并销毁尚未消费的上下文包。
    ///
    /// # Errors
    ///
    /// 交付不存在、目标不匹配、已经终结或存储不可用时返回错误。
    pub async fn decline(
        &self,
        handoff_id: HandoffId,
    ) -> Result<HandoffResolutionOutcome, HandoffReceptionFailure> {
        let occurred_at = self.clock.now();
        self.resolve(
            handoff_id,
            HandoffStoreCommand::Decline {
                target_instance_id: self.identity.agent_instance_id(),
                occurred_at,
            },
            occurred_at,
        )
        .await
    }

    /// 撤销并销毁当前实例尚未消费的上下文包。
    ///
    /// # Errors
    ///
    /// 交付不存在、目标不匹配、尚未批准、已经终结或存储不可用时返回错误。
    pub async fn revoke(
        &self,
        handoff_id: HandoffId,
    ) -> Result<HandoffResolutionOutcome, HandoffReceptionFailure> {
        let occurred_at = self.clock.now();
        self.resolve(
            handoff_id,
            HandoffStoreCommand::Revoke {
                target_instance_id: self.identity.agent_instance_id(),
                occurred_at,
            },
            occurred_at,
        )
        .await
    }

    /// 到期并销毁当前实例的上下文包。
    ///
    /// # Errors
    ///
    /// 尚未到期、目标不匹配、已经终结或存储不可用时返回错误。
    pub async fn expire(
        &self,
        handoff_id: HandoffId,
    ) -> Result<HandoffResolutionOutcome, HandoffReceptionFailure> {
        let occurred_at = self.clock.now();
        self.resolve(
            handoff_id,
            HandoffStoreCommand::Expire {
                target_instance_id: self.identity.agent_instance_id(),
                occurred_at,
            },
            occurred_at,
        )
        .await
    }

    async fn authorize(&self, handoff: &ContextHandoff) -> Result<(), HandoffReceptionFailure> {
        let principal_id = handoff.approved_by_principal_id().ok_or_else(|| {
            HandoffReceptionFailure::simple(HandoffReceptionFailureKind::InvalidEnvelope)
        })?;
        let fields = handoff.fields();
        let decision = self
            .authorization
            .authorize(&HandoffAuthorizationRequest {
                principal_id,
                requester_agent_id: fields.requester_agent_id,
                requester_instance_id: fields.requester_instance_id,
                target_agent_id: fields.target_agent_id,
                target_instance_id: fields.target_instance_id,
            })
            .await
            .map_err(|_| {
                HandoffReceptionFailure::simple(
                    HandoffReceptionFailureKind::AuthorizationUnavailable,
                )
            })?;
        if decision != HandoffAuthorizationDecision::Allowed {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Unauthorized,
            ));
        }
        Ok(())
    }

    async fn resolve(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
        occurred_at: agent_room_domain::time::UtcMillis,
    ) -> Result<HandoffResolutionOutcome, HandoffReceptionFailure> {
        let outcome = self
            .store
            .apply(handoff_id, command)
            .await
            .map_err(HandoffReceptionFailure::store)?;
        let HandoffStoreCommandOutcome::Updated(handoff) = outcome else {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::Store,
            ));
        };
        let receipt = self.send_receipt(&handoff, occurred_at).await;
        Ok(HandoffResolutionOutcome::new(
            handoff.fields().id,
            handoff.status(),
            receipt,
        ))
    }

    fn verify_target(&self, handoff: &ContextHandoff) -> Result<(), HandoffReceptionFailure> {
        if handoff.fields().target_agent_id != self.identity.agent_id()
            || handoff.fields().target_instance_id != self.identity.agent_instance_id()
        {
            return Err(HandoffReceptionFailure::simple(
                HandoffReceptionFailureKind::WrongTarget,
            ));
        }
        Ok(())
    }

    async fn replayed_outcome(&self, handoff: ContextHandoff) -> HandoffReceptionOutcome {
        self.outcome_for(handoff, true).await
    }

    async fn outcome_for(
        &self,
        handoff: ContextHandoff,
        replayed: bool,
    ) -> HandoffReceptionOutcome {
        let receipt = self.send_receipt(&handoff, self.clock.now()).await;
        if handoff.status() == HandoffStatus::Delivered {
            HandoffReceptionOutcome::Delivered {
                handoff_id: handoff.fields().id,
                replayed,
                receipt,
            }
        } else {
            HandoffReceptionOutcome::AlreadyResolved {
                handoff_id: handoff.fields().id,
                status: handoff.status(),
                receipt,
            }
        }
    }

    async fn send_receipt(
        &self,
        handoff: &ContextHandoff,
        occurred_at: agent_room_domain::time::UtcMillis,
    ) -> HandoffReceiptDelivery {
        let Ok(target) = self
            .directory
            .resolve(handoff.fields().requester_instance_id)
            .await
        else {
            return HandoffReceiptDelivery::Pending;
        };
        if target.agent_id() != handoff.fields().requester_agent_id
            || target.instance_id() != handoff.fields().requester_instance_id
        {
            return HandoffReceiptDelivery::Pending;
        }
        let Ok(event) = receipt_event(&self.identity, self.signer.as_ref(), handoff, occurred_at)
        else {
            return HandoffReceiptDelivery::Pending;
        };
        let request = EncryptedHandoffToDeviceRequest::new(target, event);
        match self.transport.send(&request).await {
            Ok(()) => HandoffReceiptDelivery::Confirmed,
            Err(_) => HandoffReceiptDelivery::Pending,
        }
    }
}

fn validate_content(
    handoff: &ContextHandoff,
    content: &super::HandoffContentRead,
) -> Result<(), HandoffReceptionFailure> {
    let reference = &handoff.fields().content;
    let byte_length = u64::try_from(content.body.len()).map_err(|_| {
        HandoffReceptionFailure::simple(HandoffReceptionFailureKind::IntegrityMismatch)
    })?;
    let digest = agent_room_domain::content::Sha256Digest::from_bytes(
        Sha256::digest(content.body.as_ref()).into(),
    );
    if byte_length != reference.byte_length().value()
        || content.media_type != *reference.media_type()
        || digest != reference.digest()
    {
        return Err(HandoffReceptionFailure::simple(
            HandoffReceptionFailureKind::IntegrityMismatch,
        ));
    }
    Ok(())
}

fn same_approved_request(left: &ContextHandoff, right: &ContextHandoff) -> bool {
    left.fields() == right.fields()
        && left.approved_by_principal_id() == right.approved_by_principal_id()
        && left.approved_at() == right.approved_at()
}

const fn map_envelope_failure(failure: HandoffEnvelopeFailure) -> HandoffReceptionFailure {
    match failure {
        HandoffEnvelopeFailure::WrongEventType | HandoffEnvelopeFailure::InvalidEnvelope => {
            HandoffReceptionFailure::simple(HandoffReceptionFailureKind::InvalidEnvelope)
        }
    }
}
