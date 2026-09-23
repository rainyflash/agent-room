use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use agent_room_application::ports::{
    DeviceProofVerifier, DeviceSignature, MatrixBackfillPage, MatrixBackfillRequest,
    MatrixBackfillToken, MatrixEventId, MatrixEventType, MatrixFailure, MatrixFailureKind,
    MatrixOperation, MatrixResult, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixTransactionId, MatrixUserId,
    PortFuture,
};
use agent_room_bridge_core::messages::{
    MessageAuthenticationDecision, MessageAuthenticationFailure, MessageAuthenticationFailureKind,
    MessageBackfillBatch, MessageBackfillSource, MessageEventAuthenticator, MessageProjectionBatch,
    MessageProjectionMutation, MessageProjectionStoreFailure, MessageProjectionStoreFailureKind,
    MessageStoreFailure, MessageStoreFailureKind, MessageSubmissionClaim,
    MessageSubmissionClaimOutcome, MessageSubmissionFingerprint, MessageSubmissionKind,
    MessageSubmissionRecord, MessageSubmissionRepository, MessageSubmissionState,
    MessageSyncDependencies, MessageSyncFailureKind, MessageSyncIssueReason, MessageSyncService,
    MessageTimelineProjectionStore, PendingTimelineGap, ProjectedActorInstanceVerification,
};
use agent_room_domain::{
    devices::DevicePublicSigningKey,
    ids::{AgentId, AgentInstanceId, MessageSubmissionId},
    time::UtcMillis,
};
use agent_room_identity_adapter::{Ed25519DeviceProofVerifier, Ed25519DeviceSigningKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use uuid::Uuid;

const ACTOR_AGENT_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3";
const ACTOR_INSTANCE_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4";
const ACTOR_MATRIX_ID: &str = "@agent:matrix.test";

struct 验签认证器 {
    public_key: DevicePublicSigningKey,
    fail: AtomicBool,
    historical_revoked: AtomicBool,
}

impl MessageEventAuthenticator for 验签认证器 {
    fn authenticate<'a>(
        &'a self,
        _agent_id: AgentId,
        _instance_id: AgentInstanceId,
        _origin_server_timestamp: UtcMillis,
        canonical_event: &'a [u8],
        signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<MessageAuthenticationDecision, MessageAuthenticationFailure>> {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(MessageAuthenticationFailure::new(
                    MessageAuthenticationFailureKind::Unavailable,
                ))
            });
        }
        let decision =
            if Ed25519DeviceProofVerifier.verify(&self.public_key, canonical_event, signature) {
                if self.historical_revoked.load(Ordering::SeqCst) {
                    MessageAuthenticationDecision::TrustedHistoricalRevoked
                } else {
                    MessageAuthenticationDecision::Trusted
                }
            } else {
                MessageAuthenticationDecision::InvalidSignature
            };
        Box::pin(async move { Ok(decision) })
    }
}

#[derive(Default)]
struct 记录投影存储 {
    batches: Mutex<Vec<MessageProjectionBatch>>,
    backfills: Mutex<Vec<MessageBackfillBatch>>,
    fail: AtomicBool,
}

impl 记录投影存储 {
    /// 同步与补缺口写下的全部投影事件（按写入顺序）。
    fn projected(&self) -> Vec<(MatrixRoomId, MatrixEventId)> {
        let mut events = Vec::new();
        for batch in self.batches.lock().expect("投影记录锁可用").iter() {
            for mutation in batch.mutations() {
                events.push((mutation.room_id().clone(), mutation.event_id().clone()));
            }
        }
        for batch in self.backfills.lock().expect("补缺口记录锁可用").iter() {
            for mutation in batch.mutations() {
                events.push((mutation.room_id().clone(), mutation.event_id().clone()));
            }
        }
        events
    }
}

