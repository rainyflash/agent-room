use std::sync::Arc;

use agent_room_application::ports::{
    Clock, MatrixEventId, MatrixFailureKind, MatrixRoomEncryption, MatrixRoomId, PortFuture,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    handoffs::{
        ApproveHandoffRequest, HandoffConsumptionOutcome, HandoffContentFailureKind,
        HandoffDeliveryFailure, HandoffDeliveryFailureKind, HandoffDeliveryOutcome,
        HandoffDeliveryService, HandoffReceptionFailure, HandoffReceptionFailureKind,
        HandoffReceptionService, HandoffResolutionOutcome, HandoffStoreFailureKind,
        HandoffTransportFailureKind, handoff_source_matches_projection,
    },
    messages::{
        MessageBodyProtectionService, MessageContentFailureKind, MessageContentReadFailureKind,
        MessageContentSourceQuery, MessagePreviewQuery, MessagePublicationFailure,
        MessagePublicationFailureKind, MessagePublicationOutcome, MessagePublicationService,
        MessageStoreFailureKind, MessageTimelineQueryFailure, MessageTimelineQueryFailureKind,
        MessageTimelineQueryRepository, OpenMessageContentFailure, OpenMessageContentFailureKind,
        OpenMessageContentRequest, OpenMessageContentService, ProjectedMessageActor,
        ProjectedMessagePreview, ProtectMessageBodyFailure, ProtectMessageBodyFailureKind,
        ProtectMessageBodyRequest, SendMessageRequest,
    },
    presence::{PresenceProjectionFailureKind, PresenceProjectionRepository, PresenceQuery},
    status::{
        HostAgentState, StatusPublicationFailure, StatusPublicationFailureKind,
        StatusPublicationOutcome,
    },
};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcAgentSummary, IpcApproveHandoffRequest, IpcConsumedHandoff,
    IpcContentReference, IpcDeclinedHandoff, IpcErrorCategory, IpcGetPresenceRequest,
    IpcHandoffPermission, IpcHandoffPurpose, IpcHandoffRequest, IpcHandoffStatus,
    IpcHandoffSubmission, IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance,
    IpcMessageSensitivity, IpcOpenContentRequest, IpcOpenedContent, IpcPresenceSummary,
    IpcPublishStatusRequest, IpcPublishedStatus, IpcResponse, IpcSelfSummary,
    IpcSendMessageRequest, IpcSentMessage, IpcSubmissionState, IpcWorkStatus,
};
use agent_room_domain::{
    agent_status::AgentWorkStatus,
    content::{ContentByteLength, ContentMediaType},
    handoff::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffPermission,
        HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
        HandoffSourceEventId, HandoffStatus,
    },
    ids::{
        AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, MessageSubmissionId, PrincipalId,
    },
    messages::{
        MessageContentReference, MessageLanguage, MessagePreview, MessageProvenance,
        MessageRelation, MessageRiskFlag, MessageRiskFlags, MessageSensitivity, MessageSummary,
        MessageTitle,
    },
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use uuid::{Uuid, Version};

use super::{BridgeIpcDispatchFailure, BridgeStatusReader, agent_runtime_unavailable};
use crate::agent_status::AgentStatusPublicationHandle;

const MAXIMUM_HANDOFF_LIFETIME_MILLIS: i64 = 60 * 60 * 1_000;

#[derive(Clone)]
pub(crate) struct BridgeAgentRuntimeSnapshot {
    identity: BridgeAgentIdentity,
    matrix_device_id: String,
    room_id: MatrixRoomId,
    granted_capabilities: Vec<String>,
    status: Option<Arc<AgentStatusPublicationHandle>>,
    publication: Option<Arc<MessagePublicationService>>,
    handoff_delivery: Option<Arc<dyn AgentHandoffDeliveryRuntime>>,
    handoffs: Option<Arc<dyn AgentHandoffRuntime>>,
    presence: Option<Arc<dyn PresenceProjectionRepository>>,
    room_encryption: MatrixRoomEncryption,
    message_content_protection: Option<Arc<MessageBodyProtectionService>>,
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
            handoff_delivery: None,
            handoffs: None,
            presence: None,
            room_encryption: MatrixRoomEncryption::Unencrypted,
            message_content_protection: None,
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

