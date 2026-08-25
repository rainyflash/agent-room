use std::sync::Arc;

use agent_room_application::ports::{
    DeviceSignature, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch,
    MatrixTimelineEvent,
};
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    ids::{
        AgentId, AgentInstanceId, ContentEncryptionContextId, ContentId, MessageId,
        MessageRevisionId,
    },
    messages::{
        CLIENT_CONTENT_KEY_BYTES, CLIENT_CONTENT_NONCE_BYTES, ClientContentEncryption,
        ClientContentEncryptionAlgorithm, MessageContentReference, MessageLanguage, MessagePreview,
        MessageProvenance, MessageRelation, MessageRevisionKind, MessageRiskFlag, MessageRiskFlags,
        MessageSensitivity, MessageSummary, MessageTitle,
    },
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::{
    ActorRef, ContentRef, MessagePreview as WireMessagePreview, MessagePreviewEvent,
    MessageRevisionEvent, MessageRevisionKind as WireMessageRevisionKind,
    MessageSensitivity as WireMessageSensitivity, Provenance,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use serde_json::Value;
use uuid::{Uuid, Version};

use crate::agent_identity::BridgeAgentIdentity;

pub use crate::agent_verification::{
    AgentEventAuthenticationDecision as MessageAuthenticationDecision,
    AgentEventAuthenticationFailure as MessageAuthenticationFailure,
    AgentEventAuthenticationFailureKind as MessageAuthenticationFailureKind,
    AgentEventAuthenticator as MessageEventAuthenticator,
};

use super::{
    MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailure,
    MessageStoreFailure, MessageSubmissionRepository, MessageSyncIssue, MessageSyncIssueReason,
    MessageTimelineGap, MessageTimelineProjectionStore, ProjectedMessageActor,
    ProjectedMessagePreview, ProjectedMessageRevision,
    wire::{PREVIEW_EVENT_TYPE, REVISION_EVENT_TYPE},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSyncFailureKind {
    SubmissionStore,
    Authentication,
    ProjectionStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSyncFailure {
    kind: MessageSyncFailureKind,
    submission_store: Option<MessageStoreFailure>,
    authentication: Option<MessageAuthenticationFailure>,
    projection_store: Option<MessageProjectionStoreFailure>,
}

impl MessageSyncFailure {
    const fn submission_store(failure: MessageStoreFailure) -> Self {
        Self {
            kind: MessageSyncFailureKind::SubmissionStore,
            submission_store: Some(failure),
            authentication: None,
            projection_store: None,
        }
    }

    const fn authentication(failure: MessageAuthenticationFailure) -> Self {
        Self {
            kind: MessageSyncFailureKind::Authentication,
            submission_store: None,
            authentication: Some(failure),
            projection_store: None,
        }
    }

    const fn projection_store(failure: MessageProjectionStoreFailure) -> Self {
        Self {
            kind: MessageSyncFailureKind::ProjectionStore,
            submission_store: None,
            authentication: None,
            projection_store: Some(failure),
        }
    }

    pub const fn kind(self) -> MessageSyncFailureKind {
        self.kind
    }

    pub const fn submission_store_failure(self) -> Option<MessageStoreFailure> {
        self.submission_store
    }

    pub const fn authentication_failure(self) -> Option<MessageAuthenticationFailure> {
        self.authentication
    }

    pub const fn projection_store_failure(self) -> Option<MessageProjectionStoreFailure> {
        self.projection_store
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSyncOutcome {
    pub accepted_events: usize,
    pub isolated_events: usize,
    pub timeline_gaps: usize,
    pub reconciled_submissions: usize,
}

pub struct MessageSyncDependencies {
    pub authenticator: Arc<dyn MessageEventAuthenticator>,
    pub projections: Arc<dyn MessageTimelineProjectionStore>,
    pub submissions: Arc<dyn MessageSubmissionRepository>,
}

pub struct MessageSyncService {
    authenticator: Arc<dyn MessageEventAuthenticator>,
    projections: Arc<dyn MessageTimelineProjectionStore>,
    submissions: Arc<dyn MessageSubmissionRepository>,
}

impl MessageSyncService {
    pub fn new(dependencies: MessageSyncDependencies) -> Self {
        Self {
            authenticator: dependencies.authenticator,
            projections: dependencies.projections,
            submissions: dependencies.submissions,
        }
    }

    /// 对账本机未知提交，验证消息事件并原子推进预览投影游标。
    ///
    /// 单个结构错误或不可信事件会被隔离；依赖不可用时整个批次不推进游标。
    ///
    /// # Errors
    ///
    /// 提交状态、实例验签服务或投影存储不可用时返回阶段化错误。
    pub async fn process(
        &self,
        sync: &MatrixSyncBatch,
    ) -> Result<MessageSyncOutcome, MessageSyncFailure> {
        let reconciled_submissions = self.reconcile_submissions(sync).await?;
        let mut mutations = Vec::new();
        let mut issues = Vec::new();
        let mut gaps = Vec::new();

        for room in sync
            .rooms()
            .iter()
            .filter(|room| room.kind() == MatrixRoomSyncKind::Joined)
        {
            if room.timeline_limited() {
                gaps.push(MessageTimelineGap {
                    room_id: room.room_id().clone(),
                    previous_batch: room.previous_batch().cloned(),
                });
            }
            for event in room.timeline() {
                if !is_message_event(event) {
                    continue;
                }
                match parse_pending_message(room.room_id(), event) {
                    Ok(pending) => {
                        let decision = self
                            .authenticator
                            .authenticate(
                                pending.agent_id,
                                pending.instance_id,
                                pending.origin_server_timestamp,
                                &pending.canonical_event,
                                &pending.signature,
                            )
                            .await
                            .map_err(MessageSyncFailure::authentication)?;
                        match decision {
                            MessageAuthenticationDecision::Trusted => {
                                mutations.push(pending.mutation);
                            }
                            MessageAuthenticationDecision::TrustedHistoricalRevoked => {
                                let mut mutation = pending.mutation;
                                mutation.mark_instance_revoked_after_event();
                                mutations.push(mutation);
                            }
                            _ => {
                                issues.push(issue(
                                    room.room_id(),
                                    event,
                                    authentication_issue(decision),
                                ));
                            }
                        }
                    }
                    Err(reason) => issues.push(issue(room.room_id(), event, reason)),
                }
            }
        }

        let outcome = MessageSyncOutcome {
            accepted_events: mutations.len(),
            isolated_events: issues.len(),
            timeline_gaps: gaps.len(),
            reconciled_submissions,
        };
        self.projections
            .apply(&MessageProjectionBatch::new(
                sync.next_batch().clone(),
                mutations,
                issues,
                gaps,
            ))
            .await
            .map_err(MessageSyncFailure::projection_store)?;
        Ok(outcome)
    }

    async fn reconcile_submissions(
        &self,
        sync: &MatrixSyncBatch,
    ) -> Result<usize, MessageSyncFailure> {
        let mut reconciled = 0;
        for event in sync.rooms().iter().flat_map(MatrixRoomSync::timeline) {
            let (Some(transaction_id), Some(event_id)) = (event.transaction_id(), event.event_id())
            else {
                continue;
            };
            if self
                .submissions
                .observe_transaction(transaction_id, event_id)
                .await
                .map_err(MessageSyncFailure::submission_store)?
                .is_some()
            {
                reconciled += 1;
            }
        }
        Ok(reconciled)
    }
}

struct PendingMessage {
    mutation: MessageProjectionMutation,
    agent_id: AgentId,
    instance_id: AgentInstanceId,
    origin_server_timestamp: UtcMillis,
    canonical_event: Vec<u8>,
    signature: DeviceSignature,
}

fn parse_pending_message(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
) -> Result<PendingMessage, MessageSyncIssueReason> {
    if event.event_id().is_none() || event.sender().is_none() || event.state_key().is_some() {
        return Err(MessageSyncIssueReason::MissingEnvelope);
    }
    let origin_server_timestamp = event
        .origin_server_timestamp()
        .and_then(|timestamp| i64::try_from(timestamp).ok())
        .and_then(|timestamp| UtcMillis::new(timestamp).ok())
        .ok_or(MessageSyncIssueReason::MissingEnvelope)?;
    validate_property_limits(event.content())?;
    let (canonical_event, signature) = canonical_and_signature(event.content())?;
    let mutation = match event.event_type().as_str() {
        PREVIEW_EVENT_TYPE => parse_preview_event(room_id, event)?,
        REVISION_EVENT_TYPE => parse_revision_event(room_id, event)?,
        _ => return Err(MessageSyncIssueReason::InvalidEnvelope),
    };
    let identity = match &mutation {
        MessageProjectionMutation::Preview(preview) => preview.actor.identity(),
        MessageProjectionMutation::Revision(revision) => revision.actor.identity(),
    };
    let agent_id = identity.agent_id();
    let instance_id = identity.agent_instance_id();
    Ok(PendingMessage {
        mutation,
        agent_id,
        instance_id,
        origin_server_timestamp,
        canonical_event,
        signature,
    })
}

fn parse_preview_event(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
) -> Result<MessageProjectionMutation, MessageSyncIssueReason> {
    let wire = serde_json::from_value::<MessagePreviewEvent>(event.content().clone())
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    validate_common(
        &wire.schema_version,
        &wire.event_type,
        PREVIEW_EVENT_TYPE,
        &wire.room_id,
        room_id,
        &wire.correlation_id,
    )?;
    let context_id = wire.id.clone();
    let actor = parse_actor(wire.actor, event)?;
    let preview = parse_preview(wire.preview)?;
    let content = parse_content(&wire.content, &preview, &context_id, event)?;
    let relation = wire
        .relation
        .map(|relation| {
            if relation.kind != "reply" {
                return Err(MessageSyncIssueReason::InvalidEnvelope);
            }
            Ok(MessageRelation::ReplyTo(parse_message_id(
                &relation.target_message_id,
            )?))
        })
        .transpose()?;
    Ok(MessageProjectionMutation::Preview(
        ProjectedMessagePreview {
            event_id: event.event_id().expect("已检查事件标识").clone(),
            transaction_id: event.transaction_id().cloned(),
            room_id: room_id.clone(),
            message_id: parse_message_id(&wire.id)?,
            created_at: parse_time(&wire.created_at)?,
            origin_server_timestamp: event.origin_server_timestamp(),
            actor,
            preview,
            content,
            relation,
        },
    ))
}

fn parse_revision_event(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
) -> Result<MessageProjectionMutation, MessageSyncIssueReason> {
    let wire = serde_json::from_value::<MessageRevisionEvent>(event.content().clone())
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    validate_common(
        &wire.schema_version,
        &wire.event_type,
        REVISION_EVENT_TYPE,
        &wire.room_id,
        room_id,
        &wire.correlation_id,
    )?;
    let context_id = wire.id.clone();
    let actor = parse_actor(wire.actor, event)?;
    let (kind, preview, content) = match wire.kind {
        WireMessageRevisionKind::Replace => {
            let preview = parse_preview(
                wire.preview
                    .ok_or(MessageSyncIssueReason::InvalidEnvelope)?,
            )?;
            let content = parse_content(
                wire.content
                    .as_ref()
                    .ok_or(MessageSyncIssueReason::InvalidEnvelope)?,
                &preview,
                &context_id,
                event,
            )?;
            (MessageRevisionKind::Replace, Some(preview), Some(content))
        }
        WireMessageRevisionKind::Redact => {
            require_absent_revision_payload(wire.preview.as_ref(), wire.content.as_ref())?;
            (MessageRevisionKind::Redact, None, None)
        }
        WireMessageRevisionKind::Moderate => {
            require_absent_revision_payload(wire.preview.as_ref(), wire.content.as_ref())?;
            (MessageRevisionKind::Moderate, None, None)
        }
    };
    Ok(MessageProjectionMutation::Revision(
        ProjectedMessageRevision {
            event_id: event.event_id().expect("已检查事件标识").clone(),
            transaction_id: event.transaction_id().cloned(),
            room_id: room_id.clone(),
            revision_id: MessageRevisionId::from_uuid(parse_v7(&wire.id)?),
            target_message_id: parse_message_id(&wire.target_message_id)?,
            created_at: parse_time(&wire.created_at)?,
            origin_server_timestamp: event.origin_server_timestamp(),
            actor,
            kind,
            preview,
            content,
        },
    ))
}

fn parse_actor(
    actor: ActorRef,
    event: &MatrixTimelineEvent,
) -> Result<ProjectedMessageActor, MessageSyncIssueReason> {
    let sender = event
        .sender()
        .ok_or(MessageSyncIssueReason::MissingEnvelope)?;
    if actor.agent.matrix_user_id != sender.as_str() {
        return Err(MessageSyncIssueReason::SenderMismatch);
    }
    let provenance = match actor.provenance {
        Provenance::Human => MessageProvenance::Human,
        Provenance::HumanConfirmedAgent => MessageProvenance::HumanConfirmedAgent,
        Provenance::AutonomousAgent => MessageProvenance::AutonomousAgent,
    };
    let mut identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(parse_v7(&actor.agent.agent_id)?),
        actor.agent.display_name,
        actor.agent.matrix_user_id,
        AgentInstanceId::from_uuid(parse_v7(&actor.instance_id)?),
    )
    .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    if let Some(avatar_url) = actor.agent.avatar_url {
        identity = identity
            .with_avatar_url(avatar_url)
            .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    }
    Ok(ProjectedMessageActor::new(identity, provenance))
}

fn parse_preview(preview: WireMessagePreview) -> Result<MessagePreview, MessageSyncIssueReason> {
    let sensitivity = match preview.sensitivity {
        WireMessageSensitivity::Normal => MessageSensitivity::Normal,
        WireMessageSensitivity::Sensitive => MessageSensitivity::Sensitive,
        WireMessageSensitivity::Restricted => MessageSensitivity::Restricted,
    };
    let language = preview
        .language
        .map(MessageLanguage::new)
        .transpose()
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    let risk_flags = preview
        .risk_flags
        .into_iter()
        .map(MessageRiskFlag::new)
        .collect::<Result<Vec<_>, _>>()
        .and_then(MessageRiskFlags::new)
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    Ok(MessagePreview::new(
        MessageTitle::new(preview.title).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?,
        MessageSummary::new(preview.summary)
            .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?,
        ContentMediaType::new(preview.content_type)
            .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?,
        language,
        sensitivity,
        risk_flags,
    ))
}

fn parse_content(
    content: &ContentRef,
    preview: &MessagePreview,
    expected_context_id: &str,
    event: &MatrixTimelineEvent,
) -> Result<MessageContentReference, MessageSyncIssueReason> {
    if content.fetch_mode != "on_demand" || content.media_type != preview.content_type().as_str() {
        return Err(MessageSyncIssueReason::InvalidEnvelope);
    }
    let reference = MessageContentReference::new(
        ContentId::from_uuid(parse_v7(&content.content_id)?),
        Sha256Digest::from_bytes(parse_sha256(&content.digest_sha256)?),
        content.size_bytes,
    )
    .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    let Some(encryption) = content.encryption.as_ref() else {
        return Ok(reference);
    };
    if !event.end_to_end_encrypted() || encryption.context_id != expected_context_id {
        return Err(MessageSyncIssueReason::InvalidEnvelope);
    }
    Ok(reference.with_client_encryption(parse_client_encryption(encryption)?))
}

fn parse_client_encryption(
    encryption: &agent_room_protocol_conformance::generated::ClientContentEncryption,
) -> Result<ClientContentEncryption, MessageSyncIssueReason> {
    let algorithm = ClientContentEncryptionAlgorithm::try_from(encryption.algorithm.as_str())
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    let context_id = ContentEncryptionContextId::from_uuid(parse_v7(&encryption.context_id)?);
    let key: [u8; CLIENT_CONTENT_KEY_BYTES] = URL_SAFE_NO_PAD
        .decode(&encryption.key_base64_url)
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?
        .try_into()
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    let nonce: [u8; CLIENT_CONTENT_NONCE_BYTES] = URL_SAFE_NO_PAD
        .decode(&encryption.nonce_base64_url)
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?
        .try_into()
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    ClientContentEncryption::new(
        algorithm,
        context_id,
        key,
        nonce,
        encryption.plaintext_size_bytes,
    )
    .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)
}

fn validate_common(
    schema_version: &str,
    event_type: &str,
    expected_event_type: &str,
    event_room_id: &str,
    room_id: &MatrixRoomId,
    correlation_id: &str,
) -> Result<(), MessageSyncIssueReason> {
    if event_room_id != room_id.as_str() {
        return Err(MessageSyncIssueReason::RoomMismatch);
    }
    if schema_version != "1.0"
        || event_type != expected_event_type
        || Uuid::parse_str(correlation_id).is_err()
    {
        return Err(MessageSyncIssueReason::InvalidEnvelope);
    }
    Ok(())
}

fn canonical_and_signature(
    content: &Value,
) -> Result<(Vec<u8>, DeviceSignature), MessageSyncIssueReason> {
    let encoded = content
        .get("signature")
        .and_then(Value::as_str)
        .ok_or(MessageSyncIssueReason::InvalidEnvelope)?;
    let signature = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| MessageSyncIssueReason::InvalidEnvelope)
        .and_then(|bytes| {
            DeviceSignature::new(bytes).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)
        })?;
    let mut unsigned = content.clone();
    unsigned
        .as_object_mut()
        .ok_or(MessageSyncIssueReason::InvalidEnvelope)?
        .remove("signature");
    let canonical_event =
        serde_jcs::to_vec(&unsigned).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    Ok((canonical_event, signature))
}

fn require_absent_revision_payload(
    preview: Option<&WireMessagePreview>,
    content: Option<&ContentRef>,
) -> Result<(), MessageSyncIssueReason> {
    if preview.is_none() && content.is_none() {
        Ok(())
    } else {
        Err(MessageSyncIssueReason::InvalidEnvelope)
    }
}

fn parse_message_id(value: &str) -> Result<MessageId, MessageSyncIssueReason> {
    Ok(MessageId::from_uuid(parse_v7(value)?))
}

fn parse_v7(value: &str) -> Result<Uuid, MessageSyncIssueReason> {
    let value = Uuid::parse_str(value).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    if value.get_version() == Some(Version::SortRand) {
        Ok(value)
    } else {
        Err(MessageSyncIssueReason::InvalidEnvelope)
    }
}

fn parse_time(value: &str) -> Result<UtcMillis, MessageSyncIssueReason> {
    let parsed =
        DateTime::parse_from_rfc3339(value).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)?;
    UtcMillis::new(parsed.timestamp_millis()).map_err(|_| MessageSyncIssueReason::InvalidEnvelope)
}

