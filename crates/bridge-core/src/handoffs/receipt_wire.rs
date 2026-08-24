use std::collections::BTreeMap;

use agent_room_application::ports::{MatrixEvent, MatrixTransactionId};
use agent_room_domain::{
    handoff::{ContextHandoff, HandoffStatus},
    messages::MessageProvenance,
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::{HandoffReceiptEvent, HandoffReceiptStatus};

use crate::{agent_identity::BridgeAgentIdentity, ports::DeviceSigningIdentity};

use super::wire::{HandoffWireFailure, actor_ref, rfc3339, signed_event, version_seven};

pub(super) const HANDOFF_RECEIPT_EVENT_TYPE: &str = "org.agentroom.handoff.receipt.v1";

pub(super) fn receipt_event(
    identity: &BridgeAgentIdentity,
    signer: &dyn DeviceSigningIdentity,
    handoff: &ContextHandoff,
    occurred_at: UtcMillis,
) -> Result<MatrixEvent, HandoffWireFailure> {
    let status = receipt_status(handoff.status())?;
    let handoff_id = version_seven(handoff.fields().id.as_uuid())?;
    let unsigned = HandoffReceiptEvent {
        actor: actor_ref(identity, MessageProvenance::HumanConfirmedAgent)?,
        correlation_id: handoff_id.clone(),
        created_at: rfc3339(occurred_at)?,
        event_type: HANDOFF_RECEIPT_EVENT_TYPE.to_owned(),
        failure_code: handoff.failure_code().map(|code| code.as_str().to_owned()),
        id: handoff_id,
        requester_instance_id: version_seven(handoff.fields().requester_instance_id.as_uuid())?,
        schema_version: "1.0".to_owned(),
        signature: String::new(),
        status,
        extensions: BTreeMap::new(),
    };
    signed_event(
        HANDOFF_RECEIPT_EVENT_TYPE,
        receipt_transaction_id(handoff)?,
        &unsigned,
        signer,
    )
}

fn receipt_transaction_id(
    handoff: &ContextHandoff,
) -> Result<MatrixTransactionId, HandoffWireFailure> {
    MatrixTransactionId::new(format!(
        "agent-room-handoff-receipt-{}-{}",
        handoff.fields().id,
        handoff.status().as_str()
    ))
    .map_err(|_| HandoffWireFailure::InvalidIdentifier)
}

const fn receipt_status(status: HandoffStatus) -> Result<HandoffReceiptStatus, HandoffWireFailure> {
    match status {
        HandoffStatus::Delivered => Ok(HandoffReceiptStatus::Delivered),
        HandoffStatus::Consumed => Ok(HandoffReceiptStatus::Consumed),
        HandoffStatus::Declined => Ok(HandoffReceiptStatus::Declined),
        HandoffStatus::Revoked => Ok(HandoffReceiptStatus::Revoked),
        HandoffStatus::Expired => Ok(HandoffReceiptStatus::Expired),
        HandoffStatus::Failed => Ok(HandoffReceiptStatus::Failed),
        HandoffStatus::Proposed | HandoffStatus::Approved => Err(HandoffWireFailure::Serialization),
    }
}