    pub(crate) fn with_message_content_protection(
        mut self,
        protection: Arc<MessageBodyProtectionService>,
    ) -> Self {
        self.message_content_protection = Some(protection);
        self
    }

    #[must_use]
    pub(crate) const fn with_room_encryption(
        mut self,
        room_encryption: MatrixRoomEncryption,
    ) -> Self {
        self.room_encryption = room_encryption;
        self
    }

    pub(crate) fn with_handoffs(mut self, handoffs: Arc<dyn AgentHandoffRuntime>) -> Self {
        self.handoffs = Some(handoffs);
        self
    }

    pub(crate) fn with_handoff_delivery(
        mut self,
        handoff_delivery: Arc<dyn AgentHandoffDeliveryRuntime>,
    ) -> Self {
        self.handoff_delivery = Some(handoff_delivery);
        self
    }

    pub(crate) fn with_presence(mut self, presence: Arc<dyn PresenceProjectionRepository>) -> Self {
        self.presence = Some(presence);
        self
    }
}

pub(crate) trait AgentHandoffDeliveryRuntime: Send + Sync {
    fn approve_and_send(
        &self,
        request: ApproveHandoffRequest,
    ) -> PortFuture<'_, Result<HandoffDeliveryOutcome, HandoffDeliveryFailure>>;
}

impl AgentHandoffDeliveryRuntime for HandoffDeliveryService {
    fn approve_and_send(
        &self,
        request: ApproveHandoffRequest,
    ) -> PortFuture<'_, Result<HandoffDeliveryOutcome, HandoffDeliveryFailure>> {
        Box::pin(HandoffDeliveryService::approve_and_send(self, request))
    }
}