impl MessageTimelineProjectionStore for 记录投影存储 {
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(MessageProjectionStoreFailure::new(
                    MessageProjectionStoreFailureKind::Unavailable,
                ))
            });
        }
        self.batches
            .lock()
            .expect("投影记录锁可用")
            .push(batch.clone());
        Box::pin(async { Ok(()) })
    }

    fn sync_cursor(
        &self,
    ) -> PortFuture<'_, Result<Option<MatrixSyncToken>, MessageProjectionStoreFailure>> {
        let cursor = self
            .batches
            .lock()
            .expect("投影记录锁可用")
            .last()
            .map(|batch| batch.next_batch().clone());
        Box::pin(async move { Ok(cursor) })
    }

    fn room_has_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<bool, MessageProjectionStoreFailure>> {
        let known = self.projected().iter().any(|(room, _)| room == room_id);
        Box::pin(async move { Ok(known) })
    }

    fn known_events<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_ids: &'a [MatrixEventId],
    ) -> PortFuture<'a, Result<Vec<MatrixEventId>, MessageProjectionStoreFailure>> {
        let projected = self.projected();
        let known = event_ids
            .iter()
            .filter(|id| {
                projected
                    .iter()
                    .any(|(room, event)| room == room_id && event == *id)
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(known) })
    }

    fn pending_gaps(
        &self,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<PendingTimelineGap>, MessageProjectionStoreFailure>> {
        let resolved = self
            .backfills
            .lock()
            .expect("补缺口记录锁可用")
            .iter()
            .map(|batch| batch.gap().clone())
            .collect::<Vec<_>>();
        let pending = self
            .batches
            .lock()
            .expect("投影记录锁可用")
            .iter()
            .flat_map(|batch| {
                batch.gaps().iter().filter_map(|gap| {
                    gap.previous_batch
                        .clone()
                        .map(|previous_batch| PendingTimelineGap {
                            sync_token: batch.next_batch().clone(),
                            room_id: gap.room_id.clone(),
                            previous_batch,
                        })
                })
            })
            .filter(|gap| !resolved.contains(gap))
            .take(usize::from(limit))
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn apply_backfill<'a>(
        &'a self,
        batch: &'a MessageBackfillBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        self.backfills
            .lock()
            .expect("补缺口记录锁可用")
            .push(batch.clone());
        Box::pin(async { Ok(()) })
    }
}

/// 按预先排好的页往回翻时间线，并记下每次从哪个令牌开始翻。
#[derive(Default)]
struct 往回翻页源 {
    pages: Mutex<std::collections::VecDeque<MatrixResult<MatrixBackfillPage>>>,
    requests: Mutex<Vec<String>>,
}

impl MessageBackfillSource for 往回翻页源 {
    fn backfill_page<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        self.requests
            .lock()
            .expect("请求记录锁可用")
            .push(request.from().as_str().to_owned());
        let page = self
            .pages
            .lock()
            .expect("页锁可用")
            .pop_front()
            .unwrap_or_else(|| {
                Err(MatrixFailure::new(
                    MatrixOperation::Backfill,
                    MatrixFailureKind::NotFound,
                ))
            });
        Box::pin(async move { page })
    }
}

fn backfill_token(value: &str) -> MatrixBackfillToken {
    MatrixBackfillToken::new(value).expect("往回翻页令牌有效")
}

fn human_chat(event_id: &str, text: &str) -> MatrixTimelineEvent {
    let mut payload = preview_payload(
        Uuid::now_v7(),
        room_id().as_str(),
        "2026-09-05T12:00:00.000Z",
        None,
    );
    payload["schemaVersion"] = json!("2.0");
    payload["eventType"] = json!("io.github.rainyflash.agentroom.message.preview.v2");
    payload["actor"] = json!({"kind": "human", "principalId": Uuid::now_v7(), "displayName": "小雨", "matrixUserId": ACTOR_MATRIX_ID});
    payload["preview"]["contentType"] = json!("text/plain");
    payload["content"]["mediaType"] = json!("text/plain");
    payload["preview"]["conversation"] = json!({"text": text, "mentions": []});
    timeline_event(
        event_id,
        "io.github.rainyflash.agentroom.message.preview.v2",
        payload,
        None,
    )
}

fn limited_sync(
    token: &str,
    previous_batch: &str,
    events: Vec<MatrixTimelineEvent>,
) -> MatrixSyncBatch {
    MatrixSyncBatch::new(
        MatrixSyncToken::new(token).expect("游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            true,
            Some(backfill_token(previous_batch)),
            events,
            Vec::new(),
        )],
    )
}

fn event_ids(events: &[(MatrixRoomId, MatrixEventId)]) -> Vec<&str> {
    events.iter().map(|(_, event)| event.as_str()).collect()
}

