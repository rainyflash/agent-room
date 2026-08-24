use std::sync::Arc;

use agent_room_application::ports::Clock;
use agent_room_domain::{
    handoff::{HandoffFailureCode, HandoffStatus},
    ids::HandoffId,
};

use crate::{agent_identity::BridgeAgentIdentity, ports::DeviceSigningIdentity};

use super::{
    ApproveHandoffRequest, EncryptedHandoffToDeviceGateway, EncryptedHandoffToDeviceRequest,
    HandoffAuthorizationDecision, HandoffAuthorizationGateway, HandoffAuthorizationRequest,
    HandoffDeliveryOutcome, HandoffDirectoryFailureKind, HandoffInstanceDirectory, HandoffStore,
    HandoffStoreCommand, HandoffStoreFailure, HandoffTransportFailure, HandoffTransportFailureKind,
    wire::{HandoffWireFailure, request_event},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffDeliveryFailureKind {
    InvalidIntent,
    Unauthorized,
    AuthorizationUnavailable,
    SigningUnavailable,
    Serialization,
    DirectoryUnavailable,
    Store,
    Transport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffDeliveryFailure {
    kind: HandoffDeliveryFailureKind,
    store: Option<HandoffStoreFailure>,
    transport: Option<HandoffTransportFailure>,
}

impl HandoffDeliveryFailure {
    const fn simple(kind: HandoffDeliveryFailureKind) -> Self {
        Self {
            kind,
            store: None,
            transport: None,
        }
    }

    const fn store(failure: HandoffStoreFailure) -> Self {
        Self {
            kind: HandoffDeliveryFailureKind::Store,
            store: Some(failure),
            transport: None,
        }
    }

    const fn transport(failure: HandoffTransportFailure) -> Self {
        Self {
            kind: HandoffDeliveryFailureKind::Transport,
            store: None,
            transport: Some(failure),
        }
    }

    pub const fn kind(self) -> HandoffDeliveryFailureKind {
        self.kind
    }

    pub const fn store_failure(self) -> Option<HandoffStoreFailure> {
        self.store
    }

    pub const fn transport_failure(self) -> Option<HandoffTransportFailure> {
        self.transport
    }
}

pub struct HandoffDeliveryDependencies {
    pub identity: BridgeAgentIdentity,
    pub signer: Arc<dyn DeviceSigningIdentity>,
    pub clock: Arc<dyn Clock>,
    pub authorization: Arc<dyn HandoffAuthorizationGateway>,
    pub directory: Arc<dyn HandoffInstanceDirectory>,
    pub transport: Arc<dyn EncryptedHandoffToDeviceGateway>,
    pub store: Arc<dyn HandoffStore>,
}

pub struct HandoffDeliveryService {
    identity: BridgeAgentIdentity,
    signer: Arc<dyn DeviceSigningIdentity>,
    clock: Arc<dyn Clock>,
    authorization: Arc<dyn HandoffAuthorizationGateway>,
    directory: Arc<dyn HandoffInstanceDirectory>,
    transport: Arc<dyn EncryptedHandoffToDeviceGateway>,
    store: Arc<dyn HandoffStore>,
}

impl HandoffDeliveryService {
    pub fn new(dependencies: HandoffDeliveryDependencies) -> Self {
        Self {
            identity: dependencies.identity,
            signer: dependencies.signer,
            clock: dependencies.clock,
            authorization: dependencies.authorization,
            directory: dependencies.directory,
            transport: dependencies.transport,
            store: dependencies.store,
        }
    }

    /// 批准、签名并向精确 Matrix 设备发送上下文交付请求。
    ///
    /// # Errors
    ///
    /// 本机身份不匹配、授权被拒、签名失败、目录或存储不可用时返回阶段化错误。
    pub async fn approve_and_send(
        &self,
        mut request: ApproveHandoffRequest,
    ) -> Result<HandoffDeliveryOutcome, HandoffDeliveryFailure> {
        self.validate_requester(&request)?;
        let fields = request.handoff().fields();
        let authorization = HandoffAuthorizationRequest {
            principal_id: request.principal_id(),
            requester_agent_id: fields.requester_agent_id,
            requester_instance_id: fields.requester_instance_id,
            target_agent_id: fields.target_agent_id,
            target_instance_id: fields.target_instance_id,
        };
        let decision = self
            .authorization
            .authorize(&authorization)
            .await
            .map_err(|_| {
                HandoffDeliveryFailure::simple(HandoffDeliveryFailureKind::AuthorizationUnavailable)
            })?;
        if decision != HandoffAuthorizationDecision::Allowed {
            return Err(HandoffDeliveryFailure::simple(
                HandoffDeliveryFailureKind::Unauthorized,
            ));
        }

        let principal_id = request.principal_id();
        request
            .handoff_mut()
            .approve(principal_id, self.clock.now())
            .map_err(|_| {
                HandoffDeliveryFailure::simple(HandoffDeliveryFailureKind::InvalidIntent)
            })?;
        let event = request_event(&self.identity, self.signer.as_ref(), &request)
            .map_err(map_wire_failure)?;
        let record = self
            .store
            .record_outgoing(request.handoff())
            .await
            .map_err(HandoffDeliveryFailure::store)?;
        if record.handoff().status() != HandoffStatus::Approved {
            return Ok(HandoffDeliveryOutcome::AlreadyResolved {
                handoff_id: request.handoff().fields().id,
                status: record.handoff().status(),
            });
        }

        let target = match self
            .directory
            .resolve(request.handoff().fields().target_instance_id)
            .await
        {
            Ok(target)
                if target.agent_id() == request.handoff().fields().target_agent_id
                    && target.instance_id() == request.handoff().fields().target_instance_id =>
            {
                target
            }
            Ok(_) => {
                return self
                    .mark_failed(
                        request.handoff().fields().id,
                        "handoff.target_identity_mismatch",
                    )
                    .await;
            }
            Err(failure) if failure.kind() == HandoffDirectoryFailureKind::NotFound => {
                return self
                    .mark_failed(request.handoff().fields().id, "handoff.target_not_found")
                    .await;
            }
            Err(_) => {
                return Err(HandoffDeliveryFailure::simple(
                    HandoffDeliveryFailureKind::DirectoryUnavailable,
                ));
            }
        };
        let delivery = EncryptedHandoffToDeviceRequest::new(target, event);
        match self.transport.send(&delivery).await {
            Ok(()) => Ok(HandoffDeliveryOutcome::Submitted {
                handoff_id: request.handoff().fields().id,
                reused: record.reused(),
            }),
            Err(failure) if failure.kind() == HandoffTransportFailureKind::UnknownCommit => {
                Ok(HandoffDeliveryOutcome::DeliveryUncertain {
                    handoff_id: request.handoff().fields().id,
                })
            }
            Err(failure)
                if matches!(
                    failure.kind(),
                    HandoffTransportFailureKind::Rejected | HandoffTransportFailureKind::Internal
                ) =>
            {
                self.mark_failed(request.handoff().fields().id, "handoff.transport_rejected")
                    .await
            }
            Err(failure) => Err(HandoffDeliveryFailure::transport(failure)),
        }
    }

    fn validate_requester(
        &self,
        request: &ApproveHandoffRequest,
    ) -> Result<(), HandoffDeliveryFailure> {
        let fields = request.handoff().fields();
        if fields.requester_agent_id != self.identity.agent_id()
            || fields.requester_instance_id != self.identity.agent_instance_id()
        {
            return Err(HandoffDeliveryFailure::simple(
                HandoffDeliveryFailureKind::InvalidIntent,
            ));
        }
        Ok(())
    }

    async fn mark_failed(
        &self,
        handoff_id: HandoffId,
        code: &'static str,
    ) -> Result<HandoffDeliveryOutcome, HandoffDeliveryFailure> {
        let failure_code = HandoffFailureCode::new(code).map_err(|_| {
            HandoffDeliveryFailure::simple(HandoffDeliveryFailureKind::InvalidIntent)
        })?;
        self.store
            .apply(
                handoff_id,
                HandoffStoreCommand::Fail {
                    code: failure_code,
                    occurred_at: self.clock.now(),
                },
            )
            .await
            .map_err(HandoffDeliveryFailure::store)?;
        Ok(HandoffDeliveryOutcome::Failed {
            handoff_id,
            code: code.to_owned(),
        })
    }
}

const fn map_wire_failure(failure: HandoffWireFailure) -> HandoffDeliveryFailure {
    let kind = match failure {
        HandoffWireFailure::InvalidIdentifier => HandoffDeliveryFailureKind::InvalidIntent,
        HandoffWireFailure::Serialization => HandoffDeliveryFailureKind::Serialization,
        HandoffWireFailure::Signing => HandoffDeliveryFailureKind::SigningUnavailable,
    };
    HandoffDeliveryFailure::simple(kind)
}
