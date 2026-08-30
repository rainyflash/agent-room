use std::sync::Arc;

use agent_room_application::ports::{Clock, PortFuture};
use agent_room_domain::{
    content::Sha256Digest,
    handoff::{HandoffFailureCode, HandoffPermission, TargetedHandoff, TargetedHandoffStatus},
    ids::HandoffId,
};
use sha2::{Digest as _, Sha256};

use crate::messages::{MessageContentReadGateway, MessageContentReadRequest};

use super::{TargetedHandoffQueueGateway, TargetedHandoffReceipt, TargetedHandoffTarget};

pub const TARGETED_HANDOFF_INBOX_PAGE_LIMIT: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedHandoffInboxFailureKind {
    Conflict,
    NotFound,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffInboxFailure {
    kind: TargetedHandoffInboxFailureKind,
}

impl TargetedHandoffInboxFailure {
    pub const fn new(kind: TargetedHandoffInboxFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> TargetedHandoffInboxFailureKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedHandoffInboxRecordOutcome {
    Created(TargetedHandoff),
    Existing(TargetedHandoff),
}

impl TargetedHandoffInboxRecordOutcome {
    pub const fn handoff(&self) -> &TargetedHandoff {
        match self {
            Self::Created(handoff) | Self::Existing(handoff) => handoff,
        }
    }
}

/// 只持久化云端已领取交接的元数据；实现不得在此表保存消息正文。
pub trait TargetedHandoffInbox: Send + Sync {
    fn accept<'a>(
        &'a self,
        handoff: &'a TargetedHandoff,
    ) -> PortFuture<'a, Result<TargetedHandoffInboxRecordOutcome, TargetedHandoffInboxFailure>>;

    fn find(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<TargetedHandoff>, TargetedHandoffInboxFailure>>;

    fn list(
        &self,
        target: TargetedHandoffTarget,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<TargetedHandoff>, TargetedHandoffInboxFailure>>;

    fn remove(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<bool, TargetedHandoffInboxFailure>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedHandoffClaimOutcome {
    Empty,
    Stored(Box<TargetedHandoff>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedTargetedHandoff {
    handoff: TargetedHandoff,
    body: Arc<[u8]>,
}

impl ConsumedTargetedHandoff {
    pub const fn new(handoff: TargetedHandoff, body: Arc<[u8]>) -> Self {
        Self { handoff, body }
    }

    pub const fn handoff(&self) -> &TargetedHandoff {
        &self.handoff
    }

    pub const fn body(&self) -> &Arc<[u8]> {
        &self.body
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedHandoffInboxServiceFailureKind {
    InvalidRequest,
    NotFound,
    Expired,
    PermissionDenied,
    ContentUnavailable,
    IntegrityMismatch,
    QueueUnavailable,
    StorageUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffInboxServiceFailure {
    kind: TargetedHandoffInboxServiceFailureKind,
}

impl TargetedHandoffInboxServiceFailure {
    pub const fn new(kind: TargetedHandoffInboxServiceFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> TargetedHandoffInboxServiceFailureKind {
        self.kind
    }
}

pub struct TargetedHandoffInboxDependencies {
    pub target: TargetedHandoffTarget,
    pub queue: Arc<dyn TargetedHandoffQueueGateway>,
    pub inbox: Arc<dyn TargetedHandoffInbox>,
    pub content: Arc<dyn MessageContentReadGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct TargetedHandoffInboxService {
    target: TargetedHandoffTarget,
    queue: Arc<dyn TargetedHandoffQueueGateway>,
    inbox: Arc<dyn TargetedHandoffInbox>,
    content: Arc<dyn MessageContentReadGateway>,
    clock: Arc<dyn Clock>,
}

impl TargetedHandoffInboxService {
    pub fn new(dependencies: TargetedHandoffInboxDependencies) -> Self {
        Self {
            target: dependencies.target,
            queue: dependencies.queue,
            inbox: dependencies.inbox,
            content: dependencies.content,
            clock: dependencies.clock,
        }
    }

    /// 从云端领取一个工作项并只落本地元数据。
    ///
    /// # Errors
    ///
    /// 队列或本地存储不可用时返回稳定失败。本地落盘失败后会尽力向云端提交失败回执，
    /// 避免把不可见的交接永久留在 `delivered` 状态。
    pub async fn claim_once(
        &self,
    ) -> Result<TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure> {
        let Some(handoff) = self.queue.claim_next(self.target).await.map_err(|_| {
            service_failure(TargetedHandoffInboxServiceFailureKind::QueueUnavailable)
        })?
        else {
            return Ok(TargetedHandoffClaimOutcome::Empty);
        };
        if !matches_target(&handoff, self.target)
            || handoff.status() != TargetedHandoffStatus::Delivered
        {
            return Err(service_failure(
                TargetedHandoffInboxServiceFailureKind::InvalidRequest,
            ));
        }
        if let Ok(outcome) = self.inbox.accept(&handoff).await {
            return Ok(TargetedHandoffClaimOutcome::Stored(Box::new(
                outcome.handoff().clone(),
            )));
        }
        let code = stable_failure_code("bridge.inbox_persist_failed");
        let _ = self
            .queue
            .record_receipt(
                self.target,
                handoff.fields().id,
                &TargetedHandoffReceipt::Failed(code),
            )
            .await;
        Err(service_failure(
            TargetedHandoffInboxServiceFailureKind::StorageUnavailable,
        ))
    }

    /// 列出当前实例已经领取、尚未由 Agent 处理的交接。
    ///
    /// # Errors
    ///
    /// 页大小越界或本地存储不可用时返回稳定失败。
    pub async fn list_pending(
        &self,
        limit: u16,
    ) -> Result<Vec<TargetedHandoff>, TargetedHandoffInboxServiceFailure> {
        if !(1..=TARGETED_HANDOFF_INBOX_PAGE_LIMIT).contains(&limit) {
            return Err(service_failure(
                TargetedHandoffInboxServiceFailureKind::InvalidRequest,
            ));
        }
        self.inbox.list(self.target, limit).await.map_err(|_| {
            service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
        })
    }

    /// 明确下载并消费一个交接。正文只有在云端消费回执成功且本地元数据删除后才会返回。
    ///
    /// # Errors
    ///
    /// 交接不存在、过期、权限不允许、正文不可用、完整性失败或回执失败时返回稳定错误。
    pub async fn consume(
        &self,
        handoff_id: HandoffId,
    ) -> Result<ConsumedTargetedHandoff, TargetedHandoffInboxServiceFailure> {
        let handoff = self.load_active(handoff_id).await?;
        if !scope_allows_content(&handoff) {
            return self
                .fail_local_handoff(
                    handoff,
                    "bridge.permission_mismatch",
                    TargetedHandoffInboxServiceFailureKind::PermissionDenied,
                )
                .await;
        }
        let fields = handoff.fields();
        let opened = self
            .content
            .open(&MessageContentReadRequest::new(
                fields.content.content_id(),
                fields.content.byte_length().value(),
            ))
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::ContentUnavailable)
            })?;
        if !content_matches(&handoff, &opened) {
            return self
                .fail_local_handoff(
                    handoff,
                    "bridge.content_integrity_mismatch",
                    TargetedHandoffInboxServiceFailureKind::IntegrityMismatch,
                )
                .await;
        }
        self.queue
            .record_receipt(self.target, handoff_id, &TargetedHandoffReceipt::Consumed)
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::QueueUnavailable)
            })?;
        self.inbox
            .remove(self.target, handoff_id)
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
            })?;
        Ok(ConsumedTargetedHandoff::new(handoff, opened.bytes))
    }

    /// 拒绝一个未消费交接，并在云端回执成功后清除本地元数据。
    ///
    /// # Errors
    ///
    /// 交接不存在、过期、队列或本地存储不可用时返回稳定失败。
    pub async fn decline(
        &self,
        handoff_id: HandoffId,
    ) -> Result<TargetedHandoff, TargetedHandoffInboxServiceFailure> {
        self.load_active(handoff_id).await?;
        let resolved = self
            .queue
            .record_receipt(
                self.target,
                handoff_id,
                &TargetedHandoffReceipt::Declined(stable_failure_code("agent.declined")),
            )
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::QueueUnavailable)
            })?;
        self.inbox
            .remove(self.target, handoff_id)
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
            })?;
        Ok(resolved)
    }

    async fn load_active(
        &self,
        handoff_id: HandoffId,
    ) -> Result<TargetedHandoff, TargetedHandoffInboxServiceFailure> {
        let handoff = self
            .inbox
            .find(self.target, handoff_id)
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
            })?
            .ok_or_else(|| service_failure(TargetedHandoffInboxServiceFailureKind::NotFound))?;
        if !matches_target(&handoff, self.target) {
            return Err(service_failure(
                TargetedHandoffInboxServiceFailureKind::InvalidRequest,
            ));
        }
        if self.clock.now() >= handoff.fields().expires_at {
            self.inbox
                .remove(self.target, handoff_id)
                .await
                .map_err(|_| {
                    service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
                })?;
            return Err(service_failure(
                TargetedHandoffInboxServiceFailureKind::Expired,
            ));
        }
        Ok(handoff)
    }

    async fn fail_local_handoff<T>(
        &self,
        handoff: TargetedHandoff,
        failure_code: &str,
        kind: TargetedHandoffInboxServiceFailureKind,
    ) -> Result<T, TargetedHandoffInboxServiceFailure> {
        let handoff_id = handoff.fields().id;
        self.queue
            .record_receipt(
                self.target,
                handoff_id,
                &TargetedHandoffReceipt::Failed(stable_failure_code(failure_code)),
            )
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::QueueUnavailable)
            })?;
        self.inbox
            .remove(self.target, handoff_id)
            .await
            .map_err(|_| {
                service_failure(TargetedHandoffInboxServiceFailureKind::StorageUnavailable)
            })?;
        Err(service_failure(kind))
    }
}

