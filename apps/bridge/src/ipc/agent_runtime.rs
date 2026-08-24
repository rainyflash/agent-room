use std::sync::Arc;

use agent_room_application::ports::{MatrixEventId, MatrixFailureKind, MatrixRoomId};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        MessageBody, MessageContentFailureKind, MessageContentReadFailureKind, MessagePreviewQuery,
        MessagePublicationFailure, MessagePublicationFailureKind, MessagePublicationOutcome,
        MessagePublicationService, MessageStoreFailureKind, MessageTimelineQueryFailure,
        MessageTimelineQueryFailureKind, MessageTimelineQueryRepository, OpenMessageContentFailure,
        OpenMessageContentFailureKind, OpenMessageContentRequest, OpenMessageContentService,
        ProjectedMessageActor, ProjectedMessagePreview, SendMessageRequest,
    },
    status::{
        HostAgentState, StatusPublicationFailure, StatusPublicationFailureKind,
        StatusPublicationOutcome,
    },
};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcErrorCategory,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
    IpcOpenContentRequest, IpcOpenedContent, IpcPublishStatusRequest, IpcPublishedStatus,
    IpcResponse, IpcSelfSummary, IpcSendMessageRequest, IpcSentMessage, IpcSubmissionState,
    IpcWorkStatus,
};
use agent_room_domain::{
    content::{ContentEncryptionMode, ContentMediaType},
    ids::{ContentId, MessageId, MessageSubmissionId},
    messages::{
        MessageContentReference, MessageLanguage, MessagePreview, MessageProvenance,
        MessageRelation, MessageRiskFlag, MessageRiskFlags, MessageSensitivity, MessageSummary,
        MessageTitle,
    },
};
use uuid::{Uuid, Version};

use super::{BridgeIpcDispatchFailure, BridgeStatusReader, agent_runtime_unavailable};
use crate::agent_status::AgentStatusPublicationHandle;

#[derive(Clone)]
pub(crate) struct BridgeAgentRuntimeSnapshot {
    identity: BridgeAgentIdentity,
    matrix_device_id: String,
    room_id: MatrixRoomId,
    granted_capabilities: Vec<String>,
    status: Option<Arc<AgentStatusPublicationHandle>>,
    publication: Option<Arc<MessagePublicationService>>,
}

impl BridgeAgentRuntimeSnapshot {
    pub(crate) fn new(
        identity: BridgeAgentIdentity,
        matrix_device_id: impl Into<String>,
        room_id: MatrixRoomId,
        granted_capabilities: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            identity,
            matrix_device_id: matrix_device_id.into(),
            room_id,
            granted_capabilities: granted_capabilities
                .into_iter()
                .map(str::to_owned)
                .collect(),
            status: None,
            publication: None,
        }
    }

    pub(crate) fn with_status(mut self, status: Arc<AgentStatusPublicationHandle>) -> Self {
        self.status = Some(status);
        self
    }

    pub(crate) fn with_message_publication(
        mut self,
        publication: Arc<MessagePublicationService>,
    ) -> Self {
        self.publication = Some(publication);
        self
    }
}

pub(crate) trait BridgeAgentRuntimeReader: Send + Sync {
    fn read_agent_runtime(&self) -> Option<BridgeAgentRuntimeSnapshot>;
}

pub(super) struct AgentRuntimeIpcFacade {
    status_reader: Arc<dyn BridgeStatusReader>,
    runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
    previews: Arc<dyn MessageTimelineQueryRepository>,
    content: Arc<OpenMessageContentService>,
}

impl AgentRuntimeIpcFacade {
    pub(super) fn new(
        status_reader: Arc<dyn BridgeStatusReader>,
        runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
        previews: Arc<dyn MessageTimelineQueryRepository>,
        content: Arc<OpenMessageContentService>,
    ) -> Self {
        Self {
            status_reader,
            runtime_reader,
            previews,
            content,
        }
    }

