use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, Sha256Digest,
    },
    ids::{AgentId, ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};
use sha2::{Digest, Sha256};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, ContentAccessMode, ContentAccessPolicy, ContentAuthorizationDecision,
        ContentAuthorizationFailure, ContentAuthorizationRequest, ContentByteStream,
        ContentLifecycleTransition, ContentMembershipAuthorizer, ContentRepository,
        ContentScanFailure, ContentScanner, ContentStorageKeyFactory,
        ContentStorageKeyGenerationFailure, ContentUploadClaim, ContentUploadClaimOutcome,
        ContentUploadFingerprint, IdentifierFactory, MatrixRoomId, ObjectStoreFailure,
        PrivateContentObjectStore,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginContentUploadRequest {
    pub request_id: ContentUploadRequestId,
    pub owner_principal_id: PrincipalId,
    pub actor_agent_id: Option<AgentId>,
    pub matrix_room_id: MatrixRoomId,
    pub access_mode: ContentAccessMode,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
    pub encryption_mode: ContentEncryptionMode,
    pub expires_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginContentUploadOutcome {
    Created {
        content: ContentObject,
        access_policy: ContentAccessPolicy,
    },
    Existing {
        content: ContentObject,
        access_policy: ContentAccessPolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginContentUploadFailure {
    Denied,
    Authorization(ContentAuthorizationFailure),
    Domain(DomainError),
    StorageKey(ContentStorageKeyGenerationFailure),
    Repository(RepositoryError),
}

pub type BeginContentUploadResult<T> = Result<T, BeginContentUploadFailure>;

pub trait ContentIdentifierFactory: Send + Sync {
    fn content_id(&self) -> ContentId;
}

impl<T> ContentIdentifierFactory for T
where
    T: IdentifierFactory + ?Sized,
{
    fn content_id(&self) -> ContentId {
        IdentifierFactory::content_id(self)
    }
}

pub struct BeginContentUploadDependencies {
    pub clock: Arc<dyn Clock>,
    pub identifiers: Arc<dyn ContentIdentifierFactory>,
    pub storage_keys: Arc<dyn ContentStorageKeyFactory>,
    pub repository: Arc<dyn ContentRepository>,
    pub authorizer: Arc<dyn ContentMembershipAuthorizer>,
}

pub struct BeginContentUploadService {
    clock: Arc<dyn Clock>,
    identifiers: Arc<dyn ContentIdentifierFactory>,
    storage_keys: Arc<dyn ContentStorageKeyFactory>,
    repository: Arc<dyn ContentRepository>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
}

impl BeginContentUploadService {
    pub fn new(dependencies: BeginContentUploadDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            identifiers: dependencies.identifiers,
            storage_keys: dependencies.storage_keys,
            repository: dependencies.repository,
            authorizer: dependencies.authorizer,
        }
    }

    /// 幂等创建上传会话和未绑定事件的房间访问策略。
    ///
    /// # Errors
    ///
    /// 声明违反领域约束、随机对象键生成失败或仓储不可用时返回错误。
    pub async fn begin(
        &self,
        request: BeginContentUploadRequest,
    ) -> BeginContentUploadResult<BeginContentUploadOutcome> {
        self.ensure_room_membership(&request).await?;
        let created_at = self.clock.now();
        let content_id = self.identifiers.content_id();
        let storage_key = self
            .storage_keys
            .generate(content_id)
            .map_err(BeginContentUploadFailure::StorageKey)?;
        let fingerprint = upload_fingerprint(&request);
        let scan_state = match request.encryption_mode {
            ContentEncryptionMode::ServerSide => ContentScanState::Pending,
            ContentEncryptionMode::ClientE2ee => ContentScanState::NotApplicable,
        };
        let content = ContentObject::begin_upload(ContentObjectFields {
            id: content_id,
            owner_principal_id: request.owner_principal_id,
            storage_key,
            digest: request.digest,
            byte_length: request.byte_length,
            media_type: request.media_type,
            encryption_mode: request.encryption_mode,
            scan_state,
            lifecycle_state: ContentLifecycleState::Uploading,
            expires_at: request.expires_at,
            created_at,
            deleted_at: None,
        })
        .map_err(BeginContentUploadFailure::Domain)?;
        let access_policy = ContentAccessPolicy::new(
            content_id,
            request.matrix_room_id,
            request.access_mode,
            created_at,
        );
        let outcome = self
            .repository
            .claim_upload(&ContentUploadClaim {
                request_id: request.request_id,
                fingerprint,
                content,
                access_policy,
            })
            .await
            .map_err(BeginContentUploadFailure::Repository)?;
        Ok(match outcome {
            ContentUploadClaimOutcome::Created {
                content,
                access_policy,
            } => BeginContentUploadOutcome::Created {
                content,
                access_policy,
            },
            ContentUploadClaimOutcome::Existing {
                content,
                access_policy,
            } => BeginContentUploadOutcome::Existing {
                content,
                access_policy,
            },
        })
    }

    async fn ensure_room_membership(
        &self,
        request: &BeginContentUploadRequest,
    ) -> BeginContentUploadResult<()> {
        let decision = self
            .authorizer
            .authorize(&ContentAuthorizationRequest {
                principal_id: request.owner_principal_id,
                actor_agent_id: request.actor_agent_id,
                owner_principal_id: request.owner_principal_id,
                matrix_room_id: request.matrix_room_id.clone(),
                access_mode: ContentAccessMode::RoomMember,
            })
            .await
            .map_err(BeginContentUploadFailure::Authorization)?;
        match decision {
            ContentAuthorizationDecision::Allowed => Ok(()),
            ContentAuthorizationDecision::Denied => Err(BeginContentUploadFailure::Denied),
        }
    }
}

fn upload_fingerprint(request: &BeginContentUploadRequest) -> ContentUploadFingerprint {
    let mut hasher = Sha256::new();
    update_field(&mut hasher, request.owner_principal_id.as_uuid().as_bytes());
    match request.actor_agent_id {
        Some(agent_id) => {
            hasher.update([1]);
            update_field(&mut hasher, agent_id.as_uuid().as_bytes());
        }
        None => hasher.update([0]),
    }
    update_field(&mut hasher, request.matrix_room_id.as_str().as_bytes());
    update_field(&mut hasher, request.access_mode.as_str().as_bytes());
    update_field(&mut hasher, request.digest.as_bytes());
    update_field(&mut hasher, &request.byte_length.value().to_be_bytes());
    update_field(&mut hasher, request.media_type.as_str().as_bytes());
    update_field(&mut hasher, request.encryption_mode.as_str().as_bytes());
    match request.expires_at {
        Some(expires_at) => {
            hasher.update([1]);
            update_field(&mut hasher, &expires_at.value().to_be_bytes());
        }
        None => hasher.update([0]),
    }
    ContentUploadFingerprint::from_bytes(hasher.finalize().into())
}

fn update_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

pub struct CompleteContentUploadRequest {
    pub principal_id: PrincipalId,
    pub content_id: ContentId,
    pub body: ContentByteStream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteContentUploadOutcome {
    Activated(ContentObject),
    AlreadyActive(ContentObject),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentUploadFailureStage {
    Load,
    StoreObject,
    ScanObject,
    RecordScan,
    Activate,
    MarkOrphaned,
    DeleteObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentUploadCompensationFailures {
    pub metadata: Option<RepositoryError>,
    pub object: Option<ObjectStoreFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteContentUploadFailure {
    NotFound,
    Forbidden,
    InvalidState(ContentLifecycleState),
    Repository {
        stage: ContentUploadFailureStage,
        error: RepositoryError,
    },
    ObjectStore {
        stage: ContentUploadFailureStage,
        error: ObjectStoreFailure,
    },
    Scan(ContentScanFailure),
    IntegrityMismatch {
        expected_digest: Sha256Digest,
        actual_digest: Sha256Digest,
        expected_byte_length: ContentByteLength,
        actual_byte_length: ContentByteLength,
        compensation: ContentUploadCompensationFailures,
    },
    ScanRejected {
        outcome: ContentScanState,
        compensation: ContentUploadCompensationFailures,
    },
}

pub type CompleteContentUploadResult<T> = Result<T, CompleteContentUploadFailure>;

pub struct CompleteContentUploadDependencies {
    pub clock: Arc<dyn Clock>,
    pub repository: Arc<dyn ContentRepository>,
    pub object_store: Arc<dyn PrivateContentObjectStore>,
    pub scanner: Arc<dyn ContentScanner>,
}

pub struct CompleteContentUploadService {
    clock: Arc<dyn Clock>,
    repository: Arc<dyn ContentRepository>,
    object_store: Arc<dyn PrivateContentObjectStore>,
    scanner: Arc<dyn ContentScanner>,
}

impl CompleteContentUploadService {
    pub fn new(dependencies: CompleteContentUploadDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            repository: dependencies.repository,
            object_store: dependencies.object_store,
            scanner: dependencies.scanner,
        }
    }

    /// 以分块流写入私有对象，验证真实摘要和长度，通过扫描后激活元数据。
    ///
    /// # Errors
    ///
    /// 内容不存在、主体不匹配、生命周期非法、对象存储/扫描/仓储失败或完整性不匹配时返回阶段化错误。
    pub async fn complete(
        &self,
        request: CompleteContentUploadRequest,
    ) -> CompleteContentUploadResult<CompleteContentUploadOutcome> {
        let content = self
            .repository
            .find_content(request.content_id)
            .await
            .map_err(|error| repository_failure(ContentUploadFailureStage::Load, error))?
            .ok_or(CompleteContentUploadFailure::NotFound)?;
        if content.owner_principal_id() != request.principal_id {
            return Err(CompleteContentUploadFailure::Forbidden);
        }
        match content.lifecycle_state() {
            ContentLifecycleState::Active => {
                return Ok(CompleteContentUploadOutcome::AlreadyActive(content));
            }
            ContentLifecycleState::Uploading => {}
            state => return Err(CompleteContentUploadFailure::InvalidState(state)),
        }

        let receipt = self
            .object_store
            .put(&content, request.body)
            .await
            .map_err(|error| CompleteContentUploadFailure::ObjectStore {
                stage: ContentUploadFailureStage::StoreObject,
                error,
            })?;
        if receipt.digest != content.digest() || receipt.byte_length != content.byte_length() {
            let compensation = self.compensate(&content).await;
            return Err(CompleteContentUploadFailure::IntegrityMismatch {
                expected_digest: content.digest(),
                actual_digest: receipt.digest,
                expected_byte_length: content.byte_length(),
                actual_byte_length: receipt.byte_length,
                compensation,
            });
        }

        let scanned = self.ensure_scanned(content).await?;
        match scanned.scan_state() {
            ContentScanState::Clean | ContentScanState::NotApplicable => {
                let active = self
                    .repository
                    .activate(scanned.id(), self.clock.now())
                    .await
                    .map_err(|error| {
                        repository_failure(ContentUploadFailureStage::Activate, error)
                    })?;
                Ok(CompleteContentUploadOutcome::Activated(active))
            }
            ContentScanState::Suspicious | ContentScanState::Rejected => {
                let outcome = scanned.scan_state();
                let compensation = self.compensate(&scanned).await;
                Err(CompleteContentUploadFailure::ScanRejected {
                    outcome,
                    compensation,
                })
            }
            ContentScanState::Pending => {
                Err(CompleteContentUploadFailure::Scan(ContentScanFailure::new(
                    "content.complete.scan_pending",
                    crate::ports::ContentScanFailureKind::InvalidResponse,
                )))
            }
        }
    }

    async fn ensure_scanned(
        &self,
        content: ContentObject,
    ) -> CompleteContentUploadResult<ContentObject> {
        if content.encryption_mode() == ContentEncryptionMode::ClientE2ee
            || content.scan_state() != ContentScanState::Pending
        {
            return Ok(content);
        }
        let outcome = self
            .scanner
            .scan(&content)
            .await
            .map_err(CompleteContentUploadFailure::Scan)?;
        if matches!(
            outcome,
            ContentScanState::Pending | ContentScanState::NotApplicable
        ) {
            return Err(CompleteContentUploadFailure::Scan(ContentScanFailure::new(
                "content.complete.scan_invalid_outcome",
                crate::ports::ContentScanFailureKind::InvalidResponse,
            )));
        }
        self.repository
            .record_scan(content.id(), outcome, self.clock.now())
            .await
            .map_err(|error| repository_failure(ContentUploadFailureStage::RecordScan, error))
    }

    async fn compensate(&self, content: &ContentObject) -> ContentUploadCompensationFailures {
        let metadata = self
            .repository
            .transition(&ContentLifecycleTransition {
                content_id: content.id(),
                expected: ContentLifecycleState::Uploading,
                target: ContentLifecycleState::Orphaned,
                changed_at: self.clock.now(),
            })
            .await
            .err();
        let object = self.object_store.delete(content).await.err();
        ContentUploadCompensationFailures { metadata, object }
    }
}

fn repository_failure(
    stage: ContentUploadFailureStage,
    error: RepositoryError,
) -> CompleteContentUploadFailure {
    CompleteContentUploadFailure::Repository { stage, error }
}
