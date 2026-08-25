use std::pin::Pin;

use agent_room_domain::{
    content::{ContentObject, ContentScanState, ContentStorageKey},
    ids::{AgentId, ContentId, PrincipalId},
    time::UtcMillis,
};
use futures_util::Stream;

use crate::persistence::RepositoryResult;

use super::{MatrixEventId, MatrixRoomId, MatrixUserId, PortFuture};

mod failures;
mod models;

pub use failures::{
    ContentAuthorizationFailure, ContentAuthorizationFailureKind, ContentAuthorizationResult,
    ContentRateLimitFailure, ContentRateLimitFailureKind, ContentRateLimitResult,
    ContentScanFailure, ContentScanFailureKind, ContentScanResult,
    ContentStorageKeyGenerationFailure, ContentStorageKeyGenerationResult, ContentStreamFailure,
    ContentStreamFailureKind, ContentTicketFailure, ContentTicketFailureKind, ContentTicketResult,
    ObjectStoreFailure, ObjectStoreFailureKind, ObjectStoreResult,
};
pub use models::{
    ContentAccessMode, ContentAccessPolicy, ContentAuthorizationDecision,
    ContentAuthorizationIntent, ContentAuthorizationRequest, ContentDownloadAttempt,
    ContentEventBinding, ContentLifecycleTransition, ContentRateLimitDecision, ContentReadTicket,
    ContentReadTicketClaims, ContentUploadClaim, ContentUploadClaimOutcome,
    ContentUploadFingerprint, ObjectWriteReceipt, OpenedContentObject, ReclaimableContentQuery,
};

pub type ContentStreamResult<T> = Result<T, ContentStreamFailure>;
pub type ContentByteStream =
    Pin<Box<dyn Stream<Item = ContentStreamResult<Vec<u8>>> + Send + 'static>>;

/// 内容元数据与幂等上传声明的权威存储。
pub trait ContentRepository: Send + Sync {
    fn claim_upload<'a>(
        &'a self,
        claim: &'a ContentUploadClaim,
    ) -> PortFuture<'a, RepositoryResult<ContentUploadClaimOutcome>>;

    fn find_content(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentObject>>>;

    fn find_access_policy(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentAccessPolicy>>>;

    fn activate(
        &self,
        content_id: ContentId,
        activated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>>;

    fn record_scan(
        &self,
        content_id: ContentId,
        outcome: ContentScanState,
        scanned_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>>;

    fn bind_event<'a>(
        &'a self,
        binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>>;

    fn transition<'a>(
        &'a self,
        transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>>;

    fn list_reclaimable<'a>(
        &'a self,
        query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>>;

    fn mark_deleted(
        &self,
        content_id: ContentId,
        deleted_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>>;
}

/// 默认私有的 S3 兼容对象存储边界；调用方永远拿不到永久对象 URL。
pub trait PrivateContentObjectStore: Send + Sync {
    fn put<'a>(
        &'a self,
        content: &'a ContentObject,
        body: ContentByteStream,
    ) -> PortFuture<'a, ObjectStoreResult<ObjectWriteReceipt>>;

    fn open<'a>(
        &'a self,
        content: &'a ContentObject,
    ) -> PortFuture<'a, ObjectStoreResult<OpenedContentObject>>;

    fn delete<'a>(&'a self, content: &'a ContentObject) -> PortFuture<'a, ObjectStoreResult<()>>;
}

/// 每次签发和使用票据时都可重新查询 Matrix 权威成员状态。
pub trait ContentMembershipAuthorizer: Send + Sync {
    fn authorize<'a>(
        &'a self,
        request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>>;
}

/// 把主体或其明确指定的 Agent 解析为当前有效的 Matrix 用户。
///
/// Agent 查询必须同时验证主体仍有操作权限，不能信任客户端自报的 Matrix 用户标识。
pub trait ContentPrincipalIdentityLookup: Send + Sync {
    fn find_active_matrix_user(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>>;

    fn find_active_agent_matrix_user(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>>;
}

/// 发行和验证分钟级自包含票据；实现不得持久化原始票据。
pub trait ContentReadTicketCodec: Send + Sync {
    fn issue<'a>(
        &'a self,
        claims: &'a ContentReadTicketClaims,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicket>>;

    fn verify<'a>(
        &'a self,
        ticket: &'a ContentReadTicket,
        expected_principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicketClaims>>;
}

pub trait ContentDownloadLimiter: Send + Sync {
    fn check<'a>(
        &'a self,
        attempt: &'a ContentDownloadAttempt,
    ) -> PortFuture<'a, ContentRateLimitResult<ContentRateLimitDecision>>;
}

/// 在隔离实现中扫描服务端可见正文；客户端 E2EE 密文不得送入此端口。
pub trait ContentScanner: Send + Sync {
    fn scan<'a>(
        &'a self,
        content: &'a ContentObject,
    ) -> PortFuture<'a, ContentScanResult<ContentScanState>>;
}

/// 使用加密安全随机源生成与用户、房间和原文件名无关的对象键。
pub trait ContentStorageKeyFactory: Send + Sync {
    /// 为内容生成不透明私有对象键。
    ///
    /// # Errors
    ///
    /// 操作系统随机源不可用或生成结果无法满足对象键约束时返回错误。
    fn generate(
        &self,
        content_id: ContentId,
    ) -> ContentStorageKeyGenerationResult<ContentStorageKey>;
}
