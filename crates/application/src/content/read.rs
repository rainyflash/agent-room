use std::{fmt, sync::Arc};

use agent_room_domain::{
    DomainError,
    content::{
        ContentByteLength, ContentLifecycleState, ContentMediaType, ContentObject, Sha256Digest,
    },
    ids::{AgentId, ContentId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, ContentAuthorizationDecision, ContentAuthorizationFailure,
        ContentAuthorizationIntent, ContentAuthorizationRequest, ContentByteStream,
        ContentDownloadAttempt, ContentDownloadLimiter, ContentMembershipAuthorizer,
        ContentRateLimitDecision, ContentRateLimitFailure, ContentReadTicket,
        ContentReadTicketClaims, ContentReadTicketCodec, ContentRepository, ContentStreamFailure,
        ContentStreamFailureKind, ContentTicketFailure, MatrixRoomId, ObjectStoreFailure,
        PrivateContentObjectStore,
    },
};

const MAX_TICKET_LIFETIME_MILLIS: u64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentReadTicketLifetime(DurationMillis);

impl ContentReadTicketLifetime {
    /// 创建分钟级读取票据寿命。
    ///
    /// # Errors
    ///
    /// 零时长或超过五分钟时返回错误，避免把短期能力票据退化成长期下载凭据。
    pub fn new(milliseconds: u64) -> Result<Self, DomainError> {
        if milliseconds > MAX_TICKET_LIFETIME_MILLIS {
            return Err(DomainError::Validation {
                field: "content_read_ticket_lifetime",
                reason: "不能超过五分钟",
            });
        }
        DurationMillis::new(milliseconds).map(Self)
    }

