//! 网络 Agent 发言：交给本机 Bridge 同一个发布服务去签名、上传正文、发到 Matrix、绑定正文。
//! 这里只是把服务要的几个端口接到服务器这一侧：签名用封存的实例种子，发送用 Agent 自己的
//! Matrix 会话（进过加密房间的用它的加密客户端），正文直接进进程内的内容服务，提交记录按网络
//! Agent 存进数据库。

use std::sync::Arc;

use agent_room_application::{
    content::{
        BeginContentUploadOutcome, BeginContentUploadRequest, BindContentEventFailure,
        BindContentEventRequest, CompleteContentUploadFailure, CompleteContentUploadRequest,
        ContentUseCases, RedactContentFailure, RedactContentRequest,
    },
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, ContentAccessMode, ContentByteStream, DeviceSignature, MatrixAcceptedEvent,
        MatrixEvent, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixResult,
        MatrixRoomId, MatrixTransactionId, NetworkAgentMatrixGateway, NetworkAgentSubmissionClaim,
        NetworkAgentSubmissionClaimOutcome, NetworkAgentSubmissionKind,
        NetworkAgentSubmissionRecord, NetworkAgentSubmissionState, NetworkAgentSubmissionStore,
        PortFuture, SecretValue,
    },
};
use agent_room_bridge_core::{
    messages::{
        AutomationAuthorizationGateway, AutomationAuthorizationRequest,
        AutomationAuthorizationResult, MessageContentBindRequest, MessageContentFailure,
        MessageContentFailureKind, MessageContentGateway, MessageContentRecord,
        MessageContentRedactRequest, MessageContentUploadRequest, MessageEventPublisher,
        MessageStoreFailure, MessageStoreFailureKind, MessageSubmissionClaim,
        MessageSubmissionClaimOutcome, MessageSubmissionFingerprint, MessageSubmissionKind,
        MessageSubmissionRecord, MessageSubmissionRepository, MessageSubmissionState,
    },
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        DeviceSigningIdentity,
    },
};
use agent_room_domain::{
    devices::DevicePublicSigningKey,
    ids::{AgentId, MessageSubmissionId, NetworkAgentId, PrincipalId},
};
use agent_room_identity_adapter::Ed25519DeviceSigningKey;

/// 用封存的实例种子签名：与本机 Bridge 用实例密钥签名的效果完全一样。
pub(super) struct SessionSigner {
    key: Ed25519DeviceSigningKey,
}

impl SessionSigner {
    pub(super) fn from_seed(seed: &SecretValue) -> Option<Self> {
        Ed25519DeviceSigningKey::from_encoded_seed(seed)
            .ok()
            .map(|key| Self { key })
    }
}

const fn credential_failure() -> BridgeCredentialFailure {
    BridgeCredentialFailure::new(BridgeCredentialFailureKind::Corrupt)
}

impl DeviceSigningIdentity for SessionSigner {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        self.key.public_key().map_err(|_| credential_failure())
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.key.sign(message).map_err(|_| credential_failure())
    }
}

/// 以 Agent 自己的 Matrix 会话发出事件。
pub(super) struct SessionPublisher {
    pub(super) matrix: Arc<dyn NetworkAgentMatrixGateway>,
    pub(super) access_token: SecretValue,
}

impl MessageEventPublisher for SessionPublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        Box::pin(async move {
            self.matrix
                .send_event(&self.access_token, room_id, event)
                .await
                .map_err(unknown_commit_on_timeout)
        })
    }
}

/// 以进过加密房间的 Agent 的加密客户端发出事件：加密房间里由它加密。
pub(super) struct ClientPublisher {
    pub(super) matrix: Arc<dyn MatrixGateway>,
}

impl MessageEventPublisher for ClientPublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        Box::pin(async move {
            self.matrix
                .send_event(room_id, event)
                .await
                .map_err(unknown_commit_on_timeout)
        })
    }
}

/// 请求发出去后超时，Matrix 可能已经收下：按“不知道”处理，用同一个事务 ID 重发不会重复。
fn unknown_commit_on_timeout(failure: MatrixFailure) -> MatrixFailure {
    if failure.kind() == MatrixFailureKind::Timeout {
        MatrixFailure::new(failure.operation(), MatrixFailureKind::UnknownCommit)
    } else {
        failure
    }
}

/// 网络 Agent 的发言由服务器自己放行：频率已经按网络 Agent 限过，没有另外的发言授权。
pub(super) struct QuotaAlreadyTaken;

impl AutomationAuthorizationGateway for QuotaAlreadyTaken {
    fn authorize<'a>(
        &'a self,
        _request: &'a AutomationAuthorizationRequest,
    ) -> PortFuture<'a, AutomationAuthorizationResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// 正文直接进进程内的内容服务：归网络 Agent 的合成主体所有，由这个 Agent 发布，房间成员可读，