#[tokio::test]
async fn 离线期间漏掉的消息按时间先后补回_接上已有消息就停() {
    let fixture = 测试夹具::new();
    let service = fixture.service();
    let source = 往回翻页源::default();
    // 第一次同步到这个房间：本来就只取最近一段，不算漏消息，不记缺口。
    let first = service
        .process(&limited_sync("s1", "p0", vec![human_chat("$ev-a", "一")]))
        .await
        .expect("可同步");
    assert_eq!(first.timeline_gaps, 0);
    // 离线太久：这次同步只带最近一条，中间几条要往回补。
    let second = service
        .process(&limited_sync("s2", "p1", vec![human_chat("$ev-e", "五")]))
        .await
        .expect("可同步");
    assert_eq!(second.timeline_gaps, 1);
    source.pages.lock().expect("页锁可用").extend([
        Ok(MatrixBackfillPage::new(
            backfill_token("p1"),
            Some(backfill_token("p2")),
            vec![human_chat("$ev-d", "四"), human_chat("$ev-c", "三")],
        )),
        Ok(MatrixBackfillPage::new(
            backfill_token("p2"),
            Some(backfill_token("p3")),
            vec![
                human_chat("$ev-b", "二"),
                human_chat("$ev-a", "一"),
                human_chat("$ev-z", "更早"),
            ],
        )),
    ]);
    let outcome = service.fill_gaps(&source).await.expect("可补缺口");
    assert_eq!(outcome.filled_gaps, 1);
    assert_eq!(outcome.accepted_events, 3);
    assert_eq!(outcome.truncated_gaps, 0);
    assert_eq!(
        *source.requests.lock().expect("请求记录锁可用"),
        ["p1", "p2"]
    );
    // 补回的消息按时间先后排在已收到的之后；碰到已有的 $a 就停，更早的不会重复写入。
    assert_eq!(
        event_ids(&fixture.projections.projected()),
        ["$ev-a", "$ev-e", "$ev-b", "$ev-c", "$ev-d"]
    );
    // 缺口已结清：再来一轮什么都不做。
    let again = service.fill_gaps(&source).await.expect("可补缺口");
    assert_eq!(again.filled_gaps, 0);
    assert_eq!(source.requests.lock().expect("请求记录锁可用").len(), 2);
}

#[tokio::test]
async fn 暂时读不到的缺口留到下一轮_不让读的放弃_翻到上限就停() {
    let fixture = 测试夹具::new();
    let service = fixture.service();
    let source = 往回翻页源::default();
    service
        .process(&limited_sync("s1", "p0", vec![human_chat("$ev-a", "一")]))
        .await
        .expect("可同步");
    service
        .process(&limited_sync("s2", "p1", vec![human_chat("$ev-z", "最新")]))
        .await
        .expect("可同步");
    source
        .pages
        .lock()
        .expect("页锁可用")
        .push_back(Err(MatrixFailure::new(
            MatrixOperation::Backfill,
            MatrixFailureKind::Timeout,
        )));
    let deferred = service.fill_gaps(&source).await.expect("可补缺口");
    assert_eq!(deferred.deferred_gaps, 1);
    assert_eq!(deferred.filled_gaps, 0);
    // 下一轮：一直翻不到已有消息，翻满五页就停，只补回这五页。
    for page in 0..5 {
        source
            .pages
            .lock()
            .expect("页锁可用")
            .push_back(Ok(MatrixBackfillPage::new(
                backfill_token(&format!("q{page}")),
                Some(backfill_token(&format!("q{}", page + 1))),
                vec![human_chat(&format!("$gap-{page}"), "中间")],
            )));
    }
    let truncated = service.fill_gaps(&source).await.expect("可补缺口");
    assert_eq!(truncated.filled_gaps, 1);
    assert_eq!(truncated.truncated_gaps, 1);
    assert_eq!(truncated.accepted_events, 5);
    assert_eq!(
        event_ids(&fixture.projections.projected())[2..],
        ["$gap-4", "$gap-3", "$gap-2", "$gap-1", "$gap-0"]
    );
    // 房间不让读了（例如已离开）：放弃这段缺口，不再每轮重试。
    service
        .process(&limited_sync(
            "s3",
            "r1",
            vec![human_chat("$ev-y", "又来一条")],
        ))
        .await
        .expect("可同步");
    source
        .pages
        .lock()
        .expect("页锁可用")
        .push_back(Err(MatrixFailure::new(
            MatrixOperation::Backfill,
            MatrixFailureKind::Forbidden,
        )));
    let given_up = service.fill_gaps(&source).await.expect("可补缺口");
    assert_eq!(given_up.filled_gaps, 1);
    assert_eq!(given_up.accepted_events, 0);
    assert_eq!(
        service
            .fill_gaps(&source)
            .await
            .expect("可补缺口")
            .filled_gaps,
        0
    );
}

