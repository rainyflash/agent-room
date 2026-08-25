use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use agent_room_application::ports::{
    DeviceProofVerifier, DeviceSignature, MatrixBackfillToken, MatrixEventId, MatrixEventType,
    MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
    MatrixTimelineEvent, MatrixTransactionId, MatrixUserId, PortFuture,
};
use agent_room_bridge_core::messages::{
    MessageAuthenticationDecision, MessageAuthenticationFailure, MessageAuthenticationFailureKind,
    MessageEventAuthenticator, MessageProjectionBatch, MessageProjectionMutation,
    MessageProjectionStoreFailure, MessageProjectionStoreFailureKind, MessageStoreFailure,
    MessageStoreFailureKind, MessageSubmissionClaim, MessageSubmissionClaimOutcome,
    MessageSubmissionFingerprint, MessageSubmissionKind, MessageSubmissionRecord,
    MessageSubmissionRepository, MessageSubmissionState, MessageSyncDependencies,
    MessageSyncFailureKind, MessageSyncIssueReason, MessageSyncService,
    MessageTimelineProjectionStore, ProjectedActorInstanceVerification,
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
    fail: AtomicBool,
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
        "org.agentroom.message.preview.v1",
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
    assert_eq!(outcome.timeline_gaps, 1);
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
        "algorithm": "org.agentroom.content.aes-256-gcm.v1",
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
        "org.agentroom.message.preview.v1",
        encrypted_payload,
        None,
    )
    .with_trusted_end_to_end_encryption();
    let leaked = signed_timeline_event(
        &fixture.signing_key,
        "$plaintext:matrix.test",
        "org.agentroom.message.preview.v1",
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
        "org.agentroom.message.preview.v1",
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

fn mixed_sync(fixture: &测试夹具, message_id: Uuid, reply_target: Uuid) -> MatrixSyncBatch {
    let room_id = room_id();
    let preview = signed_timeline_event(
        &fixture.signing_key,
        "$preview:matrix.test",
        "org.agentroom.message.preview.v1",
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
        "org.agentroom.message.preview.v1",
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
        "org.agentroom.message.preview.v1",
        bad_signature_payload,
        None,
    );
    let revision = signed_timeline_event(
        &fixture.signing_key,
        "$revision:matrix.test",
        "org.agentroom.message.revision.v1",
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
        "org.agentroom.message.preview.v1",
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
        "eventType": "org.agentroom.message.preview.v1",
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
        "eventType": "org.agentroom.message.revision.v1",
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