fn parse_sha256(value: &str) -> Result<[u8; 32], MessageSyncIssueReason> {
    if value.len() != 64 {
        return Err(MessageSyncIssueReason::InvalidEnvelope);
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(bytes)
}

const fn hex_nibble(value: u8) -> Result<u8, MessageSyncIssueReason> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(MessageSyncIssueReason::InvalidEnvelope),
    }
}

fn validate_property_limits(content: &Value) -> Result<(), MessageSyncIssueReason> {
    let object = bounded_object(content, 24)?;
    let actor = object
        .get("actor")
        .ok_or(MessageSyncIssueReason::InvalidEnvelope)
        .and_then(|value| bounded_object(value, 12))?;
    actor
        .get("agent")
        .ok_or(MessageSyncIssueReason::InvalidEnvelope)
        .and_then(|value| bounded_object(value, 12))?;
    if let Some(preview) = object.get("preview") {
        bounded_object(preview, 16)?;
    }
    if let Some(content) = object.get("content") {
        bounded_object(content, 16)?;
    }
    if let Some(relation) = object.get("relation") {
        bounded_object(relation, 8)?;
    }
    Ok(())
}

fn bounded_object(
    value: &Value,
    maximum_properties: usize,
) -> Result<&serde_json::Map<String, Value>, MessageSyncIssueReason> {
    let object = value
        .as_object()
        .ok_or(MessageSyncIssueReason::InvalidEnvelope)?;
    if object.len() <= maximum_properties {
        Ok(object)
    } else {
        Err(MessageSyncIssueReason::InvalidEnvelope)
    }
}