#[derive(Default)]
struct 记录对账仓库 {
    observations: AtomicUsize,
}

impl MessageSubmissionRepository for 记录对账仓库 {
    fn claim<'a>(
        &'a self,
        _claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>> {
        Box::pin(async { Err(MessageStoreFailure::new(MessageStoreFailureKind::NotFound)) })
    }

    fn mark_submit_unknown(
        &self,
        _submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        unsupported_submission_operation()
    }

    fn mark_accepted<'a>(
        &'a self,
        _submission_id: MessageSubmissionId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        unsupported_submission_operation()
    }

    fn mark_bound(
        &self,
        _submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        unsupported_submission_operation()
    }

    fn observe_transaction<'a>(
        &'a self,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        let record = MessageSubmissionRecord {
            submission_id: MessageSubmissionId::from_uuid(Uuid::now_v7()),
            kind: MessageSubmissionKind::Preview,
            fingerprint: MessageSubmissionFingerprint::from_bytes([1; 32]),
            transaction_id: transaction_id.clone(),
            state: MessageSubmissionState::Accepted,
            event_id: Some(event_id.clone()),
        };
        Box::pin(async move { Ok(Some(record)) })
    }
}

fn unsupported_submission_operation<'a>()
-> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
    Box::pin(async { Err(MessageStoreFailure::new(MessageStoreFailureKind::NotFound)) })
}

struct 测试夹具 {
    signing_key: Ed25519DeviceSigningKey,
    authenticator: Arc<验签认证器>,
    projections: Arc<记录投影存储>,
    submissions: Arc<记录对账仓库>,
}

impl 测试夹具 {
    fn new() -> Self {
        let signing_key = Ed25519DeviceSigningKey::generate().expect("测试私钥可生成");
        let public_key = signing_key.public_key().expect("测试公钥可读取");
        Self {
            signing_key,
            authenticator: Arc::new(验签认证器 {
                public_key,
                fail: AtomicBool::new(false),
                historical_revoked: AtomicBool::new(false),
            }),
            projections: Arc::new(记录投影存储::default()),
            submissions: Arc::new(记录对账仓库::default()),
        }
    }

    fn service(&self) -> MessageSyncService {
        MessageSyncService::new(MessageSyncDependencies {
            authenticator: self.authenticator.clone(),
            projections: self.projections.clone(),
            submissions: self.submissions.clone(),
        })
    }
}

#[tokio::test]
async fn 撤销前的可信历史事件进入投影并携带已撤销实例标记() {
    let fixture = 测试夹具::new();
    fixture
        .authenticator
        .historical_revoked
        .store(true, Ordering::SeqCst);
    let event = signed_timeline_event(
        &fixture.signing_key,
        "$historical:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        preview_payload(
            Uuid::now_v7(),
            room_id().as_str(),
            "2026-08-24T12:00:00.000Z",
            None,
        ),
        None,
    );
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("historical-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![event],
            Vec::new(),
        )],
    );

    let outcome = fixture
        .service()
        .process(&sync)
        .await
        .expect("撤销前历史事件可投影");

    assert_eq!(outcome.accepted_events, 1);
    assert_eq!(outcome.isolated_events, 0);
    let batches = fixture.projections.batches.lock().expect("投影记录锁可用");
    let MessageProjectionMutation::Preview(preview) = &batches[0].mutations()[0] else {
        panic!("历史事件应是预览投影");
    };
    assert_eq!(
        preview.actor.instance_verification(),
        ProjectedActorInstanceVerification::RevokedAfterEvent
    );
}

