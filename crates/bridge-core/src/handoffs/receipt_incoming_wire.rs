use agent_room_application::ports::DeviceSignature;
use agent_room_domain::{
    handoff::HandoffFailureCode,
    ids::{AgentInstanceId, HandoffId},
};
use agent_room_protocol_conformance::generated::{HandoffReceiptEvent, HandoffReceiptStatus};

use super::{
    DecryptedHandoffToDeviceEvent, HandoffReceiptRecord, RemoteHandoffReceiptStatus,
    incoming_wire::{
        HandoffEnvelopeFailure, authentication_material, parse_actor, parse_time, parse_v7,
    },
    receipt_wire::HANDOFF_RECEIPT_EVENT_TYPE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedHandoffReceipt {
    pub record: HandoffReceiptRecord,
    pub canonical_event: Vec<u8>,
    pub signature: DeviceSignature,
}

pub(super) fn parse_receipt(
    event: &DecryptedHandoffToDeviceEvent,
) -> Result<ParsedHandoffReceipt, HandoffEnvelopeFailure> {
    if event.event_type().as_str() != HANDOFF_RECEIPT_EVENT_TYPE {
        return Err(HandoffEnvelopeFailure::WrongEventType);
    }
    let wire: HandoffReceiptEvent = serde_json::from_value(event.content().clone())
        .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    if wire.schema_version != "1.0"
        || wire.event_type != HANDOFF_RECEIPT_EVENT_TYPE
        || wire.correlation_id != wire.id
    {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    let (identity, _) = parse_actor(&wire.actor)?;
    if identity.matrix_user_id() != event.sender() {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    let status = receipt_status(&wire.status);
    let failure_code = match (status, wire.failure_code) {
        (RemoteHandoffReceiptStatus::Failed, Some(code)) => Some(
            HandoffFailureCode::new(code).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
        ),
        (RemoteHandoffReceiptStatus::Failed, None) | (_, Some(_)) => {
            return Err(HandoffEnvelopeFailure::InvalidEnvelope);
        }
        (_, None) => None,
    };
    let (canonical_event, signature) = authentication_material(event.content())?;
    Ok(ParsedHandoffReceipt {
        record: HandoffReceiptRecord::new(
            HandoffId::from_uuid(parse_v7(&wire.id)?),
            identity.agent_id(),
            identity.agent_instance_id(),
            AgentInstanceId::from_uuid(parse_v7(&wire.requester_instance_id)?),
            status,
            failure_code,
            parse_time(&wire.created_at)?,
        ),
        canonical_event,
        signature,
    })
}

const fn receipt_status(value: &HandoffReceiptStatus) -> RemoteHandoffReceiptStatus {
    match value {
        HandoffReceiptStatus::Delivered => RemoteHandoffReceiptStatus::Delivered,
        HandoffReceiptStatus::Consumed => RemoteHandoffReceiptStatus::Consumed,
        HandoffReceiptStatus::Declined => RemoteHandoffReceiptStatus::Declined,
        HandoffReceiptStatus::Revoked => RemoteHandoffReceiptStatus::Revoked,
        HandoffReceiptStatus::Expired => RemoteHandoffReceiptStatus::Expired,
        HandoffReceiptStatus::Failed => RemoteHandoffReceiptStatus::Failed,
    }
}