fn is_message_event(event: &MatrixTimelineEvent) -> bool {
    matches!(
        event.event_type().as_str(),
        PREVIEW_EVENT_TYPE | REVISION_EVENT_TYPE
    )
}

const fn authentication_issue(decision: MessageAuthenticationDecision) -> MessageSyncIssueReason {
    match decision {
        MessageAuthenticationDecision::Trusted
        | MessageAuthenticationDecision::TrustedHistoricalRevoked => {
            MessageSyncIssueReason::InvalidEnvelope
        }
        MessageAuthenticationDecision::UnknownInstance => MessageSyncIssueReason::UnknownInstance,
        MessageAuthenticationDecision::RevokedInstance => MessageSyncIssueReason::RevokedInstance,
        MessageAuthenticationDecision::AgentInstanceMismatch => {
            MessageSyncIssueReason::AgentInstanceMismatch
        }
        MessageAuthenticationDecision::InvalidSignature => MessageSyncIssueReason::InvalidSignature,
        MessageAuthenticationDecision::OutsideInstanceValidityWindow => {
            MessageSyncIssueReason::OutsideInstanceValidityWindow
        }
    }
}

fn issue(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
    reason: MessageSyncIssueReason,
) -> MessageSyncIssue {
    MessageSyncIssue {
        room_id: room_id.clone(),
        event_id: event.event_id().cloned(),
        reason,
    }
}

