use agent_room_application::ports::{
    DeviceSignature, MatrixEvent, MatrixEventType, MatrixTransactionId,
};
use agent_room_domain::{
    ids::MessageSubmissionId,
    messages::{MessagePreview, MessageRelation},
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{SecondsFormat, TimeZone as _, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{agent_identity::BridgeAgentIdentity, ports::DeviceSigningIdentity};

use super::{
    EditMessageRequest, MessageContentRecord, MessageSubmissionFingerprint, RedactMessageRequest,
    SendMessageRequest,
};

pub(super) const PREVIEW_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.message.preview.v1";
pub(super) const REVISION_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.message.revision.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageWireFailure {
    InvalidIdentifier,
    Serialization,
    Signing,
}

pub(super) fn preview_transaction_id(
    submission_id: MessageSubmissionId,
) -> Result<MatrixTransactionId, MessageWireFailure> {
    MatrixTransactionId::new(format!("agent-room-message-{submission_id}"))
        .map_err(|_| MessageWireFailure::InvalidIdentifier)
}

pub(super) fn revision_transaction_id(
    submission_id: MessageSubmissionId,
) -> Result<MatrixTransactionId, MessageWireFailure> {
    MatrixTransactionId::new(format!("agent-room-revision-{submission_id}"))
        .map_err(|_| MessageWireFailure::InvalidIdentifier)
}

pub(super) fn preview_fingerprint(
    identity: &BridgeAgentIdentity,
    request: &SendMessageRequest,
) -> Result<MessageSubmissionFingerprint, MessageWireFailure> {
    fingerprint(&PreviewIntentFingerprint {
        kind: "preview",
        room_id: request.room_id().as_str(),
        actor: actor(identity, request.provenance().as_str()),
        preview: preview(request.preview()),
        body: body_fingerprint(request.body()),
        relation: request.relation().map(relation),
    })
}

pub(super) fn edit_fingerprint(
    identity: &BridgeAgentIdentity,
    request: &EditMessageRequest,
) -> Result<MessageSubmissionFingerprint, MessageWireFailure> {
    fingerprint(&EditIntentFingerprint {
        kind: "replace",
        room_id: request.room_id().as_str(),
        actor: actor(identity, request.provenance().as_str()),
        target_message_id: request.target_message_id().as_uuid(),
        preview: preview(request.preview()),
        body: body_fingerprint(request.body()),
    })
}

pub(super) fn redact_fingerprint(
    identity: &BridgeAgentIdentity,
    request: &RedactMessageRequest,
) -> Result<MessageSubmissionFingerprint, MessageWireFailure> {
    fingerprint(&RedactIntentFingerprint {
        kind: "redact",
        room_id: request.room_id().as_str(),
        actor: actor(identity, request.provenance().as_str()),
        target_message_id: request.target_message_id().as_uuid(),
        target_content_id: request.target_content_id().as_uuid(),
    })
}

pub(super) fn preview_event(
    identity: &BridgeAgentIdentity,
    signer: &dyn DeviceSigningIdentity,
    request: &SendMessageRequest,
    transaction_id: MatrixTransactionId,
    content: &MessageContentRecord,
) -> Result<MatrixEvent, MessageWireFailure> {
    let submission_id = request.submission_id();
    let unsigned = PreviewEvent {
        schema_version: "1.0",
        event_type: PREVIEW_EVENT_TYPE,
        id: submission_id.as_uuid(),
        created_at: submission_time(submission_id)?,
        actor: actor(identity, request.provenance().as_str()),
        correlation_id: submission_id.as_uuid(),
        room_id: request.room_id().as_str(),
        preview: preview(request.preview()),
        content: content_ref(content, request.body()),
        relation: request.relation().map(relation),
    };
    signed_event(PREVIEW_EVENT_TYPE, transaction_id, &unsigned, signer)
}

pub(super) fn edit_event(
    identity: &BridgeAgentIdentity,
    signer: &dyn DeviceSigningIdentity,
    request: &EditMessageRequest,
    transaction_id: MatrixTransactionId,
    content: &MessageContentRecord,
) -> Result<MatrixEvent, MessageWireFailure> {
    let submission_id = request.submission_id();
    let unsigned = ReplaceEvent {
        schema_version: "1.0",
        event_type: REVISION_EVENT_TYPE,
        id: submission_id.as_uuid(),
        created_at: submission_time(submission_id)?,
        actor: actor(identity, request.provenance().as_str()),
        correlation_id: submission_id.as_uuid(),
        room_id: request.room_id().as_str(),
        target_message_id: request.target_message_id().as_uuid(),
        kind: "replace",
        preview: preview(request.preview()),
        content: content_ref(content, request.body()),
    };
    signed_event(REVISION_EVENT_TYPE, transaction_id, &unsigned, signer)
}

pub(super) fn redact_event(
    identity: &BridgeAgentIdentity,
    signer: &dyn DeviceSigningIdentity,
    request: &RedactMessageRequest,
    transaction_id: MatrixTransactionId,
) -> Result<MatrixEvent, MessageWireFailure> {
    let submission_id = request.submission_id();
    let unsigned = RedactEvent {
        schema_version: "1.0",
        event_type: REVISION_EVENT_TYPE,
        id: submission_id.as_uuid(),
        created_at: submission_time(submission_id)?,
        actor: actor(identity, request.provenance().as_str()),
        correlation_id: submission_id.as_uuid(),
        room_id: request.room_id().as_str(),
        target_message_id: request.target_message_id().as_uuid(),
        kind: "redact",
    };
    signed_event(REVISION_EVENT_TYPE, transaction_id, &unsigned, signer)
}

fn fingerprint(value: &impl Serialize) -> Result<MessageSubmissionFingerprint, MessageWireFailure> {
    let canonical = serde_jcs::to_vec(value).map_err(|_| MessageWireFailure::Serialization)?;
    Ok(MessageSubmissionFingerprint::from_bytes(
        Sha256::digest(canonical).into(),
    ))
}

fn signed_event(
    event_type: &str,
    transaction_id: MatrixTransactionId,
    unsigned: &impl Serialize,
    signer: &dyn DeviceSigningIdentity,
) -> Result<MatrixEvent, MessageWireFailure> {
    let mut content =
        serde_json::to_value(unsigned).map_err(|_| MessageWireFailure::Serialization)?;
    let canonical = serde_jcs::to_vec(&content).map_err(|_| MessageWireFailure::Serialization)?;
    let signature = signer
        .sign(&canonical)
        .map_err(|_| MessageWireFailure::Signing)?;
    insert_signature(&mut content, &signature)?;
    MatrixEvent::new(
        MatrixEventType::new(event_type).map_err(|_| MessageWireFailure::Serialization)?,
        transaction_id,
        content,
    )
    .map_err(|_| MessageWireFailure::Serialization)
}

fn insert_signature(
    content: &mut Value,
    signature: &DeviceSignature,
) -> Result<(), MessageWireFailure> {
    let object = content
        .as_object_mut()
        .ok_or(MessageWireFailure::Serialization)?;
    object.insert(
        "signature".to_owned(),
        Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
    );
    Ok(())
}

fn submission_time(submission_id: MessageSubmissionId) -> Result<String, MessageWireFailure> {
    let timestamp = submission_id
        .as_uuid()
        .get_timestamp()
        .ok_or(MessageWireFailure::InvalidIdentifier)?;
    let (seconds, nanoseconds) = timestamp.to_unix();
    let milliseconds = seconds
        .checked_mul(1_000)
        .and_then(|value| value.checked_add(u64::from(nanoseconds) / 1_000_000))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(MessageWireFailure::InvalidIdentifier)?;
    rfc3339(UtcMillis::new(milliseconds).map_err(|_| MessageWireFailure::InvalidIdentifier)?)
}

fn rfc3339(value: UtcMillis) -> Result<String, MessageWireFailure> {
    Utc.timestamp_millis_opt(value.value())
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(MessageWireFailure::Serialization)
}

fn actor<'a>(identity: &'a BridgeAgentIdentity, provenance: &'a str) -> WireActor<'a> {
    WireActor {
        agent: WireAgent {
            agent_id: identity.agent_id().as_uuid(),
            display_name: identity.display_name(),
            matrix_user_id: identity.matrix_user_id().as_str(),
            avatar_url: identity.avatar_url(),
        },
        instance_id: identity.agent_instance_id().as_uuid(),
        provenance,
    }
}

fn preview(value: &MessagePreview) -> WirePreview<'_> {
    WirePreview {
        conversation: value.conversation().map(|chat| WireConversation {
            text: chat.text(),
            mentions: chat.mentions(),
        }),
        title: value.title().as_str(),
        summary: value.summary().as_str(),
        content_type: value.content_type().as_str(),
        language: value
            .language()
            .map(agent_room_domain::messages::MessageLanguage::as_str),
        sensitivity: value.sensitivity().as_str(),
        risk_flags: value
            .risk_flags()
            .iter()
            .map(agent_room_domain::messages::MessageRiskFlag::as_str)
            .collect(),
    }
}

fn body_fingerprint(value: &super::MessageBody) -> WireBodyFingerprint<'_> {
    WireBodyFingerprint {
        digest_sha256: hex(value.digest().as_bytes()),
        size_bytes: value.byte_length().value(),
        media_type: value.media_type().as_str(),
        encryption_mode: value.encryption_mode().as_str(),
        encryption: value.client_encryption().map(client_encryption),
        expires_at_unix_ms: value.expires_at().map(UtcMillis::value),
    }
}