pub(crate) trait AgentHandoffRuntime: Send + Sync {
    fn inspect_pending(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<ContextHandoff, HandoffReceptionFailure>>;

    fn consume(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<HandoffConsumptionOutcome, HandoffReceptionFailure>>;

    fn decline(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<HandoffResolutionOutcome, HandoffReceptionFailure>>;
}

impl AgentHandoffRuntime for HandoffReceptionService {
    fn inspect_pending(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<ContextHandoff, HandoffReceptionFailure>> {
        Box::pin(HandoffReceptionService::inspect_pending(self, handoff_id))
    }

    fn consume(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<HandoffConsumptionOutcome, HandoffReceptionFailure>> {
        Box::pin(HandoffReceptionService::consume(self, handoff_id))
    }

    fn decline(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<HandoffResolutionOutcome, HandoffReceptionFailure>> {
        Box::pin(HandoffReceptionService::decline(self, handoff_id))
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
    clock: Arc<dyn Clock>,
}

impl AgentRuntimeIpcFacade {
    pub(super) fn new(
        status_reader: Arc<dyn BridgeStatusReader>,
        runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
        previews: Arc<dyn MessageTimelineQueryRepository>,
        content: Arc<OpenMessageContentService>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            status_reader,
            runtime_reader,
            previews,
            content,
            clock,
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

    pub(super) async fn get_presence(
        &self,
        request: IpcGetPresenceRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let room_id = requested_room(Some(request.room_id), &runtime.room_id)?;
        let agent_ids = request
            .agent_ids
            .iter()
            .map(|value| {
                parse_uuid_v7(value, "bridge.ipc.agent_id_invalid").map(AgentId::from_uuid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let query = PresenceQuery::new(room_id, agent_ids, self.clock.now())
            .map_err(|_| invalid_request("bridge.ipc.presence_targets_invalid"))?;
        let entries = runtime
            .presence
            .ok_or_else(agent_runtime_unavailable)?
            .list(&query)
            .await
            .map_err(map_presence_projection_failure)?
            .iter()
            .map(|observation| {
                let presence = observation.presence();
                IpcPresenceSummary {
                    room_id: presence.room_id().as_str().to_owned(),
                    agent: ipc_agent(presence.identity()),
                    instance_id: presence.identity().agent_instance_id().to_string(),
                    status: ipc_work_status(observation.status()),
                    observed_at_unix_ms: observation.observed_at().value(),
                    lease_expires_at_unix_ms: presence.lease_expires_at().value(),
                }
            })
            .collect();
        Ok(IpcResponse::Presence { entries })
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
                content: ipc_content(&source.content, source.preview.content_type().as_str()),
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
        let protection = runtime
            .message_content_protection
            .as_ref()
            .ok_or_else(agent_runtime_unavailable)?;
        let body = protection
            .protect(&ProtectMessageBodyRequest {
                submission_id,
                room_id: &room_id,
                room_encryption: runtime.room_encryption,
                media_type: &media_type,
                plaintext: request.body.as_bytes(),
                expires_at: None,
            })
            .map_err(map_body_protection_failure)?;
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

    pub(super) async fn approve_handoff(
        &self,
        request: IpcApproveHandoffRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let delivery = runtime
            .handoff_delivery
            .ok_or_else(agent_runtime_unavailable)?;
        let room_id = requested_room(Some(request.room_id), &runtime.room_id)?;
        let source = self
            .find_projected_source(room_id, parse_content_id(&request.source_content_id)?)
            .await?;
        let proposed_at = self.clock.now();
        let expires_at = UtcMillis::new(request.expires_at_unix_ms)
            .map_err(|_| invalid_request("bridge.ipc.handoff_expiry_invalid"))?;
        let lifetime = expires_at.value().saturating_sub(proposed_at.value());
        if !(1..=MAXIMUM_HANDOFF_LIFETIME_MILLIS).contains(&lifetime) {
            return Err(invalid_request("bridge.ipc.handoff_expiry_invalid"));
        }
        let permissions = handoff_permissions(request.permissions, source.preview.content_type())?;
        let source_identity = source.actor.identity().clone();
        let handoff = ContextHandoff::propose(ContextHandoffFields {
            id: parse_handoff_id(&request.handoff_id)?,
            requester_agent_id: runtime.identity.agent_id(),
            requester_instance_id: runtime.identity.agent_instance_id(),
            source: HandoffSource::new(
                MatrixRoomReference::new(source.room_id.as_str().to_owned())
                    .map_err(|_| internal_failure("bridge.handoff_source_invalid"))?,
                HandoffSourceEventId::new(source.event_id.as_str().to_owned())
                    .map_err(|_| internal_failure("bridge.handoff_source_invalid"))?,
                source.message_id,
                HandoffSourceActor::new(
                    source_identity.agent_id(),
                    source_identity.agent_instance_id(),
                    source.actor.provenance(),
                ),
            ),
            target_agent_id: parse_uuid_v7(
                &request.target_agent_id,
                "bridge.ipc.target_agent_id_invalid",
            )
            .map(AgentId::from_uuid)?,
            target_instance_id: parse_uuid_v7(
                &request.target_instance_id,
                "bridge.ipc.target_instance_id_invalid",
            )
            .map(AgentInstanceId::from_uuid)?,
            content: HandoffContentReference::new(
                source.content.content_id(),
                source.content.digest(),
                ContentByteLength::new(source.content.size_bytes())
                    .map_err(|_| internal_failure("bridge.handoff_source_invalid"))?,
                source.preview.content_type().clone(),
            ),
            permissions,
            purpose: handoff_purpose(request.purpose),
            risk_flags: source.preview.risk_flags().clone(),
            proposed_at,
            expires_at,
        })
        .map_err(|_| invalid_request("bridge.handoff_intent_invalid"))?;
        let approval = ApproveHandoffRequest::new(
            handoff,
            source_identity,
            parse_uuid_v7(&request.principal_id, "bridge.ipc.principal_id_invalid")
                .map(PrincipalId::from_uuid)?,
        )
        .map_err(|_| internal_failure("bridge.handoff_source_mismatch"))?;
        let outcome = delivery
            .approve_and_send(approval)
            .await
            .map_err(map_handoff_delivery_failure)?;
        Ok(IpcResponse::ApprovedHandoff {
            handoff: ipc_handoff_submission(outcome)?,
        })
    }

    pub(super) async fn consume_handoff(
        &self,
        request: IpcHandoffRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let handoffs = runtime.handoffs.ok_or_else(agent_runtime_unavailable)?;
        let handoff_id = parse_handoff_id(&request.handoff_id)?;
        let pending = handoffs
            .inspect_pending(handoff_id)
            .await
            .map_err(map_handoff_reception_failure)?;
        let media_type = pending.fields().content.media_type().as_str();
        if media_type != "application/json" && !media_type.starts_with("text/") {
            return Err(invalid_request("bridge.handoff_content_type_unsupported"));
        }
        let source = self.find_handoff_source(&pending).await?;
        let consumed = handoffs
            .consume(handoff_id)
            .await
            .map_err(map_handoff_reception_failure)?;
        let context = consumed.context();
        let handoff = context.handoff();
        if !consumption_matches_pending(handoff, &pending) {
            return Err(internal_failure("bridge.handoff_consumption_mismatch"));
        }
        let body = std::str::from_utf8(context.body().as_ref())
            .map_err(|_| internal_failure("bridge.handoff_content_encoding_invalid"))?
            .to_owned();
        let fields = handoff.fields();
        Ok(IpcResponse::ConsumedHandoff {
            handoff: IpcConsumedHandoff {
                handoff_id: fields.id.to_string(),
                source_room_id: fields.source.room_id().as_str().to_owned(),
                source_event_id: fields.source.event_id().as_str().to_owned(),
                source_actor: ipc_actor(&source.actor),
                purpose: fields.purpose.as_str().to_owned(),
                risk_flags: fields
                    .risk_flags
                    .iter()
                    .map(|flag| flag.as_str().to_owned())
                    .collect(),
                content: IpcContentReference {
                    content_id: fields.content.content_id().to_string(),
                    digest_sha256: encode_hex(fields.content.digest().as_bytes()),
                    media_type: fields.content.media_type().as_str().to_owned(),
                    size_bytes: fields.content.byte_length().value(),
                },
                body,
            },
        })
    }

    pub(super) async fn decline_handoff(
        &self,
        request: IpcHandoffRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let handoffs = runtime.handoffs.ok_or_else(agent_runtime_unavailable)?;
        let outcome = handoffs
            .decline(parse_handoff_id(&request.handoff_id)?)
            .await
            .map_err(map_handoff_reception_failure)?;
        if outcome.status() != HandoffStatus::Declined {
            return Err(internal_failure("bridge.handoff_resolution_invalid"));
        }
        Ok(IpcResponse::DeclinedHandoff {
            handoff: IpcDeclinedHandoff {
                handoff_id: outcome.handoff_id().to_string(),
                status: IpcHandoffStatus::Declined,
            },
        })
    }

    async fn find_handoff_source(
        &self,
        handoff: &ContextHandoff,
    ) -> Result<ProjectedMessagePreview, BridgeIpcDispatchFailure> {
        let fields = handoff.fields();
        let room_id = MatrixRoomId::new(fields.source.room_id().as_str().to_owned())
            .map_err(|_| internal_failure("bridge.handoff_source_invalid"))?;
        let source = self
            .find_projected_source(room_id, fields.content.content_id())
            .await?;
        if !handoff_source_matches_projection(&source, handoff) {
            return Err(internal_failure("bridge.handoff_source_mismatch"));
        }
        Ok(source)
    }

    async fn find_projected_source(
        &self,
        room_id: MatrixRoomId,
        content_id: ContentId,
    ) -> Result<ProjectedMessagePreview, BridgeIpcDispatchFailure> {
        self.previews
            .find_content_source(&MessageContentSourceQuery::new(room_id, content_id))
            .await
            .map_err(map_handoff_projection_failure)?
            .ok_or_else(|| invalid_request("bridge.handoff_source_missing"))
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

fn handoff_permissions(
    permissions: Vec<IpcHandoffPermission>,
    media_type: &ContentMediaType,
) -> Result<HandoffPermissions, BridgeIpcDispatchFailure> {
    let permissions =
        HandoffPermissions::new(permissions.into_iter().map(|permission| match permission {
            IpcHandoffPermission::ReadText => HandoffPermission::ReadText,
            IpcHandoffPermission::ReadAttachments => HandoffPermission::ReadAttachments,
            IpcHandoffPermission::IncludeMetadata => HandoffPermission::IncludeMetadata,
        }))
        .map_err(|_| invalid_request("bridge.ipc.handoff_permissions_invalid"))?;
    let text =
        media_type.as_str() == "application/json" || media_type.as_str().starts_with("text/");
    let valid = if text {
        permissions.contains(HandoffPermission::ReadText)
            && !permissions.contains(HandoffPermission::ReadAttachments)
    } else {
        permissions.contains(HandoffPermission::ReadAttachments)
            && !permissions.contains(HandoffPermission::ReadText)
    };
    if !valid {
        return Err(invalid_request("bridge.ipc.handoff_permissions_invalid"));
    }
    Ok(permissions)
}

const fn handoff_purpose(purpose: IpcHandoffPurpose) -> HandoffPurpose {
    match purpose {
        IpcHandoffPurpose::Inspect => HandoffPurpose::Inspect,
        IpcHandoffPurpose::Summarize => HandoffPurpose::Summarize,
        IpcHandoffPurpose::ReplyDraft => HandoffPurpose::ReplyDraft,
    }
}

fn ipc_handoff_submission(
    outcome: HandoffDeliveryOutcome,
) -> Result<IpcHandoffSubmission, BridgeIpcDispatchFailure> {
    match outcome {
        HandoffDeliveryOutcome::Submitted { handoff_id, reused } => {
            Ok(IpcHandoffSubmission::Submitted {
                handoff_id: handoff_id.to_string(),
                reused,
            })
        }
        HandoffDeliveryOutcome::DeliveryUncertain { handoff_id } => {
            Ok(IpcHandoffSubmission::DeliveryUncertain {
                handoff_id: handoff_id.to_string(),
            })
        }
        HandoffDeliveryOutcome::AlreadyResolved { handoff_id, status } => {
            Ok(IpcHandoffSubmission::Resolved {
                handoff_id: handoff_id.to_string(),
                status: ipc_handoff_status(status)?,
            })
        }
        HandoffDeliveryOutcome::Failed { handoff_id, code } => Ok(IpcHandoffSubmission::Failed {
            handoff_id: handoff_id.to_string(),
            code,
        }),
    }
}

const fn ipc_handoff_status(
    status: HandoffStatus,
) -> Result<IpcHandoffStatus, BridgeIpcDispatchFailure> {
    match status {
        HandoffStatus::Proposed => Err(internal_failure("bridge.handoff_state_invalid")),
        HandoffStatus::Approved => Ok(IpcHandoffStatus::Approved),
        HandoffStatus::Delivered => Ok(IpcHandoffStatus::Delivered),
        HandoffStatus::Consumed => Ok(IpcHandoffStatus::Consumed),
        HandoffStatus::Declined => Ok(IpcHandoffStatus::Declined),
        HandoffStatus::Revoked => Ok(IpcHandoffStatus::Revoked),
        HandoffStatus::Expired => Ok(IpcHandoffStatus::Expired),
        HandoffStatus::Failed => Ok(IpcHandoffStatus::Failed),
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

const fn ipc_work_status(status: AgentWorkStatus) -> IpcWorkStatus {
    match status {
        AgentWorkStatus::Offline => IpcWorkStatus::Offline,
        AgentWorkStatus::Idle => IpcWorkStatus::Idle,
        AgentWorkStatus::Working => IpcWorkStatus::Working,
        AgentWorkStatus::WaitingInput => IpcWorkStatus::WaitingInput,
        AgentWorkStatus::Blocked => IpcWorkStatus::Blocked,
        AgentWorkStatus::Completed => IpcWorkStatus::Completed,
    }
}

const fn map_presence_projection_failure(
    failure: agent_room_bridge_core::presence::PresenceProjectionFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        PresenceProjectionFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.presence_projection_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        PresenceProjectionFailureKind::Corrupt => BridgeIpcDispatchFailure::new(
            "bridge.presence_projection_corrupt",
            IpcErrorCategory::Internal,
            false,
        ),
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
        content: ipc_content(&preview.content, preview.preview.content_type().as_str()),
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

fn ipc_content(content: &MessageContentReference, media_type: &str) -> IpcContentReference {
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

fn parse_handoff_id(value: &str) -> Result<HandoffId, BridgeIpcDispatchFailure> {
    parse_uuid_v7(value, "bridge.ipc.handoff_id_invalid").map(HandoffId::from_uuid)
}

fn consumption_matches_pending(consumed: &ContextHandoff, pending: &ContextHandoff) -> bool {
    consumed.status() == HandoffStatus::Consumed
        && consumed.fields() == pending.fields()
        && consumed.approved_by_principal_id() == pending.approved_by_principal_id()
        && consumed.approved_at() == pending.approved_at()
        && consumed.delivered_at() == pending.delivered_at()
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

fn map_handoff_reception_failure(failure: HandoffReceptionFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        HandoffReceptionFailureKind::InvalidEnvelope
        | HandoffReceptionFailureKind::IntegrityMismatch => {
            internal_failure("bridge.handoff_integrity_failed")
        }
        HandoffReceptionFailureKind::WrongTarget | HandoffReceptionFailureKind::Unauthorized => {
            BridgeIpcDispatchFailure::new(
                "bridge.handoff_forbidden",
                IpcErrorCategory::Authorization,
                false,
            )
        }
        HandoffReceptionFailureKind::UntrustedSender => BridgeIpcDispatchFailure::new(
            "bridge.handoff_sender_untrusted",
            IpcErrorCategory::Authentication,
            false,
        ),
        HandoffReceptionFailureKind::AuthenticationUnavailable
        | HandoffReceptionFailureKind::AuthorizationUnavailable => BridgeIpcDispatchFailure::new(
            "bridge.handoff_authorization_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        HandoffReceptionFailureKind::Expired => BridgeIpcDispatchFailure::new(
            "bridge.handoff_expired",
            IpcErrorCategory::Conflict,
            false,
        ),
        HandoffReceptionFailureKind::Content => failure.content_failure().map_or_else(
            || internal_failure("bridge.handoff_content_internal"),
            |content| match content.kind() {
                HandoffContentFailureKind::Denied => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_content_denied",
                    IpcErrorCategory::Authorization,
                    false,
                ),
                HandoffContentFailureKind::NotFound => {
                    invalid_request("bridge.handoff_content_not_found")
                }
                HandoffContentFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_content_unavailable",
                    IpcErrorCategory::DependencyUnavailable,
                    true,
                ),
                HandoffContentFailureKind::InvalidResponse => {
                    internal_failure("bridge.handoff_content_internal")
                }
            },
        ),
        HandoffReceptionFailureKind::Store => failure.store_failure().map_or_else(
            || internal_failure("bridge.handoff_store_internal"),
            |store| match store.kind() {
                HandoffStoreFailureKind::NotFound => invalid_request("bridge.handoff_not_found"),
                HandoffStoreFailureKind::Expired => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_expired",
                    IpcErrorCategory::Conflict,
                    false,
                ),
                HandoffStoreFailureKind::Conflict | HandoffStoreFailureKind::AlreadyResolved => {
                    BridgeIpcDispatchFailure::new(
                        "bridge.handoff_already_resolved",
                        IpcErrorCategory::Conflict,
                        false,
                    )
                }
                HandoffStoreFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_store_unavailable",
                    IpcErrorCategory::DependencyUnavailable,
                    true,
                ),
                HandoffStoreFailureKind::Corrupt => {
                    internal_failure("bridge.handoff_store_corrupt")
                }
            },
        ),
    }
}

fn map_handoff_delivery_failure(failure: HandoffDeliveryFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        HandoffDeliveryFailureKind::InvalidIntent => {
            invalid_request("bridge.handoff_intent_invalid")
        }
        HandoffDeliveryFailureKind::Unauthorized => BridgeIpcDispatchFailure::new(
            "bridge.handoff_forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        HandoffDeliveryFailureKind::AuthorizationUnavailable
        | HandoffDeliveryFailureKind::DirectoryUnavailable => BridgeIpcDispatchFailure::new(
            "bridge.handoff_authorization_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        HandoffDeliveryFailureKind::SigningUnavailable => BridgeIpcDispatchFailure::new(
            "bridge.handoff_signing_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        HandoffDeliveryFailureKind::Serialization => {
            internal_failure("bridge.handoff_serialization_failed")
        }
        HandoffDeliveryFailureKind::Store => failure.store_failure().map_or_else(
            || internal_failure("bridge.handoff_store_internal"),
            map_handoff_store_failure,
        ),
        HandoffDeliveryFailureKind::Transport => failure.transport_failure().map_or_else(
            || internal_failure("bridge.handoff_transport_internal"),
            |transport| match transport.kind() {
                HandoffTransportFailureKind::Rejected => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_transport_rejected",
                    IpcErrorCategory::Conflict,
                    false,
                ),
                HandoffTransportFailureKind::Unavailable
                | HandoffTransportFailureKind::UnknownCommit => BridgeIpcDispatchFailure::new(
                    "bridge.handoff_transport_unavailable",
                    IpcErrorCategory::DependencyUnavailable,
                    true,
                ),
                HandoffTransportFailureKind::Internal => {
                    internal_failure("bridge.handoff_transport_internal")
                }
            },
        ),
    }
}

const fn map_handoff_store_failure(
    failure: agent_room_bridge_core::handoffs::HandoffStoreFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        HandoffStoreFailureKind::Conflict | HandoffStoreFailureKind::AlreadyResolved => {
            BridgeIpcDispatchFailure::new(
                "bridge.handoff_already_resolved",
                IpcErrorCategory::Conflict,
                false,
            )
        }
        HandoffStoreFailureKind::NotFound => invalid_request("bridge.handoff_not_found"),
        HandoffStoreFailureKind::Expired => BridgeIpcDispatchFailure::new(
            "bridge.handoff_expired",
            IpcErrorCategory::Conflict,
            false,
        ),
        HandoffStoreFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.handoff_store_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        HandoffStoreFailureKind::Corrupt => internal_failure("bridge.handoff_store_corrupt"),
    }
}

const fn map_handoff_projection_failure(
    failure: MessageTimelineQueryFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageTimelineQueryFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.handoff_source_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageTimelineQueryFailureKind::CursorNotFound
        | MessageTimelineQueryFailureKind::Corrupt => {
            internal_failure("bridge.handoff_source_invalid")
        }
    }
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
        OpenMessageContentFailureKind::Cryptography => failure
            .cryptography_failure()
            .map_or_else(
                || internal_failure("bridge.message_content_crypto_internal"),
                |cryptography| match cryptography.kind() {
                    agent_room_bridge_core::messages::MessageContentCryptographyFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
                        "bridge.message_content_crypto_unavailable",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                    ),
                    agent_room_bridge_core::messages::MessageContentCryptographyFailureKind::InvalidRequest
                    | agent_room_bridge_core::messages::MessageContentCryptographyFailureKind::AuthenticationFailed => {
                        internal_failure("bridge.message_content_authentication_failed")
                    }
                },
            ),
    }
}

const fn map_body_protection_failure(
    failure: ProtectMessageBodyFailure,
) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        ProtectMessageBodyFailureKind::InvalidBody => {
            invalid_request("bridge.ipc.message_body_invalid")
        }
        ProtectMessageBodyFailureKind::Cryptography => {
            match failure.cryptography_failure() {
                Some(cryptography)
                    if matches!(
                        cryptography.kind(),
                        agent_room_bridge_core::messages::MessageContentCryptographyFailureKind::Unavailable
                    ) => BridgeIpcDispatchFailure::new(
                        "bridge.message_content_crypto_unavailable",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                    ),
                _ => internal_failure("bridge.message_content_crypto_failed"),
            }
        }
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