    pub(super) fn get_self(&self) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        Ok(IpcResponse::SelfSummary {
            summary: IpcSelfSummary {
                agent: ipc_agent(&runtime.identity),
                instance_id: runtime.identity.agent_instance_id().to_string(),
                matrix_device_id: runtime.matrix_device_id,
                connection_state: self.status_reader.read_status().state,
                granted_capabilities: runtime.granted_capabilities,
            },
        })
    }

    pub(super) async fn list_previews(
        &self,
        request: IpcListPreviewsRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let room_id = requested_room(request.room_id, &runtime.room_id)?;
        let cursor = request
            .before_event_id
            .map(MatrixEventId::new)
            .transpose()
            .map_err(|_| invalid_request("bridge.ipc.event_id_invalid"))?;
        let query = MessagePreviewQuery::new(room_id, cursor, request.limit)
            .map_err(|_| invalid_request("bridge.ipc.preview_limit_invalid"))?;
        let page = self
            .previews
            .list_previews(&query)
            .await
            .map_err(map_preview_query_failure)?;
        Ok(IpcResponse::MessagePreviews {
            previews: page.previews().iter().map(ipc_preview).collect(),
            next_cursor: page.next_cursor().map(|cursor| cursor.as_str().to_owned()),
        })
    }

    pub(super) async fn publish_status(
        &self,
        request: IpcPublishStatusRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let room_id = requested_room(Some(request.room_id), &runtime.room_id)?;
        let status = runtime.status.ok_or_else(agent_runtime_unavailable)?;
        let outcome = status
            .publish(host_status(request.status))
            .await
            .map_err(map_status_publication_failure)?;
        let lease_expires_at_unix_ms = match outcome {
            StatusPublicationOutcome::Published { lease, .. } => lease.expires_at().value(),
            StatusPublicationOutcome::NotDue {
                lease_expires_at, ..
            } => lease_expires_at.value(),
        };
        Ok(IpcResponse::PublishedStatus {
            publication: IpcPublishedStatus {
                room_id: room_id.as_str().to_owned(),
                status: request.status,
                lease_expires_at_unix_ms,
            },
        })
    }

    pub(super) async fn open_content(
        &self,
        request: IpcOpenContentRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let content_id = parse_content_id(&request.content_id)?;
        let opened = self
            .content
            .open(&OpenMessageContentRequest::new(runtime.room_id, content_id))
            .await
            .map_err(map_content_open_failure)?;
        let source = opened.source();
        Ok(IpcResponse::OpenedContent {
            content: IpcOpenedContent {
                content: ipc_content(source.content, source.preview.content_type().as_str()),
                source_room_id: source.room_id.as_str().to_owned(),
                source_event_id: source.event_id.as_str().to_owned(),
                source_actor: ipc_actor(&source.actor),
                risk_flags: source
                    .preview
                    .risk_flags()
                    .iter()
                    .map(|flag| flag.as_str().to_owned())
                    .collect(),
                body: opened.body().to_owned(),
            },
        })
    }

    pub(super) async fn send_message(
        &self,
        request: IpcSendMessageRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let room_id = requested_room(Some(request.room_id), &runtime.room_id)?;
        let publication = runtime.publication.ok_or_else(agent_runtime_unavailable)?;
        let submission_id = request
            .submission_id
            .as_deref()
            .map(parse_submission_id)
            .transpose()?
            .unwrap_or_else(|| MessageSubmissionId::from_uuid(Uuid::now_v7()));
        let media_type = ContentMediaType::new(request.media_type)
            .map_err(|_| invalid_request("bridge.ipc.media_type_invalid"))?;
        let language = request
            .language
            .map(MessageLanguage::new)
            .transpose()
            .map_err(|_| invalid_request("bridge.ipc.language_invalid"))?;
        let risk_flags = request
            .risk_flags
            .into_iter()
            .map(MessageRiskFlag::new)
            .collect::<Result<Vec<_>, _>>()
            .and_then(MessageRiskFlags::new)
            .map_err(|_| invalid_request("bridge.ipc.risk_flags_invalid"))?;
        let preview = MessagePreview::new(
            MessageTitle::new(request.title)
                .map_err(|_| invalid_request("bridge.ipc.message_title_invalid"))?,
            MessageSummary::new(request.summary)
                .map_err(|_| invalid_request("bridge.ipc.message_summary_invalid"))?,
            media_type.clone(),
            language,
            message_sensitivity(request.sensitivity),
            risk_flags,
        );
        let body = MessageBody::new(
            request.body.into_bytes(),
            media_type,
            ContentEncryptionMode::ServerSide,
            None,
        )
        .map_err(|_| invalid_request("bridge.ipc.message_body_invalid"))?;
        let relation = request
            .reply_to_message_id
            .as_deref()
            .map(parse_message_id)
            .transpose()?
            .map(MessageRelation::ReplyTo);
        let intent = SendMessageRequest::new(
            submission_id,
            room_id,
            preview,
            body,
            message_provenance(request.provenance),
            relation,
        )
        .map_err(|_| invalid_request("bridge.ipc.message_intent_invalid"))?;

        let outcome = match publication.send(&intent).await {
            Ok(outcome) => ipc_publication_outcome(outcome),
            Err(failure) if content_commit_is_unknown(failure) => IpcSentMessage {
                submission_id: submission_id.to_string(),
                state: IpcSubmissionState::UnknownCommit,
                event_id: None,
            },
            Err(failure) => return Err(map_message_publication_failure(failure)),
        };
        Ok(IpcResponse::SentMessage { message: outcome })
    }

    fn runtime_snapshot(&self) -> Result<BridgeAgentRuntimeSnapshot, BridgeIpcDispatchFailure> {
        self.runtime_reader
            .read_agent_runtime()
            .ok_or_else(agent_runtime_unavailable)
    }
}