#[tokio::test]
async fn 同步按_matrix_顺序投影并逐条隔离坏事件() {
    let fixture = 测试夹具::new();
    let message_id = Uuid::now_v7();
    let reply_target = Uuid::now_v7();
    let sync = mixed_sync(&fixture, message_id, reply_target);

    let outcome = fixture
        .service()
        .process(&sync)
        .await
        .expect("坏事件不拖垮批次");
    assert_eq!(outcome.accepted_events, 2);
    assert_eq!(outcome.isolated_events, 2);
    // 第一次同步到这个房间，本来就只取最近一段：不算漏了消息，不记缺口。
    assert_eq!(outcome.timeline_gaps, 0);
    assert_eq!(outcome.reconciled_submissions, 1);
    assert_eq!(fixture.submissions.observations.load(Ordering::SeqCst), 1);

    let batches = fixture.projections.batches.lock().expect("投影记录锁可用");
    let batch = &batches[0];
    assert_eq!(batch.next_batch().as_str(), "next-message-sync");
    assert_eq!(batch.mutations().len(), 2);
    assert!(matches!(
        batch.mutations()[0],
        MessageProjectionMutation::Preview(_)
    ));
    assert!(matches!(
        batch.mutations()[1],
        MessageProjectionMutation::Revision(_)
    ));
    let MessageProjectionMutation::Preview(preview) = &batch.mutations()[0] else {
        panic!("首个投影必须是预览");
    };
    let MessageProjectionMutation::Revision(revision) = &batch.mutations()[1] else {
        panic!("第二个投影必须是修订");
    };
    assert!(preview.created_at > revision.created_at);
    assert_eq!(
        preview.relation,
        Some(agent_room_domain::messages::MessageRelation::ReplyTo(
            agent_room_domain::ids::MessageId::from_uuid(reply_target)
        ))
    );
    assert_eq!(
        batch
            .issues()
            .iter()
            .map(|issue| issue.reason)
            .collect::<Vec<_>>(),
        vec![
            MessageSyncIssueReason::RoomMismatch,
            MessageSyncIssueReason::InvalidSignature
        ]
    );
}

#[tokio::test]
async fn 客户端正文密钥只接受来自_matrix_端到端加密事件() {
    let fixture = 测试夹具::new();
    let encrypted_id = Uuid::now_v7();
    let mut encrypted_payload = preview_payload(
        encrypted_id,
        room_id().as_str(),
        "2026-08-24T12:00:00.000Z",
        None,
    );
    encrypted_payload["content"]["encryption"] = json!({
        "algorithm": "io.github.rainyflash.agentroom.content.aes-256-gcm.v1",
        "contextId": encrypted_id,
        "keyBase64Url": URL_SAFE_NO_PAD.encode([7_u8; 32]),
        "nonceBase64Url": URL_SAFE_NO_PAD.encode([8_u8; 12]),
        "plaintextSizeBytes": 112
    });
    let mut leaked_payload = encrypted_payload.clone();
    leaked_payload["id"] = json!(Uuid::now_v7());
    leaked_payload["content"]["encryption"]["contextId"] = leaked_payload["id"].clone();
    let encrypted = signed_timeline_event(
        &fixture.signing_key,
        "$encrypted:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        encrypted_payload,
        None,
    )
    .with_trusted_end_to_end_encryption();
    let leaked = signed_timeline_event(
        &fixture.signing_key,
        "$plaintext:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        leaked_payload,
        None,
    );
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("encrypted-content-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![encrypted, leaked],
            Vec::new(),
        )],
    );

    let outcome = fixture
        .service()
        .process(&sync)
        .await
        .expect("批次应可隔离泄漏事件");

    assert_eq!(outcome.accepted_events, 1);
    assert_eq!(outcome.isolated_events, 1);
    let batches = fixture.projections.batches.lock().expect("投影记录锁可用");
    let MessageProjectionMutation::Preview(preview) = &batches[0].mutations()[0] else {
        panic!("加密事件应进入预览投影");
    };
    let encryption = preview
        .content
        .client_encryption()
        .expect("投影必须保留解密材料");
    assert_eq!(encryption.context_id().as_uuid(), encrypted_id);
    assert_eq!(encryption.key(), &[7_u8; 32]);
    assert_eq!(
        batches[0].issues()[0].reason,
        MessageSyncIssueReason::InvalidEnvelope
    );
}

#[tokio::test]
async fn 未可信设备的加密房间消息在应用验签前被隔离() {
    let fixture = 测试夹具::new();
    let event = signed_timeline_event(
        &fixture.signing_key,
        "$untrusted-encrypted:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        preview_payload(
            Uuid::now_v7(),
            room_id().as_str(),
            "2026-08-24T12:00:00.000Z",
            None,
        ),
        None,
    )
    .with_untrusted_end_to_end_encryption();
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("untrusted-encrypted-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![event],
            Vec::new(),
        )],
    );

    let outcome = fixture
        .service()
        .process(&sync)
        .await
        .expect("坏事件应被隔离");

    assert_eq!(outcome.accepted_events, 0);
    assert_eq!(outcome.isolated_events, 1);
    let batches = fixture.projections.batches.lock().expect("投影记录锁可用");
    assert_eq!(
        batches[0].issues()[0].reason,
        MessageSyncIssueReason::UntrustedEncryptedSender
    );
}

