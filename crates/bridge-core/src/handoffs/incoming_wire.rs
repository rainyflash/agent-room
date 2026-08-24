use std::collections::BTreeSet;

use agent_room_application::ports::DeviceSignature;
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffPermission,
        HandoffPermissions, HandoffPurpose, HandoffSource as DomainHandoffSource,
        HandoffSourceActor, HandoffSourceEventId,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    messages::{MessageProvenance, MessageRiskFlag, MessageRiskFlags},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::{
    ActorRef, HandoffPermission as WireHandoffPermission, HandoffPurpose as WireHandoffPurpose,
    HandoffRequestEvent, Provenance,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use uuid::{Uuid, Version};

use crate::agent_identity::BridgeAgentIdentity;

use super::{DecryptedHandoffToDeviceEvent, wire::HANDOFF_REQUEST_EVENT_TYPE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedHandoffRequest {
    pub handoff: ContextHandoff,
    pub canonical_event: Vec<u8>,
    pub signature: DeviceSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HandoffEnvelopeFailure {
    WrongEventType,
    InvalidEnvelope,
}

pub(super) fn parse_request(
    event: &DecryptedHandoffToDeviceEvent,
) -> Result<ParsedHandoffRequest, HandoffEnvelopeFailure> {
    if event.event_type().as_str() != HANDOFF_REQUEST_EVENT_TYPE {
        return Err(HandoffEnvelopeFailure::WrongEventType);
    }
    let wire: HandoffRequestEvent = serde_json::from_value(event.content().clone())
        .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    validate_header(&wire)?;
    let (canonical_event, signature) = authentication_material(event.content())?;
    let (requester_identity, requester_provenance) = parse_actor(&wire.actor)?;
    if requester_provenance != MessageProvenance::HumanConfirmedAgent
        || requester_identity.matrix_user_id() != event.sender()
    {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    let (source_identity, source_provenance) = parse_actor(&wire.source.actor)?;

    let handoff_id = HandoffId::from_uuid(parse_v7(&wire.id)?);
    let approved_at = parse_time(&wire.approved_at)?;
    if parse_time(&wire.created_at)? != approved_at {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    let expires_at = parse_time(&wire.expires_at)?;
    let principal_id = PrincipalId::from_uuid(parse_v7(&wire.approved_by_principal_id)?);
    let permissions = parse_permissions(&wire.permissions)?;
    let risk_flags = parse_risk_flags(&wire.risk_flags)?;
    let content = parse_content(&wire)?;
    let mut handoff = ContextHandoff::propose(ContextHandoffFields {
        id: handoff_id,
        requester_agent_id: requester_identity.agent_id(),
        requester_instance_id: requester_identity.agent_instance_id(),
        source: DomainHandoffSource::new(
            MatrixRoomReference::new(wire.source.room_id)
                .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
            HandoffSourceEventId::new(wire.source.event_id)
                .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
            MessageId::from_uuid(parse_v7(&wire.source.message_id)?),
            HandoffSourceActor::new(
                source_identity.agent_id(),
                source_identity.agent_instance_id(),
                source_provenance,
            ),
        ),
        target_agent_id: AgentId::from_uuid(parse_v7(&wire.target_agent_id)?),
        target_instance_id: AgentInstanceId::from_uuid(parse_v7(&wire.target_instance_id)?),
        content,
        permissions,
        purpose: parse_purpose(&wire.purpose),
        risk_flags,
        proposed_at: approved_at,
        expires_at,
    })
    .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    handoff
        .approve(principal_id, approved_at)
        .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;

    Ok(ParsedHandoffRequest {
        handoff,
        canonical_event,
        signature,
    })
}

fn validate_header(wire: &HandoffRequestEvent) -> Result<(), HandoffEnvelopeFailure> {
    if wire.schema_version != "1.0"
        || wire.event_type != HANDOFF_REQUEST_EVENT_TYPE
        || wire.correlation_id != wire.id
        || wire.content.fetch_mode != "on_demand"
        || wire
            .content
            .extensions
            .keys()
            .any(|key| matches!(key.as_str(), "downloadUrl" | "url"))
    {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    Ok(())
}

pub(super) fn authentication_material(
    content: &serde_json::Value,
) -> Result<(Vec<u8>, DeviceSignature), HandoffEnvelopeFailure> {
    let encoded = content
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .ok_or(HandoffEnvelopeFailure::InvalidEnvelope)?;
    let signature = DeviceSignature::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
    )
    .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    let mut unsigned = content.clone();
    unsigned
        .as_object_mut()
        .ok_or(HandoffEnvelopeFailure::InvalidEnvelope)?
        .remove("signature")
        .ok_or(HandoffEnvelopeFailure::InvalidEnvelope)?;
    let canonical_event =
        serde_jcs::to_vec(&unsigned).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    Ok((canonical_event, signature))
}

pub(super) fn parse_actor(
    actor: &ActorRef,
) -> Result<(BridgeAgentIdentity, MessageProvenance), HandoffEnvelopeFailure> {
    let mut identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(parse_v7(&actor.agent.agent_id)?),
        actor.agent.display_name.clone(),
        actor.agent.matrix_user_id.clone(),
        AgentInstanceId::from_uuid(parse_v7(&actor.instance_id)?),
    )
    .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    if let Some(avatar_url) = actor.agent.avatar_url.clone() {
        identity = identity
            .with_avatar_url(avatar_url)
            .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    }
    Ok((identity, parse_provenance(&actor.provenance)))
}

fn parse_content(
    wire: &HandoffRequestEvent,
) -> Result<HandoffContentReference, HandoffEnvelopeFailure> {
    Ok(HandoffContentReference::new(
        ContentId::from_uuid(parse_v7(&wire.content.content_id)?),
        parse_digest(&wire.content.digest_sha256)?,
        ContentByteLength::new(wire.content.size_bytes)
            .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
        ContentMediaType::new(wire.content.media_type.clone())
            .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?,
    ))
}

fn parse_permissions(
    values: &[WireHandoffPermission],
) -> Result<HandoffPermissions, HandoffEnvelopeFailure> {
    let permissions = values
        .iter()
        .map(|value| match value {
            WireHandoffPermission::ReadText => HandoffPermission::ReadText,
            WireHandoffPermission::ReadAttachments => HandoffPermission::ReadAttachments,
            WireHandoffPermission::IncludeMetadata => HandoffPermission::IncludeMetadata,
        })
        .collect::<Vec<_>>();
    let unique = permissions.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != permissions.len() {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    HandoffPermissions::new(permissions).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)
}

fn parse_risk_flags(values: &[String]) -> Result<MessageRiskFlags, HandoffEnvelopeFailure> {
    let flags = values
        .iter()
        .cloned()
        .map(MessageRiskFlag::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    let unique = flags
        .iter()
        .map(MessageRiskFlag::as_str)
        .collect::<BTreeSet<_>>();
    if unique.len() != flags.len() {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    MessageRiskFlags::new(flags).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)
}

const fn parse_purpose(value: &WireHandoffPurpose) -> HandoffPurpose {
    match value {
        WireHandoffPurpose::Inspect => HandoffPurpose::Inspect,
        WireHandoffPurpose::Summarize => HandoffPurpose::Summarize,
        WireHandoffPurpose::ReplyDraft => HandoffPurpose::ReplyDraft,
    }
}

const fn parse_provenance(value: &Provenance) -> MessageProvenance {
    match value {
        Provenance::Human => MessageProvenance::Human,
        Provenance::HumanConfirmedAgent => MessageProvenance::HumanConfirmedAgent,
        Provenance::AutonomousAgent => MessageProvenance::AutonomousAgent,
    }
}

pub(super) fn parse_v7(value: &str) -> Result<Uuid, HandoffEnvelopeFailure> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| {
            id.get_version() == Some(Version::SortRand) && id.to_string().as_str() == value
        })
        .ok_or(HandoffEnvelopeFailure::InvalidEnvelope)
}

pub(super) fn parse_time(value: &str) -> Result<UtcMillis, HandoffEnvelopeFailure> {
    let parsed =
        DateTime::parse_from_rfc3339(value).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    UtcMillis::new(parsed.timestamp_millis()).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)
}

fn parse_digest(value: &str) -> Result<Sha256Digest, HandoffEnvelopeFailure> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HandoffEnvelopeFailure::InvalidEnvelope);
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair =
            std::str::from_utf8(pair).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
        bytes[index] =
            u8::from_str_radix(pair, 16).map_err(|_| HandoffEnvelopeFailure::InvalidEnvelope)?;
    }
    Ok(Sha256Digest::from_bytes(bytes))
}