const fn host_status(status: IpcWorkStatus) -> HostAgentState {
    match status {
        IpcWorkStatus::Offline => HostAgentState::Disconnected,
        IpcWorkStatus::Idle => HostAgentState::Available,
        IpcWorkStatus::Working => HostAgentState::Running,
        IpcWorkStatus::WaitingInput => HostAgentState::AwaitingInput,
        IpcWorkStatus::Blocked => HostAgentState::Blocked,
        IpcWorkStatus::Completed => HostAgentState::Succeeded,
    }
}

fn requested_room(
    requested: Option<String>,
    active: &MatrixRoomId,
) -> Result<MatrixRoomId, BridgeIpcDispatchFailure> {
    let requested = requested
        .map(MatrixRoomId::new)
        .transpose()
        .map_err(|_| invalid_request("bridge.ipc.room_id_invalid"))?
        .unwrap_or_else(|| active.clone());
    if requested != *active {
        return Err(BridgeIpcDispatchFailure::new(
            "bridge.room_not_joined",
            IpcErrorCategory::Authorization,
            false,
        ));
    }
    Ok(requested)
}

fn ipc_agent(identity: &BridgeAgentIdentity) -> IpcAgentSummary {
    IpcAgentSummary {
        agent_id: identity.agent_id().to_string(),
        display_name: identity.display_name().to_owned(),
        matrix_user_id: identity.matrix_user_id().as_str().to_owned(),
        avatar_url: identity.avatar_url().map(str::to_owned),
    }
}

fn ipc_actor(actor: &ProjectedMessageActor) -> IpcActorSummary {
    IpcActorSummary {
        agent: ipc_agent(actor.identity()),
        instance_id: actor.identity().agent_instance_id().to_string(),
        provenance: ipc_provenance(actor.provenance()),
    }
}

