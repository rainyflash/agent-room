use std::sync::Arc;

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixEvent, MatrixEventId, MatrixFailure, MatrixFailureKind,
    MatrixGateway, MatrixResult, MatrixRoomId, MatrixTransactionId, PortFuture,
};
use agent_room_domain::ids::{ContentUploadRequestId, MessageSubmissionId};

use crate::{agent_identity::BridgeAgentIdentity, ports::DeviceSigningIdentity};

use super::{
    EditMessageRequest, MessageContentBindRequest, MessageContentFailure,
    MessageContentFailureKind, MessageContentGateway, MessageContentRecord,
    MessageContentRedactRequest, MessageContentUploadRequest, MessageEventPublisher,
    MessageStoreFailure, MessageSubmissionClaim, MessageSubmissionFingerprint,
    MessageSubmissionKind, MessageSubmissionRecord, MessageSubmissionRepository,
    MessageSubmissionState, RedactMessageRequest, SendMessageRequest,
    wire::{
        MessageWireFailure, edit_event, edit_fingerprint, preview_event, preview_fingerprint,
        preview_transaction_id, redact_event, redact_fingerprint, revision_transaction_id,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePublicationFailureKind {
    InvalidIntent,
    SigningUnavailable,
    Serialization,
    Store,
    Content,
    Matrix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessagePublicationFailure {
    kind: MessagePublicationFailureKind,
    store: Option<MessageStoreFailure>,
    content: Option<MessageContentFailure>,
    matrix: Option<MatrixFailure>,
}

impl MessagePublicationFailure {
    const fn simple(kind: MessagePublicationFailureKind) -> Self {
        Self {
            kind,
            store: None,
            content: None,
            matrix: None,
        }
    }

    const fn store(failure: MessageStoreFailure) -> Self {
        Self {
            kind: MessagePublicationFailureKind::Store,
            store: Some(failure),
            content: None,
            matrix: None,
        }
    }

    const fn content(failure: MessageContentFailure) -> Self {
        Self {
            kind: MessagePublicationFailureKind::Content,
            store: None,
            content: Some(failure),
            matrix: None,
        }
    }

    const fn matrix(failure: MatrixFailure) -> Self {
        Self {
            kind: MessagePublicationFailureKind::Matrix,
            store: None,
            content: None,
            matrix: Some(failure),
        }
    }

    pub const fn kind(self) -> MessagePublicationFailureKind {
        self.kind
    }

    pub const fn store_failure(self) -> Option<MessageStoreFailure> {
        self.store
    }

    pub const fn content_failure(self) -> Option<MessageContentFailure> {
        self.content
    }

    pub const fn matrix_failure(self) -> Option<MatrixFailure> {
        self.matrix
    }
}

pub type MessagePublicationResult<T> = Result<T, MessagePublicationFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePublicationOutcome {
    Published {
        submission_id: MessageSubmissionId,
        event_id: MatrixEventId,
        reused: bool,
    },
    PendingReconciliation {
        submission_id: MessageSubmissionId,
        transaction_id: MatrixTransactionId,
    },
    AcceptedBindingPending {
        submission_id: MessageSubmissionId,
        event_id: MatrixEventId,
    },
}

pub struct MessagePublicationDependencies {
    pub identity: BridgeAgentIdentity,
    pub signer: Arc<dyn DeviceSigningIdentity>,
    pub publisher: Arc<dyn MessageEventPublisher>,
    pub content: Arc<dyn MessageContentGateway>,
    pub submissions: Arc<dyn MessageSubmissionRepository>,
}

pub struct MessagePublicationService {
    identity: BridgeAgentIdentity,
    signer: Arc<dyn DeviceSigningIdentity>,
    publisher: Arc<dyn MessageEventPublisher>,
    content: Arc<dyn MessageContentGateway>,
    submissions: Arc<dyn MessageSubmissionRepository>,
}

impl MessagePublicationService {
    pub fn new(dependencies: MessagePublicationDependencies) -> Self {
        Self {
            identity: dependencies.identity,
            signer: dependencies.signer,
            publisher: dependencies.publisher,
            content: dependencies.content,
            submissions: dependencies.submissions,
        }
    }

    /// 幂等上传正文、签名并发布消息预览，最后绑定内容访问策略。
    ///
    /// # Errors
    ///
    /// 意图冲突、内容服务、签名、持久化或 Matrix 失败时返回阶段化错误。
    pub async fn send(
        &self,
        request: &SendMessageRequest,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let fingerprint = preview_fingerprint(&self.identity, request).map_err(map_wire_failure)?;
        let transaction_id =
            preview_transaction_id(request.submission_id()).map_err(map_wire_failure)?;
        let record = self
            .claim(
                request.submission_id(),
                MessageSubmissionKind::Preview,
                fingerprint,
                transaction_id.clone(),
            )
            .await?;
        if let Some(outcome) = completed_outcome(&record) {
            return Ok(outcome);
        }
        let content = self
            .upload(request.submission_id(), request.room_id(), request.body())
            .await?;
        if record.state == MessageSubmissionState::Accepted {
            return self.bind_accepted(record, content, request.room_id()).await;
        }
        let event = preview_event(
            &self.identity,
            self.signer.as_ref(),
            request,
            transaction_id,
            &content,
        )
        .map_err(map_wire_failure)?;
        self.publish_with_content(request.submission_id(), request.room_id(), event, content)
            .await
    }

    /// 发布引用原消息的替换修订，并把新正文绑定到修订事件。
    ///
    /// # Errors
    ///
    /// 意图冲突、内容服务、签名、持久化或 Matrix 失败时返回阶段化错误。
    pub async fn edit(
        &self,
        request: &EditMessageRequest,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let fingerprint = edit_fingerprint(&self.identity, request).map_err(map_wire_failure)?;
        let transaction_id =
            revision_transaction_id(request.submission_id()).map_err(map_wire_failure)?;
        let record = self
            .claim(
                request.submission_id(),
                MessageSubmissionKind::Replace,
                fingerprint,
                transaction_id.clone(),
            )
            .await?;
        if let Some(outcome) = completed_outcome(&record) {
            return Ok(outcome);
        }
        let content = self
            .upload(request.submission_id(), request.room_id(), request.body())
            .await?;
        if record.state == MessageSubmissionState::Accepted {
            return self.bind_accepted(record, content, request.room_id()).await;
        }
        let event = edit_event(
            &self.identity,
            self.signer.as_ref(),
            request,
            transaction_id,
            &content,
        )
        .map_err(map_wire_failure)?;
        self.publish_with_content(request.submission_id(), request.room_id(), event, content)
            .await
    }

    /// 先撤销正文读取权，再发布显式撤回修订。
    ///
    /// # Errors
    ///
    /// 撤销、签名、持久化或 Matrix 发布失败时返回阶段化错误。正文撤销成功后不会因事件失败而恢复。
    pub async fn redact(
        &self,
        request: &RedactMessageRequest,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let fingerprint = redact_fingerprint(&self.identity, request).map_err(map_wire_failure)?;
        let transaction_id =
            revision_transaction_id(request.submission_id()).map_err(map_wire_failure)?;
        let record = self
            .claim(
                request.submission_id(),
                MessageSubmissionKind::Redact,
                fingerprint,
                transaction_id.clone(),
            )
            .await?;
        if let Some(outcome) = completed_outcome(&record) {
            return Ok(outcome);
        }
        self.content
            .redact(&MessageContentRedactRequest {
                content_id: request.target_content_id(),
            })
            .await
            .map_err(MessagePublicationFailure::content)?;
        if record.state == MessageSubmissionState::Accepted {
            return self.complete_redaction(record).await;
        }
        let event = redact_event(
            &self.identity,
            self.signer.as_ref(),
            request,
            transaction_id,
        )
        .map_err(map_wire_failure)?;
        let accepted = self
            .publish(request.submission_id(), request.room_id(), event)
            .await?;
        let Some(event_id) = accepted else {
            return Ok(MessagePublicationOutcome::PendingReconciliation {
                submission_id: request.submission_id(),
                transaction_id: record.transaction_id,
            });
        };
        self.submissions
            .mark_bound(request.submission_id())
            .await
            .map_err(MessagePublicationFailure::store)?;
        Ok(MessagePublicationOutcome::Published {
            submission_id: request.submission_id(),
            event_id,
            reused: false,
        })
    }

    async fn claim(
        &self,
        submission_id: MessageSubmissionId,
        kind: MessageSubmissionKind,
        fingerprint: MessageSubmissionFingerprint,
        transaction_id: MatrixTransactionId,
    ) -> MessagePublicationResult<MessageSubmissionRecord> {
        self.submissions
            .claim(&MessageSubmissionClaim {
                submission_id,
                kind,
                fingerprint,
                transaction_id,
            })
            .await
            .map(|outcome| outcome.record().clone())
            .map_err(MessagePublicationFailure::store)
    }

    async fn upload(
        &self,
        submission_id: MessageSubmissionId,
        room_id: &MatrixRoomId,
        body: &super::MessageBody,
    ) -> MessagePublicationResult<MessageContentRecord> {
        let content = self
            .content
            .upload(&MessageContentUploadRequest {
                request_id: ContentUploadRequestId::from_uuid(submission_id.as_uuid()),
                room_id: room_id.clone(),
                digest: body.digest(),
                byte_length: body.byte_length(),
                media_type: body.media_type().clone(),
                encryption_mode: body.encryption_mode(),
                expires_at: body.expires_at(),
                body: Arc::clone(body.bytes()),
            })
            .await
            .map_err(MessagePublicationFailure::content)?;
        if content.digest != body.digest()
            || content.byte_length != body.byte_length()
            || content.media_type != *body.media_type()
        {
            return Err(MessagePublicationFailure::content(
                MessageContentFailure::new(MessageContentFailureKind::Internal),
            ));
        }
        Ok(content)
    }

    async fn publish_with_content(
        &self,
        submission_id: MessageSubmissionId,
        room_id: &MatrixRoomId,
        event: MatrixEvent,
        content: MessageContentRecord,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let transaction_id = event.transaction_id().clone();
        let accepted = self.publish(submission_id, room_id, event).await?;
        let Some(event_id) = accepted else {
            return Ok(MessagePublicationOutcome::PendingReconciliation {
                submission_id,
                transaction_id,
            });
        };
        self.bind(submission_id, content.content_id, room_id, event_id, false)
            .await
    }

    async fn publish(
        &self,
        submission_id: MessageSubmissionId,
        room_id: &MatrixRoomId,
        event: MatrixEvent,
    ) -> MessagePublicationResult<Option<MatrixEventId>> {
        match self.publisher.publish(room_id, &event).await {
            Ok(accepted) => {
                let event_id = accepted.event_id().clone();
                self.submissions
                    .mark_accepted(submission_id, &event_id)
                    .await
                    .map_err(MessagePublicationFailure::store)?;
                Ok(Some(event_id))
            }
            Err(failure) if failure.kind() == MatrixFailureKind::UnknownCommit => {
                self.submissions
                    .mark_submit_unknown(submission_id)
                    .await
                    .map_err(MessagePublicationFailure::store)?;
                Ok(None)
            }
            Err(failure) => Err(MessagePublicationFailure::matrix(failure)),
        }
    }

    async fn bind_accepted(
        &self,
        record: MessageSubmissionRecord,
        content: MessageContentRecord,
        room_id: &MatrixRoomId,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let event_id = record.event_id.ok_or_else(|| {
            MessagePublicationFailure::simple(MessagePublicationFailureKind::InvalidIntent)
        })?;
        self.bind(
            record.submission_id,
            content.content_id,
            room_id,
            event_id,
            true,
        )
        .await
    }

    async fn complete_redaction(
        &self,
        record: MessageSubmissionRecord,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        let event_id = record.event_id.ok_or_else(|| {
            MessagePublicationFailure::simple(MessagePublicationFailureKind::InvalidIntent)
        })?;
        self.submissions
            .mark_bound(record.submission_id)
            .await
            .map_err(MessagePublicationFailure::store)?;
        Ok(MessagePublicationOutcome::Published {
            submission_id: record.submission_id,
            event_id,
            reused: true,
        })
    }

    async fn bind(
        &self,
        submission_id: MessageSubmissionId,
        content_id: agent_room_domain::ids::ContentId,
        room_id: &MatrixRoomId,
        event_id: MatrixEventId,
        reused: bool,
    ) -> MessagePublicationResult<MessagePublicationOutcome> {
        match self
            .content
            .bind(&MessageContentBindRequest {
                content_id,
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            })
            .await
        {
            Ok(()) => {
                self.submissions
                    .mark_bound(submission_id)
                    .await
                    .map_err(MessagePublicationFailure::store)?;
                Ok(MessagePublicationOutcome::Published {
                    submission_id,
                    event_id,
                    reused,
                })
            }
            Err(failure)
                if matches!(
                    failure.kind(),
                    MessageContentFailureKind::Unavailable
                        | MessageContentFailureKind::UnknownCommit
                ) =>
            {
                Ok(MessagePublicationOutcome::AcceptedBindingPending {
                    submission_id,
                    event_id,
                })
            }
            Err(failure) => Err(MessagePublicationFailure::content(failure)),
        }
    }
}

pub struct MatrixMessageEventPublisher {
    gateway: Arc<dyn MatrixGateway>,
}

impl MatrixMessageEventPublisher {
    pub fn new(gateway: Arc<dyn MatrixGateway>) -> Self {
        Self { gateway }
    }
}

impl MessageEventPublisher for MatrixMessageEventPublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        self.gateway.send_event(room_id, event)
    }
}

fn completed_outcome(record: &MessageSubmissionRecord) -> Option<MessagePublicationOutcome> {
    if record.state != MessageSubmissionState::Bound {
        return None;
    }
    record
        .event_id
        .clone()
        .map(|event_id| MessagePublicationOutcome::Published {
            submission_id: record.submission_id,
            event_id,
            reused: true,
        })
}

const fn map_wire_failure(failure: MessageWireFailure) -> MessagePublicationFailure {
    match failure {
        MessageWireFailure::InvalidIdentifier => {
            MessagePublicationFailure::simple(MessagePublicationFailureKind::InvalidIntent)
        }
        MessageWireFailure::Serialization => {
            MessagePublicationFailure::simple(MessagePublicationFailureKind::Serialization)
        }
        MessageWireFailure::Signing => {
            MessagePublicationFailure::simple(MessagePublicationFailureKind::SigningUnavailable)
        }
    }
}