#[cfg(test)]
mod fuzz_tests {
    use agent_room_application::ports::{
        MatrixEventId, MatrixEventType, MatrixRoomId, MatrixTimelineEvent, MatrixUserId,
    };
    use proptest::prelude::*;
    use serde_json::Value;

    use super::{PREVIEW_EVENT_TYPE, REVISION_EVENT_TYPE, parse_pending_message};

    proptest! {
        #[test]
        fn 任意有界_matrix_事件只能被解析或隔离(
            content in bounded_json(),
            preview_event in any::<bool>(),
        ) {
            let room_id = MatrixRoomId::new("!fuzz:matrix.test").expect("测试房间有效");
            let event_type = if preview_event { PREVIEW_EVENT_TYPE } else { REVISION_EVENT_TYPE };
            let event = MatrixTimelineEvent::new(
                Some(MatrixEventId::new("$fuzz:matrix.test").expect("测试事件标识有效")),
                Some(MatrixUserId::new("@fuzz:matrix.test").expect("测试用户有效")),
                MatrixEventType::new(event_type).expect("测试事件类型有效"),
                None,
                None,
                Some(1_000),
                content,
            );

            if let Ok(event) = event {
                let _ = parse_pending_message(&room_id, &event);
            }
        }
    }

    fn bounded_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            (-1_000_000_i64..=1_000_000_i64).prop_map(|value| Value::Number(value.into())),
            ".{0,64}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
                prop::collection::btree_map("[a-zA-Z0-9_]{0,16}", inner, 0..8)
                    .prop_map(|values| Value::Object(values.into_iter().collect()),),
            ]
        })
    }
}