fn ipc_preview(preview: &ProjectedMessagePreview) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        message_id: preview.message_id.to_string(),
        event_id: preview.event_id.as_str().to_owned(),
        room_id: preview.room_id.as_str().to_owned(),
        actor: ipc_actor(&preview.actor),
        created_at_unix_ms: preview.created_at.value(),
        title: preview.preview.title().as_str().to_owned(),
        summary: preview.preview.summary().as_str().to_owned(),
        content: ipc_content(preview.content, preview.preview.content_type().as_str()),
        language: preview
            .preview
            .language()
            .map(|value| value.as_str().to_owned()),
        sensitivity: ipc_sensitivity(preview.preview.sensitivity()),
        risk_flags: preview
            .preview
            .risk_flags()
            .iter()
            .map(|flag| flag.as_str().to_owned())
            .collect(),
    }
}

fn ipc_content(content: MessageContentReference, media_type: &str) -> IpcContentReference {
    IpcContentReference {
        content_id: content.content_id().to_string(),
        digest_sha256: encode_hex(content.digest().as_bytes()),
        media_type: media_type.to_owned(),
        size_bytes: content.size_bytes(),
    }
}

fn parse_content_id(value: &str) -> Result<ContentId, BridgeIpcDispatchFailure> {
    let id =
        Uuid::parse_str(value).map_err(|_| invalid_request("bridge.ipc.content_id_invalid"))?;
    if id.get_version() != Some(Version::SortRand) || id.to_string() != value {
        return Err(invalid_request("bridge.ipc.content_id_invalid"));
    }
    Ok(ContentId::from_uuid(id))
}

fn parse_submission_id(value: &str) -> Result<MessageSubmissionId, BridgeIpcDispatchFailure> {
    parse_uuid_v7(value, "bridge.ipc.submission_id_invalid").map(MessageSubmissionId::from_uuid)
}

fn parse_message_id(value: &str) -> Result<MessageId, BridgeIpcDispatchFailure> {
    parse_uuid_v7(value, "bridge.ipc.message_id_invalid").map(MessageId::from_uuid)
}

fn parse_uuid_v7(value: &str, code: &'static str) -> Result<Uuid, BridgeIpcDispatchFailure> {
    let id = Uuid::parse_str(value).map_err(|_| invalid_request(code))?;
    if id.get_version() != Some(Version::SortRand) || id.to_string() != value {
        return Err(invalid_request(code));
    }
    Ok(id)
}

const fn message_provenance(value: IpcMessageProvenance) -> MessageProvenance {
    match value {
        IpcMessageProvenance::Human => MessageProvenance::Human,
        IpcMessageProvenance::HumanConfirmedAgent => MessageProvenance::HumanConfirmedAgent,
        IpcMessageProvenance::AutonomousAgent => MessageProvenance::AutonomousAgent,
    }
}

const fn message_sensitivity(value: IpcMessageSensitivity) -> MessageSensitivity {
    match value {
        IpcMessageSensitivity::Normal => MessageSensitivity::Normal,
        IpcMessageSensitivity::Sensitive => MessageSensitivity::Sensitive,
        IpcMessageSensitivity::Restricted => MessageSensitivity::Restricted,
    }
}

fn ipc_publication_outcome(outcome: MessagePublicationOutcome) -> IpcSentMessage {
    match outcome {
        MessagePublicationOutcome::Published {
            submission_id,
            event_id,
            ..
        } => IpcSentMessage {
            submission_id: submission_id.to_string(),
            state: IpcSubmissionState::Submitted,
            event_id: Some(event_id.as_str().to_owned()),
        },
        MessagePublicationOutcome::PendingReconciliation { submission_id, .. } => IpcSentMessage {
            submission_id: submission_id.to_string(),
            state: IpcSubmissionState::UnknownCommit,
            event_id: None,
        },
        MessagePublicationOutcome::AcceptedBindingPending {
            submission_id,
            event_id,
        } => IpcSentMessage {
            submission_id: submission_id.to_string(),
            state: IpcSubmissionState::BindingPending,
            event_id: Some(event_id.as_str().to_owned()),
        },
    }
}

fn content_commit_is_unknown(failure: MessagePublicationFailure) -> bool {
    failure.kind() == MessagePublicationFailureKind::Content
        && failure
            .content_failure()
            .is_some_and(|failure| failure.kind() == MessageContentFailureKind::UnknownCommit)
}