/// 与本机 Bridge 经 HTTP 上传的走同一套成员校验与扫描。
pub(super) struct InProcessContent {
    pub(super) content: Arc<dyn ContentUseCases>,
    pub(super) principal_id: PrincipalId,
    pub(super) agent_id: AgentId,
}

const fn content_failure(kind: MessageContentFailureKind) -> MessageContentFailure {
    MessageContentFailure::new(kind)
}

impl InProcessContent {
    async fn upload_internal(
        &self,
        request: &MessageContentUploadRequest,
    ) -> Result<MessageContentRecord, MessageContentFailure> {
        let begun = self
            .content
            .begin_upload(BeginContentUploadRequest {
                request_id: request.request_id,
                owner_principal_id: self.principal_id,
                actor_agent_id: Some(self.agent_id),
                matrix_room_id: request.room_id.clone(),
                access_mode: ContentAccessMode::RoomMember,
                digest: request.digest,
                byte_length: request.byte_length,
                media_type: request.media_type.clone(),
                encryption_mode: request.encryption_mode,
                expires_at: request.expires_at,
            })
            .await
            .map_err(|failure| match failure {
                agent_room_application::content::BeginContentUploadFailure::Denied => {
                    content_failure(MessageContentFailureKind::Denied)
                }
                agent_room_application::content::BeginContentUploadFailure::Domain(_) => {
                    content_failure(MessageContentFailureKind::InvalidRequest)
                }
                agent_room_application::content::BeginContentUploadFailure::Repository(error)
                    if error.kind() == RepositoryErrorKind::Conflict =>
                {
                    content_failure(MessageContentFailureKind::Conflict)
                }
                _ => content_failure(MessageContentFailureKind::Unavailable),
            })?;
        let (BeginContentUploadOutcome::Created { content, .. }
        | BeginContentUploadOutcome::Existing { content, .. }) = begun;
        let bytes = request.body.to_vec();
        let body: ContentByteStream =
            Box::pin(futures_util::stream::once(std::future::ready(Ok(bytes))));
        self.content
            .complete_upload(CompleteContentUploadRequest {
                principal_id: self.principal_id,
                content_id: content.id(),
                body,
            })
            .await
            .map_err(|failure| match failure {
                CompleteContentUploadFailure::Forbidden => {
                    content_failure(MessageContentFailureKind::Denied)
                }
                CompleteContentUploadFailure::NotFound
                | CompleteContentUploadFailure::InvalidState(_)
                | CompleteContentUploadFailure::IntegrityMismatch { .. } => {
                    content_failure(MessageContentFailureKind::Conflict)
                }
                _ => content_failure(MessageContentFailureKind::Unavailable),
            })?;
        Ok(MessageContentRecord {
            content_id: content.id(),
            digest: request.digest,
            byte_length: request.byte_length,
            media_type: request.media_type.clone(),
        })
    }

    async fn bind_internal(
        &self,
        request: &MessageContentBindRequest,
    ) -> Result<(), MessageContentFailure> {
        self.content
            .bind_event(BindContentEventRequest {
                principal_id: self.principal_id,
                content_id: request.content_id,
                matrix_room_id: request.room_id.clone(),
                matrix_event_id: request.event_id.clone(),
            })
            .await
            .map(|_| ())
            .map_err(|failure| match failure {
                BindContentEventFailure::Forbidden | BindContentEventFailure::Revoked => {
                    content_failure(MessageContentFailureKind::Denied)
                }
                BindContentEventFailure::NotFound
                | BindContentEventFailure::InvalidState(_)
                | BindContentEventFailure::PolicyMismatch
                | BindContentEventFailure::EventConflict => {
                    content_failure(MessageContentFailureKind::Conflict)
                }
                BindContentEventFailure::Repository(_) => {
                    content_failure(MessageContentFailureKind::Unavailable)
                }
            })
    }

    async fn redact_internal(
        &self,
        request: &MessageContentRedactRequest,
    ) -> Result<(), MessageContentFailure> {
        self.content
            .redact(RedactContentRequest {
                principal_id: self.principal_id,
                content_id: request.content_id,
            })
            .await
            .map(|_| ())
            .map_err(|failure| match failure {
                RedactContentFailure::Forbidden => {
                    content_failure(MessageContentFailureKind::Denied)
                }
                RedactContentFailure::NotFound | RedactContentFailure::InvalidState(_) => {
                    content_failure(MessageContentFailureKind::Conflict)
                }
                RedactContentFailure::Repository(_) => {
                    content_failure(MessageContentFailureKind::Unavailable)
                }
            })
    }
}