#[tokio::test]
async fn 解不开的加密事件留下记录而不是悄悄跳过() {
    let fixture = 测试夹具::new();
    let event = timeline_event(
        "$undecryptable:matrix.test",
        "m.room.encrypted",
        json!({
            "algorithm": "m.megolm.v1.aes-sha2",
            "ciphertext": "opaque",
            "session_id": "withheld-session",
        }),
        None,
    );
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("undecryptable-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![event],
            Vec::new(),
        )],
    );

    let outcome = fixture
        .service()
        .process(&sync)
        .await
        .expect("解不开的事件应被记录");

    assert_eq!(outcome.accepted_events, 0);
    assert_eq!(outcome.isolated_events, 1);
    let batches = fixture.projections.batches.lock().expect("投影记录锁可用");
    assert_eq!(
        batches[0].issues()[0].reason,
        MessageSyncIssueReason::Undecryptable
    );
}

#[tokio::test]
async fn 处理过的批次留下游标供重启后接着同步() {
    let fixture = 测试夹具::new();
    let service = fixture.service();
    assert_eq!(service.stored_cursor().await.expect("游标可读"), None);
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("resume-here").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            Vec::new(),
            Vec::new(),
        )],
    );
    service.process(&sync).await.expect("空批次可处理");
    assert_eq!(
        service
            .stored_cursor()
            .await
            .expect("游标可读")
            .as_ref()
            .map(MatrixSyncToken::as_str),
        Some("resume-here")
    );
}

fn mixed_sync(fixture: &测试夹具, message_id: Uuid, reply_target: Uuid) -> MatrixSyncBatch {
    let room_id = room_id();
    let preview = signed_timeline_event(
        &fixture.signing_key,
        "$preview:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        preview_payload(
            message_id,
            room_id.as_str(),
            "2099-01-01T00:00:00.000Z",
            Some(reply_target),
        ),
        Some(MatrixTransactionId::new("outgoing-unknown").expect("事务标识有效")),
    );
    let mut wrong_room_payload = preview_payload(
        Uuid::now_v7(),
        "!other:matrix.test",
        "2026-08-24T12:00:00.000Z",
        None,
    );
    sign_payload(&fixture.signing_key, &mut wrong_room_payload);
    let wrong_room = timeline_event(
        "$wrong-room:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        wrong_room_payload,
        None,
    );
    let mut bad_signature_payload = preview_payload(
        Uuid::now_v7(),
        room_id.as_str(),
        "2026-08-24T12:00:01.000Z",
        None,
    );
    sign_payload(&fixture.signing_key, &mut bad_signature_payload);
    bad_signature_payload["preview"]["summary"] = json!("签名后被篡改");
    let bad_signature = timeline_event(
        "$bad-signature:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        bad_signature_payload,
        None,
    );
    let revision = signed_timeline_event(
        &fixture.signing_key,
        "$revision:matrix.test",
        "io.github.rainyflash.agentroom.message.revision.v1",
        revision_payload(
            Uuid::now_v7(),
            message_id,
            room_id.as_str(),
            "2000-01-01T00:00:00.000Z",
        ),
        None,
    );
    let unrelated = MatrixTimelineEvent::new(
        Some(MatrixEventId::new("$unrelated:matrix.test").expect("事件标识有效")),
        Some(MatrixUserId::new(ACTOR_MATRIX_ID).expect("用户标识有效")),
        MatrixEventType::new("m.room.message").expect("事件类型有效"),
        None,
        None,
        Some(5),
        json!({"body": "不会进入自定义消息投影"}),
    )
    .expect("普通事件有效");
    MatrixSyncBatch::new(
        MatrixSyncToken::new("next-message-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id,
            MatrixRoomSyncKind::Joined,
            true,
            Some(MatrixBackfillToken::new("previous-page").expect("历史游标有效")),
            vec![preview, wrong_room, bad_signature, revision, unrelated],
            Vec::new(),
        )],
    )
}

#[tokio::test]
async fn 验签依赖不可用时绝不推进同步游标() {
    let fixture = 测试夹具::new();
    fixture.authenticator.fail.store(true, Ordering::SeqCst);
    let room_id = room_id();
    let event = signed_timeline_event(
        &fixture.signing_key,
        "$preview:matrix.test",
        "io.github.rainyflash.agentroom.message.preview.v1",
        preview_payload(
            Uuid::now_v7(),
            room_id.as_str(),
            "2026-08-24T12:00:00.000Z",
            None,
        ),
        None,
    );
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("must-not-advance").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id,
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![event],
            Vec::new(),
        )],
    );

    let failure = fixture
        .service()
        .process(&sync)
        .await
        .expect_err("认证依赖失败必须中止批次");
    assert_eq!(failure.kind(), MessageSyncFailureKind::Authentication);
    assert!(
        fixture
            .projections
            .batches
            .lock()
            .expect("投影记录锁可用")
            .is_empty()
    );
}