const fn ipc_provenance(value: MessageProvenance) -> IpcMessageProvenance {
    match value {
        MessageProvenance::Human => IpcMessageProvenance::Human,
        MessageProvenance::HumanConfirmedAgent => IpcMessageProvenance::HumanConfirmedAgent,
        MessageProvenance::AutonomousAgent => IpcMessageProvenance::AutonomousAgent,
    }
}

const fn ipc_sensitivity(value: MessageSensitivity) -> IpcMessageSensitivity {
    match value {
        MessageSensitivity::Normal => IpcMessageSensitivity::Normal,
        MessageSensitivity::Sensitive => IpcMessageSensitivity::Sensitive,
        MessageSensitivity::Restricted => IpcMessageSensitivity::Restricted,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn map_preview_query_failure(failure: MessageTimelineQueryFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageTimelineQueryFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_projection_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageTimelineQueryFailureKind::CursorNotFound => {
            invalid_request("bridge.preview_cursor_not_found")
        }
        MessageTimelineQueryFailureKind::Corrupt => BridgeIpcDispatchFailure::new(
            "bridge.message_projection_corrupt",
            IpcErrorCategory::Internal,
            false,
        ),
    }
}

fn map_status_publication_failure(failure: StatusPublicationFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        StatusPublicationFailureKind::InvalidIntent => {
            invalid_request("bridge.status_intent_invalid")
        }
        StatusPublicationFailureKind::SigningUnavailable => BridgeIpcDispatchFailure::new(
            "bridge.status_signing_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        StatusPublicationFailureKind::Matrix => {
            let retryable = failure.matrix_failure().is_some_and(|failure| {
                matches!(
                    failure.kind(),
                    MatrixFailureKind::Conflict
                        | MatrixFailureKind::RateLimited
                        | MatrixFailureKind::Timeout
                        | MatrixFailureKind::DependencyUnavailable
                        | MatrixFailureKind::UnknownCommit
                )
            });
            BridgeIpcDispatchFailure::new(
                "bridge.status_matrix_failed",
                IpcErrorCategory::DependencyUnavailable,
                retryable,
            )
        }
        StatusPublicationFailureKind::InvalidConfiguration
        | StatusPublicationFailureKind::InvalidIdentity
        | StatusPublicationFailureKind::InvalidIdentifier
        | StatusPublicationFailureKind::Serialization => BridgeIpcDispatchFailure::new(
            "bridge.status_publication_internal",
            IpcErrorCategory::Internal,
            false,
        ),
    }
}

fn map_message_publication_failure(failure: MessagePublicationFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessagePublicationFailureKind::InvalidIntent => {
            invalid_request("bridge.message_intent_invalid")
        }
        MessagePublicationFailureKind::SigningUnavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_signing_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessagePublicationFailureKind::Serialization => {
            internal_failure("bridge.message_serialization_failed")
        }
        MessagePublicationFailureKind::Store => failure.store_failure().map_or_else(
            || internal_failure("bridge.message_store_internal"),
            map_message_store_failure,
        ),
        MessagePublicationFailureKind::Content => failure.content_failure().map_or_else(
            || internal_failure("bridge.message_content_internal"),
            map_message_content_failure,
        ),
        MessagePublicationFailureKind::Matrix => failure.matrix_failure().map_or_else(
            || internal_failure("bridge.message_matrix_internal"),
            map_message_matrix_failure,
        ),
    }
}

const fn map_message_store_failure(
    failure: agent_room_bridge_core::messages::MessageStoreFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageStoreFailureKind::Conflict => BridgeIpcDispatchFailure::new(
            "bridge.message_submission_conflict",
            IpcErrorCategory::Conflict,
            false,
        ),
        MessageStoreFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_store_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageStoreFailureKind::NotFound | MessageStoreFailureKind::Corrupt => {
            internal_failure("bridge.message_store_internal")
        }
    }
}

