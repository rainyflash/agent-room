use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use agent_room_application::ports::{Clock, PortFuture};
use agent_room_bridge_core::{
    handoffs::{
        ConsumedTargetedHandoff, TargetedHandoffClaimOutcome, TargetedHandoffInbox,
        TargetedHandoffInboxDependencies, TargetedHandoffInboxFailure,
        TargetedHandoffInboxFailureKind, TargetedHandoffInboxRecordOutcome,
        TargetedHandoffInboxService, TargetedHandoffInboxServiceFailureKind,
        TargetedHandoffQueueFailure, TargetedHandoffQueueFailureKind, TargetedHandoffQueueGateway,
        TargetedHandoffReceipt, TargetedHandoffTarget,
    },
    messages::{
        DownloadedMessageContent, MessageContentReadFailure, MessageContentReadGateway,
        MessageContentReadRequest,
    },
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        HandoffContentReference, HandoffPermission, HandoffPermissions, HandoffPurpose,
        HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields, TargetedHandoffStatus,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

#[tokio::test]
async fn 后台领取只保存元数据且不会提前打开正文() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::with_claim(fixture.handoff.clone()));
    let inbox = Arc::new(内存收件箱::default());
    let content = Arc::new(测试正文::new(fixture.downloaded()));
    let service = fixture.service(queue, inbox.clone(), content.clone());

    let outcome = service.claim_once().await.expect("领取成功");
    assert!(matches!(outcome, TargetedHandoffClaimOutcome::Stored(_)));
    assert_eq!(content.open_count(), 0);
    assert_eq!(
        inbox.list(fixture.target, 10).await.expect("收件箱可读"),
        vec![fixture.handoff]
    );
}

#[tokio::test]
async fn 本地存在待处理交接时不会重复访问云端队列() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::with_claim(fixture.handoff.clone()));
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let service = fixture.service(
        queue.clone(),
        inbox,
        Arc::new(测试正文::new(fixture.downloaded())),
    );

    let outcome = service.claim_once().await.expect("本地待办可恢复");
    assert!(matches!(outcome, TargetedHandoffClaimOutcome::Pending(_)));
    assert_eq!(queue.claim_count(), 0, "本地在途任务不得触发重复云端领取");
}

#[tokio::test]
async fn 明确消费才下载正文并在云端回执后删除本地记录() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::default());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let content = Arc::new(测试正文::new(fixture.downloaded()));
    let service = fixture.service(queue.clone(), inbox.clone(), content.clone());

    let consumed = service
        .consume(fixture.handoff.fields().id)
        .await
        .expect("交接消费成功");
    assert_consumed(&consumed, fixture.body.as_slice());
    assert_eq!(content.open_count(), 1);
    assert_eq!(queue.receipt_statuses(), vec!["consumed"]);
    assert!(
        inbox
            .list(fixture.target, 10)
            .await
            .expect("收件箱可读")
            .is_empty()
    );
}

#[tokio::test]
async fn 检查待办只返回元数据且不会打开正文或提交回执() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::default());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let content = Arc::new(测试正文::new(fixture.downloaded()));
    let service = fixture.service(queue.clone(), inbox, content.clone());

    let inspected = service
        .inspect_pending(fixture.handoff.fields().id)
        .await
        .expect("待办元数据可检查");

    assert_eq!(inspected, fixture.handoff);
    assert_eq!(content.open_count(), 0);
    assert!(queue.receipt_statuses().is_empty());
}

#[tokio::test]
async fn 正文完整性失败会提交失败回执且绝不返回载荷() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::default());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let mut corrupted = fixture.downloaded();
    corrupted.bytes = Arc::from(b"tampered body".as_slice());
    let service = fixture.service(
        queue.clone(),
        inbox.clone(),
        Arc::new(测试正文::new(corrupted)),
    );

    let failure = service
        .consume(fixture.handoff.fields().id)
        .await
        .expect_err("完整性失败必须拒绝");
    assert_eq!(
        failure.kind(),
        TargetedHandoffInboxServiceFailureKind::IntegrityMismatch
    );
    assert_eq!(queue.receipt_statuses(), vec!["failed"]);
    assert!(
        inbox
            .list(fixture.target, 10)
            .await
            .expect("收件箱可读")
            .is_empty()
    );
}