    const fn duration(self) -> DurationMillis {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueContentReadTicketRequest {
    pub principal_id: PrincipalId,
    pub actor_agent_id: Option<AgentId>,
    pub content_id: ContentId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedContentReadTicket {
    pub ticket: ContentReadTicket,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueContentReadTicketFailure {
    NotFound,
    NotReadable(ContentLifecycleState),
    EventNotBound,
    Denied,
    Domain(DomainError),
    Repository(RepositoryError),
    Authorization(ContentAuthorizationFailure),
    Ticket(ContentTicketFailure),
}

pub type IssueContentReadTicketResult<T> = Result<T, IssueContentReadTicketFailure>;

pub struct IssueContentReadTicketDependencies {
    pub clock: Arc<dyn Clock>,
    pub repository: Arc<dyn ContentRepository>,
    pub authorizer: Arc<dyn ContentMembershipAuthorizer>,
    pub ticket_codec: Arc<dyn ContentReadTicketCodec>,
    pub lifetime: ContentReadTicketLifetime,
}

pub struct IssueContentReadTicketService {
    clock: Arc<dyn Clock>,
    repository: Arc<dyn ContentRepository>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
    ticket_codec: Arc<dyn ContentReadTicketCodec>,
    lifetime: ContentReadTicketLifetime,
}

impl IssueContentReadTicketService {
    pub fn new(dependencies: IssueContentReadTicketDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            repository: dependencies.repository,
            authorizer: dependencies.authorizer,
            ticket_codec: dependencies.ticket_codec,
            lifetime: dependencies.lifetime,
        }
    }

    /// 在当前成员资格、事件绑定和内容生命周期均有效时签发短期能力票据。
    ///
    /// # Errors
    ///
    /// 内容不可读、策略未绑定事件、成员资格不足或任一依赖失败时返回错误。
    pub async fn issue(
        &self,
        request: IssueContentReadTicketRequest,
    ) -> IssueContentReadTicketResult<IssuedContentReadTicket> {
        let now = self.clock.now();
        let (content, policy) = load_content_and_policy(&*self.repository, request.content_id)
            .await
            .map_err(map_issue_load_failure)?;
        ensure_readable(&content, now).map_err(map_issue_readability_failure)?;
        let event_id = policy
            .matrix_event_id()
            .cloned()
            .ok_or(IssueContentReadTicketFailure::EventNotBound)?;
        if policy.is_revoked() {
            return Err(IssueContentReadTicketFailure::Denied);
        }
        authorize(
            &*self.authorizer,
            request.principal_id,
            request.actor_agent_id,
            &content,
            policy.matrix_room_id(),
            policy.access_mode(),
        )
        .await
        .map_err(map_issue_authorization_failure)?;

        let configured_expiry = now
            .checked_add(self.lifetime.duration())
            .map_err(IssueContentReadTicketFailure::Domain)?;
        let expires_at = content
            .expires_at()
            .map_or(configured_expiry, |content_expiry| {
                content_expiry.min(configured_expiry)
            });
        let claims = ContentReadTicketClaims {
            principal_id: request.principal_id,
            actor_agent_id: request.actor_agent_id,
            content_id: content.id(),
            matrix_room_id: policy.matrix_room_id().clone(),
            matrix_event_id: event_id,
            digest: content.digest(),
            byte_length: content.byte_length(),
            media_type: content.media_type().clone(),
            issued_at: now,
            expires_at,
        };
        let ticket = self
            .ticket_codec
            .issue(&claims)
            .await
            .map_err(IssueContentReadTicketFailure::Ticket)?;
        Ok(IssuedContentReadTicket { ticket, expires_at })
    }
}

pub struct OpenContentRequest {
    pub principal_id: PrincipalId,
    pub content_id: ContentId,
    pub ticket: ContentReadTicket,
}

pub struct OpenedVerifiedContent {
    pub content_id: ContentId,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
    pub body: ContentByteStream,
}

impl fmt::Debug for OpenedVerifiedContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenedVerifiedContent")
            .field("content_id", &self.content_id)
            .field("digest", &self.digest)
            .field("byte_length", &self.byte_length)
            .field("media_type", &self.media_type)
            .field("body", &"[摘要校验内容流]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenContentFailure {
    Ticket(ContentTicketFailure),
    NotFound,
    NotReadable(ContentLifecycleState),
    StaleTicket,
    Denied,
    Repository(RepositoryError),
    Authorization(ContentAuthorizationFailure),
    RateLimit(ContentRateLimitFailure),
    RateLimited { retry_at: UtcMillis },
    ObjectStore(ObjectStoreFailure),
    ObjectMetadataMismatch,
}

pub type OpenContentResult<T> = Result<T, OpenContentFailure>;

pub struct OpenContentDependencies {
    pub clock: Arc<dyn Clock>,
    pub repository: Arc<dyn ContentRepository>,
    pub authorizer: Arc<dyn ContentMembershipAuthorizer>,
    pub ticket_codec: Arc<dyn ContentReadTicketCodec>,
    pub limiter: Arc<dyn ContentDownloadLimiter>,
    pub object_store: Arc<dyn PrivateContentObjectStore>,
}

pub struct OpenContentService {
    clock: Arc<dyn Clock>,
    repository: Arc<dyn ContentRepository>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
    ticket_codec: Arc<dyn ContentReadTicketCodec>,
    limiter: Arc<dyn ContentDownloadLimiter>,
    object_store: Arc<dyn PrivateContentObjectStore>,
}

impl OpenContentService {
    pub fn new(dependencies: OpenContentDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            repository: dependencies.repository,
            authorizer: dependencies.authorizer,
            ticket_codec: dependencies.ticket_codec,
            limiter: dependencies.limiter,
            object_store: dependencies.object_store,
        }
    }

    /// 重新校验票据、权威成员资格和限流状态后打开摘要校验流。
    ///
    /// # Errors
    ///
    /// 票据失效、权限变化、内容状态变化、限流或对象元数据不一致时拒绝读取。
    pub async fn open(
        &self,
        request: OpenContentRequest,
    ) -> OpenContentResult<OpenedVerifiedContent> {
        let now = self.clock.now();
        let claims = self
            .ticket_codec
            .verify(&request.ticket, request.principal_id, now)
            .await
            .map_err(OpenContentFailure::Ticket)?;
        if claims.content_id != request.content_id {
            return Err(OpenContentFailure::StaleTicket);
        }
        let (content, policy) = load_content_and_policy(&*self.repository, claims.content_id)
            .await
            .map_err(map_open_load_failure)?;
        ensure_readable(&content, now).map_err(map_open_readability_failure)?;
        if !claims_match(&claims, &content, &policy) {
            return Err(OpenContentFailure::StaleTicket);
        }
        if policy.is_revoked() {
            return Err(OpenContentFailure::Denied);
        }
        authorize(
            &*self.authorizer,
            request.principal_id,
            claims.actor_agent_id,
            &content,
            policy.matrix_room_id(),
            policy.access_mode(),
        )
        .await
        .map_err(map_open_authorization_failure)?;
        match self
            .limiter
            .check(&ContentDownloadAttempt {
                principal_id: request.principal_id,
                content_id: content.id(),
                matrix_room_id: policy.matrix_room_id().clone(),
                byte_length: content.byte_length(),
                attempted_at: now,
            })
            .await
            .map_err(OpenContentFailure::RateLimit)?
        {
            ContentRateLimitDecision::Allowed => {}
            ContentRateLimitDecision::RetryAt(retry_at) => {
                return Err(OpenContentFailure::RateLimited { retry_at });
            }
        }

        let opened = self
            .object_store
            .open(&content)
            .await
            .map_err(OpenContentFailure::ObjectStore)?;
        if opened.reported_digest != Some(content.digest())
            || opened.reported_byte_length != Some(content.byte_length())
        {
            return Err(OpenContentFailure::ObjectMetadataMismatch);
        }
        Ok(OpenedVerifiedContent {
            content_id: content.id(),
            digest: content.digest(),
            byte_length: content.byte_length(),
            media_type: content.media_type().clone(),
            body: integrity_checked_stream(opened.body, content.digest(), content.byte_length()),
        })
    }
}

enum LoadContentFailure {
    NotFound,
    Repository(RepositoryError),
}

async fn load_content_and_policy(
    repository: &dyn ContentRepository,
    content_id: ContentId,
) -> Result<(ContentObject, crate::ports::ContentAccessPolicy), LoadContentFailure> {
    let content = repository
        .find_content(content_id)
        .await
        .map_err(LoadContentFailure::Repository)?
        .ok_or(LoadContentFailure::NotFound)?;
    let policy = repository
        .find_access_policy(content_id)
        .await
        .map_err(LoadContentFailure::Repository)?
        .ok_or(LoadContentFailure::NotFound)?;
    Ok((content, policy))
}

fn ensure_readable(content: &ContentObject, now: UtcMillis) -> Result<(), ContentLifecycleState> {
    if content.is_readable_at(now) {
        Ok(())
    } else {
        Err(content.lifecycle_state())
    }
}

async fn authorize(
    authorizer: &dyn ContentMembershipAuthorizer,
    principal_id: PrincipalId,
    actor_agent_id: Option<AgentId>,
    content: &ContentObject,
    matrix_room_id: &MatrixRoomId,
    access_mode: crate::ports::ContentAccessMode,
) -> Result<(), AuthorizationFailure> {
    let decision = authorizer
        .authorize(&ContentAuthorizationRequest {
            principal_id,
            actor_agent_id,
            owner_principal_id: content.owner_principal_id(),
            matrix_room_id: matrix_room_id.clone(),
            access_mode,
            intent: ContentAuthorizationIntent::Read,
        })
        .await
        .map_err(AuthorizationFailure::Dependency)?;
    match decision {
        ContentAuthorizationDecision::Allowed => Ok(()),
        ContentAuthorizationDecision::Denied => Err(AuthorizationFailure::Denied),
    }
}

enum AuthorizationFailure {
    Denied,
    Dependency(ContentAuthorizationFailure),
}

fn claims_match(
    claims: &ContentReadTicketClaims,
    content: &ContentObject,
    policy: &crate::ports::ContentAccessPolicy,
) -> bool {
    claims.content_id == content.id()
        && claims.matrix_room_id == *policy.matrix_room_id()
        && policy.matrix_event_id() == Some(&claims.matrix_event_id)
        && claims.digest == content.digest()
        && claims.byte_length == content.byte_length()
        && claims.media_type == *content.media_type()
}

enum IntegrityState {
    Reading {
        source: ContentByteStream,
        hasher: Sha256,
        observed_length: u64,
        expected_digest: Sha256Digest,
        expected_length: ContentByteLength,
    },
    Finished,
}

fn integrity_checked_stream(
    source: ContentByteStream,
    expected_digest: Sha256Digest,
    expected_length: ContentByteLength,
) -> ContentByteStream {
    let state = IntegrityState::Reading {
        source,
        hasher: Sha256::new(),
        observed_length: 0,
        expected_digest,
        expected_length,
    };
    Box::pin(stream::unfold(state, |state| async move {
        let IntegrityState::Reading {
            mut source,
            mut hasher,
            observed_length,
            expected_digest,
            expected_length,
        } = state
        else {
            return None;
        };
        match source.next().await {
            Some(Ok(chunk)) => {
                let Some(observed_length) =
                    observed_length.checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
                else {
                    return Some((Err(integrity_failure()), IntegrityState::Finished));
                };
                if observed_length > expected_length.value() {
                    return Some((Err(integrity_failure()), IntegrityState::Finished));
                }
                hasher.update(&chunk);
                Some((
                    Ok(chunk),
                    IntegrityState::Reading {
                        source,
                        hasher,
                        observed_length,
                        expected_digest,
                        expected_length,
                    },
                ))
            }
            Some(Err(failure)) => Some((Err(failure), IntegrityState::Finished)),
            None => {
                let observed_digest = Sha256Digest::from_bytes(hasher.finalize().into());
                if observed_length == expected_length.value() && observed_digest == expected_digest
                {
                    None
                } else {
                    Some((Err(integrity_failure()), IntegrityState::Finished))
                }
            }
        }
    }))
}

const fn integrity_failure() -> ContentStreamFailure {
    ContentStreamFailure::new(
        "content.open.verify_integrity",
        ContentStreamFailureKind::IntegrityMismatch,
    )
}

fn map_issue_load_failure(failure: LoadContentFailure) -> IssueContentReadTicketFailure {
    match failure {
        LoadContentFailure::NotFound => IssueContentReadTicketFailure::NotFound,
        LoadContentFailure::Repository(error) => IssueContentReadTicketFailure::Repository(error),
    }
}

const fn map_issue_readability_failure(
    state: ContentLifecycleState,
) -> IssueContentReadTicketFailure {
    IssueContentReadTicketFailure::NotReadable(state)
}

fn map_issue_authorization_failure(failure: AuthorizationFailure) -> IssueContentReadTicketFailure {
    match failure {
        AuthorizationFailure::Denied => IssueContentReadTicketFailure::Denied,
        AuthorizationFailure::Dependency(error) => {
            IssueContentReadTicketFailure::Authorization(error)
        }
    }
}

fn map_open_load_failure(failure: LoadContentFailure) -> OpenContentFailure {
    match failure {
        LoadContentFailure::NotFound => OpenContentFailure::NotFound,
        LoadContentFailure::Repository(error) => OpenContentFailure::Repository(error),
    }
}

const fn map_open_readability_failure(state: ContentLifecycleState) -> OpenContentFailure {
    OpenContentFailure::NotReadable(state)
}

fn map_open_authorization_failure(failure: AuthorizationFailure) -> OpenContentFailure {
    match failure {
        AuthorizationFailure::Denied => OpenContentFailure::Denied,
        AuthorizationFailure::Dependency(error) => OpenContentFailure::Authorization(error),
    }
}

#[cfg(test)]
mod tests {
    use agent_room_domain::DomainError;

    use super::ContentReadTicketLifetime;

    #[test]
    fn 读取票据寿命被硬限制在五分钟内() {
        assert!(ContentReadTicketLifetime::new(1).is_ok());
        assert!(matches!(
            ContentReadTicketLifetime::new(300_001),
            Err(DomainError::Validation {
                field: "content_read_ticket_lifetime",
                ..
            })
        ));
    }
}