const fn map_message_content_failure(
    failure: agent_room_bridge_core::messages::MessageContentFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageContentFailureKind::InvalidRequest => {
            invalid_request("bridge.message_content_request_invalid")
        }
        MessageContentFailureKind::Denied => BridgeIpcDispatchFailure::new(
            "bridge.message_content_denied",
            IpcErrorCategory::Authorization,
            false,
        ),
        MessageContentFailureKind::Conflict => BridgeIpcDispatchFailure::new(
            "bridge.message_content_conflict",
            IpcErrorCategory::Conflict,
            false,
        ),
        MessageContentFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_content_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageContentFailureKind::UnknownCommit => BridgeIpcDispatchFailure::new(
            "bridge.message_content_commit_unknown",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageContentFailureKind::Internal => internal_failure("bridge.message_content_internal"),
    }
}

const fn map_message_matrix_failure(
    failure: agent_room_application::ports::MatrixFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MatrixFailureKind::Unauthenticated | MatrixFailureKind::AuthenticationRejected => {
            BridgeIpcDispatchFailure::new(
                "bridge.message_matrix_authentication_failed",
                IpcErrorCategory::Authentication,
                false,
            )
        }
        MatrixFailureKind::Forbidden => BridgeIpcDispatchFailure::new(
            "bridge.message_matrix_forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        MatrixFailureKind::NotFound => invalid_request("bridge.message_matrix_room_not_found"),
        MatrixFailureKind::Conflict => BridgeIpcDispatchFailure::new(
            "bridge.message_matrix_conflict",
            IpcErrorCategory::Conflict,
            false,
        ),
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => BridgeIpcDispatchFailure::new(
            "bridge.message_matrix_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MatrixFailureKind::UnsupportedVersion => BridgeIpcDispatchFailure::new(
            "bridge.message_matrix_version_unsupported",
            IpcErrorCategory::IncompatibleVersion,
            false,
        ),
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken => internal_failure("bridge.message_matrix_internal"),
    }
}

fn map_content_open_failure(failure: OpenMessageContentFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        OpenMessageContentFailureKind::Projection => failure.projection_failure().map_or_else(
            || internal_failure("bridge.message_content_internal"),
            map_preview_query_failure,
        ),
        OpenMessageContentFailureKind::NotFound => {
            invalid_request("bridge.message_content_not_found")
        }
        OpenMessageContentFailureKind::UnsupportedMediaType => {
            invalid_request("bridge.message_content_type_unsupported")
        }
        OpenMessageContentFailureKind::TooLarge => {
            invalid_request("bridge.message_content_too_large")
        }
        OpenMessageContentFailureKind::InvalidEncoding => {
            invalid_request("bridge.message_content_encoding_invalid")
        }
        OpenMessageContentFailureKind::IntegrityMismatch => {
            internal_failure("bridge.message_content_integrity_failed")
        }
        OpenMessageContentFailureKind::Content => failure.content_failure().map_or_else(
            || internal_failure("bridge.message_content_internal"),
            map_content_read_failure,
        ),
    }
}

const fn map_content_read_failure(
    failure: agent_room_bridge_core::messages::MessageContentReadFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageContentReadFailureKind::InvalidRequest => {
            invalid_request("bridge.message_content_request_invalid")
        }
        MessageContentReadFailureKind::NotFound => {
            invalid_request("bridge.message_content_not_found")
        }
        MessageContentReadFailureKind::Denied => BridgeIpcDispatchFailure::new(
            "bridge.message_content_denied",
            IpcErrorCategory::Authorization,
            false,
        ),
        MessageContentReadFailureKind::RateLimited => BridgeIpcDispatchFailure::new(
            "bridge.message_content_rate_limited",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageContentReadFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_content_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageContentReadFailureKind::InvalidResponse
        | MessageContentReadFailureKind::Internal => {
            internal_failure("bridge.message_content_internal")
        }
    }
}

const fn internal_failure(code: &'static str) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(code, IpcErrorCategory::Internal, false)
}

const fn invalid_request(code: &'static str) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(code, IpcErrorCategory::Validation, false)
}