fn content_ref<'a>(
    value: &'a MessageContentRecord,
    body: &'a super::MessageBody,
) -> WireContentRef<'a> {
    WireContentRef {
        content_id: value.content_id.as_uuid(),
        digest_sha256: hex(value.digest.as_bytes()),
        size_bytes: value.byte_length.value(),
        media_type: value.media_type.as_str(),
        fetch_mode: "on_demand",
        encryption: body.client_encryption().map(client_encryption),
    }
}

fn client_encryption(
    value: &agent_room_domain::messages::ClientContentEncryption,
) -> WireClientContentEncryption {
    WireClientContentEncryption {
        algorithm: value.algorithm().as_str(),
        context_id: value.context_id().as_uuid(),
        key_base64_url: URL_SAFE_NO_PAD.encode(value.key()),
        nonce_base64_url: URL_SAFE_NO_PAD.encode(value.nonce()),
        plaintext_size_bytes: value.plaintext_size_bytes(),
    }
}

const fn relation(value: MessageRelation) -> WireRelation {
    match value {
        MessageRelation::ReplyTo(message_id) => WireRelation {
            kind: "reply",
            target_message_id: message_id.as_uuid(),
        },
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAgent<'a> {
    agent_id: Uuid,
    display_name: &'a str,
    matrix_user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireActor<'a> {
    agent: WireAgent<'a>,
    instance_id: Uuid,
    provenance: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePreview<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation: Option<WireConversation<'a>>,
    title: &'a str,
    summary: &'a str,
    content_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    sensitivity: &'a str,
    risk_flags: Vec<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBodyFingerprint<'a> {
    digest_sha256: String,
    size_bytes: u64,
    media_type: &'a str,
    encryption_mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption: Option<WireClientContentEncryption>,
    expires_at_unix_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireContentRef<'a> {
    content_id: Uuid,
    digest_sha256: String,
    size_bytes: u64,
    media_type: &'a str,
    fetch_mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption: Option<WireClientContentEncryption>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireClientContentEncryption {
    algorithm: &'static str,
    context_id: Uuid,
    key_base64_url: String,
    nonce_base64_url: String,
    plaintext_size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRelation {
    kind: &'static str,
    target_message_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewIntentFingerprint<'a> {
    kind: &'static str,
    room_id: &'a str,
    actor: WireActor<'a>,
    preview: WirePreview<'a>,
    body: WireBodyFingerprint<'a>,
    relation: Option<WireRelation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditIntentFingerprint<'a> {
    kind: &'static str,
    room_id: &'a str,
    actor: WireActor<'a>,
    target_message_id: Uuid,
    preview: WirePreview<'a>,
    body: WireBodyFingerprint<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedactIntentFingerprint<'a> {
    kind: &'static str,
    room_id: &'a str,
    actor: WireActor<'a>,
    target_message_id: Uuid,
    target_content_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewEvent<'a> {
    schema_version: &'static str,
    event_type: &'static str,
    id: Uuid,
    created_at: String,
    actor: WireActor<'a>,
    correlation_id: Uuid,
    room_id: &'a str,
    preview: WirePreview<'a>,
    content: WireContentRef<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation: Option<WireRelation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceEvent<'a> {
    schema_version: &'static str,
    event_type: &'static str,
    id: Uuid,
    created_at: String,
    actor: WireActor<'a>,
    correlation_id: Uuid,
    room_id: &'a str,
    target_message_id: Uuid,
    kind: &'static str,
    preview: WirePreview<'a>,
    content: WireContentRef<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedactEvent<'a> {
    schema_version: &'static str,
    event_type: &'static str,
    id: Uuid,
    created_at: String,
    actor: WireActor<'a>,
    correlation_id: Uuid,
    room_id: &'a str,
    target_message_id: Uuid,
    kind: &'static str,
}

#[derive(Serialize)]
struct WireConversation<'a> {
    text: &'a str,
    mentions: &'a [String],
}