fn preview_payload(
    message_id: Uuid,
    room_id: &str,
    created_at: &str,
    reply_target: Option<Uuid>,
) -> Value {
    let mut payload = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.preview.v1",
        "id": message_id,
        "createdAt": created_at,
        "actor": actor(),
        "correlationId": Uuid::now_v7(),
        "roomId": room_id,
        "preview": {
            "title": "同步完成",
            "summary": "这里只同步结构化摘要",
            "contentType": "text/markdown",
            "language": "zh-CN",
            "sensitivity": "normal",
            "riskFlags": ["untrusted_instructions"]
        },
        "content": {
            "contentId": Uuid::now_v7(),
            "digestSha256": "11".repeat(32),
            "sizeBytes": 128,
            "mediaType": "text/markdown",
            "fetchMode": "on_demand"
        }
    });
    if let Some(reply_target) = reply_target {
        payload["relation"] = json!({
            "kind": "reply",
            "targetMessageId": reply_target
        });
    }
    payload
}

fn revision_payload(
    revision_id: Uuid,
    target_message_id: Uuid,
    room_id: &str,
    created_at: &str,
) -> Value {
    json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.revision.v1",
        "id": revision_id,
        "createdAt": created_at,
        "actor": actor(),
        "correlationId": Uuid::now_v7(),
        "roomId": room_id,
        "targetMessageId": target_message_id,
        "kind": "replace",
        "preview": {
            "title": "修订完成",
            "summary": "这个 createdAt 更早，但同步顺序更晚",
            "contentType": "text/markdown",
            "language": "zh-CN",
            "sensitivity": "normal",
            "riskFlags": []
        },
        "content": {
            "contentId": Uuid::now_v7(),
            "digestSha256": "22".repeat(32),
            "sizeBytes": 96,
            "mediaType": "text/markdown",
            "fetchMode": "on_demand"
        }
    })
}

fn actor() -> Value {
    json!({
        "agent": {
            "agentId": ACTOR_AGENT_ID,
            "displayName": "同步 Agent",
            "matrixUserId": ACTOR_MATRIX_ID,
            "avatarUrl": "https://example.test/avatar.png"
        },
        "instanceId": ACTOR_INSTANCE_ID,
        "provenance": "autonomous_agent"
    })
}

fn signed_timeline_event(
    signing_key: &Ed25519DeviceSigningKey,
    event_id: &str,
    event_type: &str,
    mut payload: Value,
    transaction_id: Option<MatrixTransactionId>,
) -> MatrixTimelineEvent {
    sign_payload(signing_key, &mut payload);
    timeline_event(event_id, event_type, payload, transaction_id)
}

fn sign_payload(signing_key: &Ed25519DeviceSigningKey, payload: &mut Value) {
    let canonical = serde_jcs::to_vec(payload).expect("事件可规范化");
    let signature = signing_key.sign(&canonical).expect("测试事件可签名");
    payload.as_object_mut().expect("事件是对象").insert(
        "signature".to_owned(),
        Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
    );
}

fn timeline_event(
    event_id: &str,
    event_type: &str,
    payload: Value,
    transaction_id: Option<MatrixTransactionId>,
) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).expect("事件标识有效")),
        Some(MatrixUserId::new(ACTOR_MATRIX_ID).expect("用户标识有效")),
        MatrixEventType::new(event_type).expect("事件类型有效"),
        None,
        transaction_id,
        Some(1_000),
        payload,
    )
    .expect("时间线事件有效")
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}