#[tokio::test]
async fn 文本正文不是_utf8_时会失败关闭且绝不把无效字节送进_ipc() {
    let fixture = Fixture::with_body(vec![0xff, 0xfe, 0xfd]);
    let queue = Arc::new(测试队列::default());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let service = fixture.service(
        queue.clone(),
        inbox.clone(),
        Arc::new(测试正文::new(fixture.downloaded())),
    );

    let failure = service
        .consume(fixture.handoff.fields().id)
        .await
        .expect_err("声明为文本的无效 UTF-8 必须拒绝");

    assert_eq!(
        failure.kind(),
        TargetedHandoffInboxServiceFailureKind::IntegrityMismatch
    );
    assert_eq!(queue.receipt_statuses(), vec!["failed"]);
    assert!(
        inbox
            .list(fixture.target, 10)
            .await
            .expect("收件箱可读")
            .is_empty()
    );
}

#[tokio::test]
async fn 云端回执失败时保留本地元数据并拒绝暴露正文() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::unavailable());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let service = fixture.service(
        queue,
        inbox.clone(),
        Arc::new(测试正文::new(fixture.downloaded())),
    );

    let failure = service
        .consume(fixture.handoff.fields().id)
        .await
        .expect_err("回执失败不得返回正文");
    assert_eq!(
        failure.kind(),
        TargetedHandoffInboxServiceFailureKind::QueueUnavailable
    );
    assert!(
        inbox
            .find(fixture.target, fixture.handoff.fields().id)
            .await
            .expect("收件箱可读")
            .is_some()
    );
}

#[tokio::test]
async fn 拒绝不下载正文且过期交接从本地清理() {
    let fixture = Fixture::new();
    let queue = Arc::new(测试队列::default());
    let inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let content = Arc::new(测试正文::new(fixture.downloaded()));
    let service = fixture.service(queue.clone(), inbox.clone(), content.clone());
    let declined = service
        .decline(fixture.handoff.fields().id)
        .await
        .expect("拒绝成功");
    assert_eq!(declined.status(), TargetedHandoffStatus::Declined);
    assert_eq!(content.open_count(), 0);
    assert_eq!(queue.receipt_statuses(), vec!["declined"]);

    let expired_inbox = Arc::new(内存收件箱::with(fixture.handoff.clone()));
    let expired_service = TargetedHandoffInboxService::new(TargetedHandoffInboxDependencies {
        target: fixture.target,
        queue: Arc::new(测试队列::default()),
        inbox: expired_inbox.clone(),
        content,
        clock: Arc::new(固定时钟(time(5_000))),
    });
    let failure = expired_service
        .consume(fixture.handoff.fields().id)
        .await
        .expect_err("到期边界必须拒绝");
    assert_eq!(
        failure.kind(),
        TargetedHandoffInboxServiceFailureKind::Expired
    );
    assert!(
        expired_inbox
            .list(fixture.target, 10)
            .await
            .expect("收件箱可读")
            .is_empty()
    );
}

#[derive(Default)]
struct 测试队列 {
    claim: Mutex<Option<TargetedHandoff>>,
    receipts: Mutex<Vec<&'static str>>,
    claims: AtomicU32,
    unavailable: bool,
}

impl 测试队列 {
    fn with_claim(handoff: TargetedHandoff) -> Self {
        Self {
            claim: Mutex::new(Some(handoff)),
            ..Self::default()
        }
    }

    const fn unavailable() -> Self {
        Self {
            claim: Mutex::new(None),
            receipts: Mutex::new(Vec::new()),
            claims: AtomicU32::new(0),
            unavailable: true,
        }
    }

    fn receipt_statuses(&self) -> Vec<&'static str> {
        self.receipts.lock().expect("回执锁可用").clone()
    }

    fn claim_count(&self) -> u32 {
        self.claims.load(Ordering::SeqCst)
    }
}

