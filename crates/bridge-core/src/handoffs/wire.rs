use std::collections::BTreeMap;

use agent_room_application::ports::{
    DeviceSignature, MatrixEvent, MatrixEventType, MatrixTransactionId,
};
use agent_room_domain::{
    handoff::{HandoffPermission, HandoffPurpose},
    messages::MessageProvenance,
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::{
    ActorRef, AgentRef, ContentRef, HandoffPermission as WireHandoffPermission,
    HandoffPurpose as WireHandoffPurpose, HandoffRequestEvent, HandoffSource, Provenance,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{SecondsFormat, TimeZone as _, Utc};
use serde_json::Value;

use crate::{agent_identity::BridgeAgentIdentity, ports::DeviceSigningIdentity};

use super::ApproveHandoffRequest;

pub(super) const HANDOFF_REQUEST_EVENT_TYPE: &str = "org.agentroom.handoff.request.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HandoffWireFailure {
    InvalidIdentifier,
    Serialization,
    Signing,
}

pub(super) fn request_event(
    requester_identity: &BridgeAgentIdentity,
    signer: &dyn DeviceSigningIdentity,
    request: &ApproveHandoffRequest,
) -> Result<MatrixEvent, HandoffWireFailure> {
    let handoff = request.handoff();
    let fields = handoff.fields();
    let approved_at = handoff
        .approved_at()
        .ok_or(HandoffWireFailure::Serialization)?;
    let id = version_seven(fields.id.as_uuid())?;
    let unsigned = HandoffRequestEvent {
        actor: actor_ref(requester_identity, MessageProvenance::HumanConfirmedAgent)?,
        approved_at: rfc3339(approved_at)?,
        approved_by_principal_id: version_seven(request.principal_id().as_uuid())?,
        content: ContentRef {
            content_id: version_seven(fields.content.content_id().as_uuid())?,
            digest_sha256: hex(fields.content.digest().as_bytes()),
            fetch_mode: "on_demand".to_owned(),
            media_type: fields.content.media_type().as_str().to_owned(),
            size_bytes: fields.content.byte_length().value(),
            extensions: BTreeMap::new(),
        },
        correlation_id: id.clone(),
        created_at: rfc3339(approved_at)?,
        event_type: HANDOFF_REQUEST_EVENT_TYPE.to_owned(),
        expires_at: rfc3339(fields.expires_at)?,
        id,
        permissions: fields.permissions.iter().map(wire_permission).collect(),
        purpose: wire_purpose(fields.purpose),
        risk_flags: fields
            .risk_flags
            .iter()
            .map(|flag| flag.as_str().to_owned())
            .collect(),
        schema_version: "1.0".to_owned(),
        signature: String::new(),
        source: HandoffSource {
            actor: actor_ref(
                request.source_identity(),
                fields.source.actor().provenance(),
            )?,
            event_id: fields.source.event_id().as_str().to_owned(),
            message_id: version_seven(fields.source.message_id().as_uuid())?,
            room_id: fields.source.room_id().as_str().to_owned(),
            extensions: BTreeMap::new(),
        },
        target_agent_id: version_seven(fields.target_agent_id.as_uuid())?,
        target_instance_id: version_seven(fields.target_instance_id.as_uuid())?,
        extensions: BTreeMap::new(),
    };
    let mut content =
        serde_json::to_value(unsigned).map_err(|_| HandoffWireFailure::Serialization)?;
    remove_signature(&mut content)?;
    let canonical = serde_jcs::to_vec(&content).map_err(|_| HandoffWireFailure::Serialization)?;
    let signature = signer
        .sign(&canonical)
        .map_err(|_| HandoffWireFailure::Signing)?;
    insert_signature(&mut content, &signature)?;

    MatrixEvent::new(
        MatrixEventType::new(HANDOFF_REQUEST_EVENT_TYPE)
            .map_err(|_| HandoffWireFailure::Serialization)?,
        request_transaction_id(fields.id)?,
        content,
    )
    .map_err(|_| HandoffWireFailure::Serialization)
}

fn request_transaction_id(
    handoff_id: agent_room_domain::ids::HandoffId,
) -> Result<MatrixTransactionId, HandoffWireFailure> {
    MatrixTransactionId::new(format!("agent-room-handoff-{handoff_id}"))
        .map_err(|_| HandoffWireFailure::InvalidIdentifier)
}

fn actor_ref(
    identity: &BridgeAgentIdentity,
    provenance: MessageProvenance,
) -> Result<ActorRef, HandoffWireFailure> {
    Ok(ActorRef {
        agent: AgentRef {
            agent_id: version_seven(identity.agent_id().as_uuid())?,
            avatar_url: identity.avatar_url().map(str::to_owned),
            display_name: identity.display_name().to_owned(),
            matrix_user_id: identity.matrix_user_id().as_str().to_owned(),
            extensions: BTreeMap::new(),
        },
        instance_id: version_seven(identity.agent_instance_id().as_uuid())?,
        provenance: wire_provenance(provenance),
        extensions: BTreeMap::new(),
    })
}

const fn wire_permission(value: HandoffPermission) -> WireHandoffPermission {
    match value {
        HandoffPermission::ReadText => WireHandoffPermission::ReadText,
        HandoffPermission::ReadAttachments => WireHandoffPermission::ReadAttachments,
        HandoffPermission::IncludeMetadata => WireHandoffPermission::IncludeMetadata,
    }
}

const fn wire_purpose(value: HandoffPurpose) -> WireHandoffPurpose {
    match value {
        HandoffPurpose::Inspect => WireHandoffPurpose::Inspect,
        HandoffPurpose::Summarize => WireHandoffPurpose::Summarize,
        HandoffPurpose::ReplyDraft => WireHandoffPurpose::ReplyDraft,
    }
}

const fn wire_provenance(value: MessageProvenance) -> Provenance {
    match value {
        MessageProvenance::Human => Provenance::Human,
        MessageProvenance::HumanConfirmedAgent => Provenance::HumanConfirmedAgent,
        MessageProvenance::AutonomousAgent => Provenance::AutonomousAgent,
    }
}

fn version_seven(value: uuid::Uuid) -> Result<String, HandoffWireFailure> {
    if value.get_version() == Some(uuid::Version::SortRand) {
        Ok(value.to_string())
    } else {
        Err(HandoffWireFailure::InvalidIdentifier)
    }
}

fn rfc3339(value: UtcMillis) -> Result<String, HandoffWireFailure> {
    Utc.timestamp_millis_opt(value.value())
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(HandoffWireFailure::Serialization)
}

fn remove_signature(content: &mut Value) -> Result<(), HandoffWireFailure> {
    content
        .as_object_mut()
        .ok_or(HandoffWireFailure::Serialization)?
        .remove("signature")
        .ok_or(HandoffWireFailure::Serialization)?;
    Ok(())
}

fn insert_signature(
    content: &mut Value,
    signature: &DeviceSignature,
) -> Result<(), HandoffWireFailure> {
    content
        .as_object_mut()
        .ok_or(HandoffWireFailure::Serialization)?
        .insert(
            "signature".to_owned(),
            Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
        );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