fn content_matches(
    handoff: &TargetedHandoff,
    opened: &crate::messages::DownloadedMessageContent,
) -> bool {
    let expected = &handoff.fields().content;
    let actual_length = u64::try_from(opened.bytes.len()).ok();
    let actual_digest = Sha256Digest::from_bytes(Sha256::digest(&opened.bytes).into());
    opened.byte_length == expected.byte_length()
        && opened.media_type == *expected.media_type()
        && opened.digest == expected.digest()
        && actual_length == Some(expected.byte_length().value())
        && actual_digest == expected.digest()
}

fn scope_allows_content(handoff: &TargetedHandoff) -> bool {
    let fields = handoff.fields();
    fields.permissions.contains(
        if matches!(
            fields.content.media_type().as_str(),
            "application/json" | "text/markdown" | "text/plain"
        ) {
            HandoffPermission::ReadText
        } else {
            HandoffPermission::ReadAttachments
        },
    )
}

fn matches_target(handoff: &TargetedHandoff, target: TargetedHandoffTarget) -> bool {
    let fields = handoff.fields();
    fields.target_agent_id == target.agent_id && fields.target_instance_id == target.instance_id
}

fn stable_failure_code(value: &str) -> HandoffFailureCode {
    HandoffFailureCode::new(value).expect("内部固定交接失败码必须满足领域约束")
}

const fn service_failure(
    kind: TargetedHandoffInboxServiceFailureKind,
) -> TargetedHandoffInboxServiceFailure {
    TargetedHandoffInboxServiceFailure::new(kind)
}