impl MessageContentGateway for InProcessContent {
    fn upload<'a>(
        &'a self,
        request: &'a MessageContentUploadRequest,
    ) -> PortFuture<'a, Result<MessageContentRecord, MessageContentFailure>> {
        Box::pin(self.upload_internal(request))
    }

    fn bind<'a>(
        &'a self,
        request: &'a MessageContentBindRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
        Box::pin(self.bind_internal(request))
    }

    fn redact<'a>(
        &'a self,
        request: &'a MessageContentRedactRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
        Box::pin(self.redact_internal(request))
    }
}

/// 发布服务要的提交记录，落在这个网络 Agent 名下。
pub(super) struct AgentSubmissions {
    pub(super) store: Arc<dyn NetworkAgentSubmissionStore>,
    pub(super) id: NetworkAgentId,
    pub(super) clock: Arc<dyn Clock>,
}

const fn submission_kind(kind: MessageSubmissionKind) -> NetworkAgentSubmissionKind {
    match kind {
        MessageSubmissionKind::Preview => NetworkAgentSubmissionKind::Preview,
        MessageSubmissionKind::Replace => NetworkAgentSubmissionKind::Replace,
        MessageSubmissionKind::Redact => NetworkAgentSubmissionKind::Redact,
    }
}

fn record(stored: NetworkAgentSubmissionRecord) -> MessageSubmissionRecord {
    MessageSubmissionRecord {
        submission_id: stored.submission_id,
        kind: match stored.kind {
            NetworkAgentSubmissionKind::Preview => MessageSubmissionKind::Preview,
            NetworkAgentSubmissionKind::Replace => MessageSubmissionKind::Replace,
            NetworkAgentSubmissionKind::Redact => MessageSubmissionKind::Redact,
        },
        fingerprint: MessageSubmissionFingerprint::from_bytes(stored.fingerprint),
        transaction_id: stored.transaction_id,
        state: match stored.state {
            NetworkAgentSubmissionState::Claimed => MessageSubmissionState::Claimed,
            NetworkAgentSubmissionState::SubmitUnknown => MessageSubmissionState::SubmitUnknown,
            NetworkAgentSubmissionState::Accepted => MessageSubmissionState::Accepted,
            NetworkAgentSubmissionState::Bound => MessageSubmissionState::Bound,
        },
        event_id: stored.event_id,
    }
}

fn store_failure(error: &RepositoryError) -> MessageStoreFailure {
    MessageStoreFailure::new(match error.kind() {
        RepositoryErrorKind::Conflict => MessageStoreFailureKind::Conflict,
        RepositoryErrorKind::NotFound => MessageStoreFailureKind::NotFound,
        RepositoryErrorKind::CorruptData => MessageStoreFailureKind::Corrupt,
        _ => MessageStoreFailureKind::Unavailable,
    })
}

impl MessageSubmissionRepository for AgentSubmissions {
    fn claim<'a>(
        &'a self,
        claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>> {
        Box::pin(async move {
            let outcome = self
                .store
                .claim(
                    self.id,
                    &NetworkAgentSubmissionClaim {
                        submission_id: claim.submission_id,
                        kind: submission_kind(claim.kind),
                        fingerprint: claim.fingerprint.as_bytes(),
                        transaction_id: claim.transaction_id.clone(),
                        claimed_at: self.clock.now(),
                    },
                )
                .await
                .map_err(|error| store_failure(&error))?;
            Ok(match outcome {
                NetworkAgentSubmissionClaimOutcome::Created(stored) => {
                    MessageSubmissionClaimOutcome::Created(record(stored))
                }
                NetworkAgentSubmissionClaimOutcome::Existing(stored) => {
                    MessageSubmissionClaimOutcome::Existing(record(stored))
                }
            })
        })
    }

    fn mark_submit_unknown(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move {
            self.store
                .mark_submit_unknown(self.id, submission_id)
                .await
                .map(record)
                .map_err(|error| store_failure(&error))
        })
    }

    fn mark_accepted<'a>(
        &'a self,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move {
            self.store
                .mark_accepted(self.id, submission_id, event_id)
                .await
                .map(record)
                .map_err(|error| store_failure(&error))
        })
    }

    fn mark_bound(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move {
            self.store
                .mark_bound(self.id, submission_id)
                .await
                .map(record)
                .map_err(|error| store_failure(&error))
        })
    }

    fn observe_transaction<'a>(
        &'a self,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        Box::pin(async move {
            self.store
                .observe_transaction(self.id, transaction_id, event_id)
                .await
                .map(|stored| stored.map(record))
                .map_err(|error| store_failure(&error))
        })
    }
}