#[tokio::test]
async fn 人类聊天与新版_agent_消息进入同一投影并隔离伪造主体() {
    let fixture = 测试夹具::new();
    let mut human = preview_payload(
        Uuid::now_v7(),
        room_id().as_str(),
        "2026-09-05T12:00:00.000Z",
        None,
    );
    human["schemaVersion"] = json!("2.0");
    human["eventType"] = json!("io.github.rainyflash.agentroom.message.preview.v2");
    human["actor"] = json!({"kind": "human", "principalId": Uuid::now_v7(), "displayName": "小雨", "matrixUserId": ACTOR_MATRIX_ID});
    human["preview"]["contentType"] = json!("text/plain");
    human["content"]["mediaType"] = json!("text/plain");
    human["preview"]["conversation"] =
        json!({"text": "你怎么看？", "mentions": ["@assistant:matrix.test"]});
    let mut forged = human.clone();
    forged["id"] = json!(Uuid::now_v7());
    forged["actor"]["matrixUserId"] = json!("@someone-else:matrix.test");
    let mut invalid_kind = human.clone();
    invalid_kind["id"] = json!(Uuid::now_v7());
    invalid_kind["actor"]["kind"] = json!("agent");
    let mut agent = preview_payload(
        Uuid::now_v7(),
        room_id().as_str(),
        "2026-09-05T12:00:01.000Z",
        None,
    );
    agent["schemaVersion"] = json!("2.0");
    agent["eventType"] = json!("io.github.rainyflash.agentroom.message.preview.v2");
    agent["actor"]["kind"] = json!("agent");
    let unsigned = agent.clone();
    sign_payload(&fixture.signing_key, &mut agent);
    let sync = MatrixSyncBatch::new(
        sync_token_for_chat(),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![
                timeline_event(
                    "$human",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    human,
                    None,
                ),
                timeline_event(
                    "$forged",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    forged,
                    None,
                ),
                timeline_event(
                    "$wrong-kind",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    invalid_kind,
                    None,
                ),
                timeline_event(
                    "$agent-v2",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    agent,
                    None,
                ),
                timeline_event(
                    "$unsigned-agent",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    unsigned,
                    None,
                ),
            ],
            Vec::new(),
        )],
    );
    let result = fixture
        .service()
        .process(&sync)
        .await
        .expect("合法事件可被投影");
    assert_eq!(result.accepted_events, 2);
    assert_eq!(result.isolated_events, 3);
    let batches = fixture.projections.batches.lock().expect("锁可用");
    let MessageProjectionMutation::Preview(message) = &batches[0].mutations()[0] else {
        panic!("应为聊天消息");
    };
    assert!(matches!(
        message.actor,
        agent_room_bridge_core::messages::ProjectedMessageActor::Human { .. }
    ));
    assert_eq!(
        message.actor.instance_verification(),
        ProjectedActorInstanceVerification::MatrixSenderMatched
    );
    assert_eq!(
        message.preview.conversation().expect("保留聊天").text(),
        "你怎么看？"
    );
}

fn sync_token_for_chat() -> MatrixSyncToken {
    MatrixSyncToken::new("chat-sync").expect("游标有效")
}

#[tokio::test]
async fn conversation_attachments_keep_the_content_reference_and_reject_paths() {
    let fixture = 测试夹具::new();
    let mut image = preview_payload(
        Uuid::now_v7(),
        room_id().as_str(),
        "2026-09-05T12:00:00.000Z",
        None,
    );
    image["schemaVersion"] = json!("2.0");
    image["eventType"] = json!("io.github.rainyflash.agentroom.message.preview.v2");
    image["actor"] = json!({"kind": "human", "principalId": Uuid::now_v7(), "displayName": "小雨", "matrixUserId": ACTOR_MATRIX_ID});
    image["preview"]["contentType"] = json!("image/png");
    image["content"]["mediaType"] = json!("image/png");
    image["preview"]["conversation"] =
        json!({"text": "看看这张设计", "mentions": [], "attachmentName": "design.png"});
    let mut invalid = image.clone();
    invalid["id"] = json!(Uuid::now_v7());
    invalid["preview"]["conversation"]["attachmentName"] = json!("../secret.png");
    let sync = MatrixSyncBatch::new(
        sync_token_for_chat(),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            vec![
                timeline_event(
                    "$image",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    image,
                    None,
                ),
                timeline_event(
                    "$bad-image",
                    "io.github.rainyflash.agentroom.message.preview.v2",
                    invalid,
                    None,
                ),
            ],
            Vec::new(),
        )],
    );
    let result = fixture.service().process(&sync).await.expect("投影完成");
    assert_eq!(result.accepted_events, 1);
    assert_eq!(result.isolated_events, 1);
    let batches = fixture.projections.batches.lock().expect("锁可用");
    let MessageProjectionMutation::Preview(message) = &batches[0].mutations()[0] else {
        panic!("附件应为消息");
    };
    assert_eq!(message.preview.content_type().as_str(), "image/png");
    assert_eq!(
        message
            .preview
            .conversation()
            .expect("聊天元数据")
            .attachment_name(),
        Some("design.png")
    );
}