impl TargetedHandoffQueueGateway for 测试队列 {
    fn claim_next(
        &self,
        _target: TargetedHandoffTarget,
    ) -> PortFuture<'_, Result<Option<TargetedHandoff>, TargetedHandoffQueueFailure>> {
        Box::pin(async move {
            self.claims.fetch_add(1, Ordering::SeqCst);
            if self.unavailable {
                return Err(queue_unavailable());
            }
            Ok(self.claim.lock().expect("领取锁可用").take())
        })
    }

    fn record_receipt<'a>(
        &'a self,
        _target: TargetedHandoffTarget,
        _handoff_id: HandoffId,
        receipt: &'a TargetedHandoffReceipt,
    ) -> PortFuture<'a, Result<TargetedHandoff, TargetedHandoffQueueFailure>> {
        Box::pin(async move {
            if self.unavailable {
                return Err(queue_unavailable());
            }
            self.receipts
                .lock()
                .expect("回执锁可用")
                .push(receipt.status());
            let mut handoff = Fixture::new().handoff;
            match receipt {
                TargetedHandoffReceipt::Consumed => {
                    handoff.consume(time(1_200)).expect("消费状态有效");
                }
                TargetedHandoffReceipt::Declined(code) => {
                    handoff
                        .decline(code.clone(), time(1_200))
                        .expect("拒绝状态有效");
                }
                TargetedHandoffReceipt::Failed(code) => {
                    handoff
                        .fail(code.clone(), time(1_200))
                        .expect("失败状态有效");
                }
            }
            Ok(handoff)
        })
    }
}

#[derive(Default)]
struct 内存收件箱 {
    entries: Mutex<BTreeMap<HandoffId, TargetedHandoff>>,
}

impl 内存收件箱 {
    fn with(handoff: TargetedHandoff) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::from([(handoff.fields().id, handoff)])),
        }
    }
}

impl TargetedHandoffInbox for 内存收件箱 {
    fn accept<'a>(
        &'a self,
        handoff: &'a TargetedHandoff,
    ) -> PortFuture<'a, Result<TargetedHandoffInboxRecordOutcome, TargetedHandoffInboxFailure>>
    {
        Box::pin(async move {
            let mut entries = self.entries.lock().expect("收件箱锁可用");
            if let Some(existing) = entries.get(&handoff.fields().id) {
                if existing == handoff {
                    return Ok(TargetedHandoffInboxRecordOutcome::Existing(
                        existing.clone(),
                    ));
                }
                return Err(inbox_failure(TargetedHandoffInboxFailureKind::Conflict));
            }
            entries.insert(handoff.fields().id, handoff.clone());
            Ok(TargetedHandoffInboxRecordOutcome::Created(handoff.clone()))
        })
    }

    fn find(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<TargetedHandoff>, TargetedHandoffInboxFailure>> {
        Box::pin(async move {
            Ok(self
                .entries
                .lock()
                .expect("收件箱锁可用")
                .get(&handoff_id)
                .filter(|handoff| matches_target(handoff, target))
                .cloned())
        })
    }

    fn list(
        &self,
        target: TargetedHandoffTarget,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<TargetedHandoff>, TargetedHandoffInboxFailure>> {
        Box::pin(async move {
            Ok(self
                .entries
                .lock()
                .expect("收件箱锁可用")
                .values()
                .filter(|handoff| matches_target(handoff, target))
                .take(usize::from(limit))
                .cloned()
                .collect())
        })
    }

    fn remove(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<bool, TargetedHandoffInboxFailure>> {
        Box::pin(async move {
            let mut entries = self.entries.lock().expect("收件箱锁可用");
            if entries
                .get(&handoff_id)
                .is_some_and(|handoff| matches_target(handoff, target))
            {
                entries.remove(&handoff_id);
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
}

struct 测试正文 {
    response: DownloadedMessageContent,
    opens: Mutex<u32>,
}

impl 测试正文 {
    fn new(response: DownloadedMessageContent) -> Self {
        Self {
            response,
            opens: Mutex::new(0),
        }
    }

    fn open_count(&self) -> u32 {
        *self.opens.lock().expect("正文计数锁可用")
    }
}

impl MessageContentReadGateway for 测试正文 {
    fn open<'a>(
        &'a self,
        _request: &'a MessageContentReadRequest,
    ) -> PortFuture<'a, Result<DownloadedMessageContent, MessageContentReadFailure>> {
        Box::pin(async move {
            *self.opens.lock().expect("正文计数锁可用") += 1;
            Ok(self.response.clone())
        })
    }
}

struct 固定时钟(UtcMillis);

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        self.0
    }
}

struct Fixture {
    target: TargetedHandoffTarget,
    handoff: TargetedHandoff,
    body: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_body(b"trusted context".to_vec())
    }

    fn with_body(body: Vec<u8>) -> Self {
        let target = TargetedHandoffTarget {
            agent_id: AgentId::from_uuid(Uuid::now_v7()),
            instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        };
        let digest = Sha256Digest::from_bytes(Sha256::digest(&body).into());
        let mut handoff = TargetedHandoff::queue(TargetedHandoffFields {
            id: HandoffId::from_uuid(Uuid::now_v7()),
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            source_room_id: MatrixRoomReference::new("!lobby:matrix.test").expect("房间引用有效"),
            source_event_id: HandoffSourceEventId::new("$event-123").expect("事件引用有效"),
            source_message_id: MessageId::from_uuid(Uuid::now_v7()),
            target_agent_id: target.agent_id,
            target_instance_id: target.instance_id,
            content: HandoffContentReference::new(
                ContentId::from_uuid(Uuid::now_v7()),
                digest,
                ContentByteLength::new(u64::try_from(body.len()).expect("长度可转换"))
                    .expect("正文长度有效"),
                ContentMediaType::new("text/plain").expect("媒体类型有效"),
            ),
            permissions: HandoffPermissions::new([HandoffPermission::ReadText]).expect("权限有效"),
            purpose: HandoffPurpose::Inspect,
            created_at: time(1_000),
            expires_at: time(5_000),
        })
        .expect("排队交接有效");
        handoff.mark_delivered(time(1_100)).expect("交付有效");
        Self {
            target,
            handoff,
            body,
        }
    }

    fn downloaded(&self) -> DownloadedMessageContent {
        DownloadedMessageContent {
            bytes: Arc::from(self.body.clone()),
            digest: self.handoff.fields().content.digest(),
            byte_length: self.handoff.fields().content.byte_length(),
            media_type: self.handoff.fields().content.media_type().clone(),
        }
    }

    fn service(
        &self,
        queue: Arc<dyn TargetedHandoffQueueGateway>,
        inbox: Arc<dyn TargetedHandoffInbox>,
        content: Arc<dyn MessageContentReadGateway>,
    ) -> TargetedHandoffInboxService {
        TargetedHandoffInboxService::new(TargetedHandoffInboxDependencies {
            target: self.target,
            queue,
            inbox,
            content,
            clock: Arc::new(固定时钟(time(1_150))),
        })
    }
}

fn assert_consumed(consumed: &ConsumedTargetedHandoff, expected: &[u8]) {
    assert_eq!(consumed.body().as_ref(), expected);
    assert_eq!(
        consumed.handoff().status(),
        TargetedHandoffStatus::Delivered
    );
}

fn matches_target(handoff: &TargetedHandoff, target: TargetedHandoffTarget) -> bool {
    handoff.fields().target_agent_id == target.agent_id
        && handoff.fields().target_instance_id == target.instance_id
}

const fn queue_unavailable() -> TargetedHandoffQueueFailure {
    TargetedHandoffQueueFailure::new(TargetedHandoffQueueFailureKind::Unavailable)
}

const fn inbox_failure(kind: TargetedHandoffInboxFailureKind) -> TargetedHandoffInboxFailure {
    TargetedHandoffInboxFailure::new(kind)
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
