//! 找回房间密钥后重读隔离的消息：只看 Bridge 核心怎么调存储和 Matrix，存储本身的排序另有测试。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::ports::{
    DeviceSignature, MatrixEventId, MatrixEventType, MatrixFailure, MatrixFailureKind,
    MatrixOperation, MatrixResult, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixTransactionId, MatrixUserId,
    PortFuture,
};
use agent_room_bridge_core::messages::{
    IsolatedSession, MessageAuthenticationDecision, MessageAuthenticationFailure,
    MessageBackfillBatch, MessageEventAuthenticator, MessageProjectionBatch,
    MessageProjectionStoreFailure, MessageRecoveryBatch, MessageRecoveryOutcome,
    MessageRecoverySource, MessageStoreFailure, MessageStoreFailureKind, MessageSubmissionClaim,
    MessageSubmissionClaimOutcome, MessageSubmissionRecord, MessageSubmissionRepository,
    MessageSyncDependencies, MessageSyncIssueReason, MessageSyncService,
    MessageTimelineProjectionStore, PendingTimelineGap, ReservedIsolatedEvent,
    UndecryptableSession,
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, MessageSubmissionId},
    time::UtcMillis,
};
use serde_json::json;
use uuid::Uuid;

const HUMAN: &str = "@rainy:matrix.test";
const HUMAN_DEVICE: &str = "WEBDEVICE";
const SESSION: &str = "session-before-join";

/// 只记录写入的投影存储；隔离事件的位置按记录的先后编号。
#[derive(Default)]
struct 记录存储 {
    batches: Mutex<Vec<MessageProjectionBatch>>,
    recoveries: Mutex<Vec<MessageRecoveryBatch>>,
}

impl 记录存储 {
    fn isolated(&self) -> Vec<(MatrixRoomId, MatrixEventId, UndecryptableSession, u64)> {
        let settled = self
            .recoveries
            .lock()
            .expect("锁可用")
            .iter()
            .flat_map(|batch| {
                batch
                    .recovered()
                    .iter()
                    .map(|mutation| mutation.event_id().clone())
                    .chain(
                        batch
                            .reclassified()
                            .iter()
                            .filter_map(|issue| issue.event_id.clone()),
                    )
                    .chain(batch.dismissed().iter().cloned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut position = 0;
        let mut isolated = Vec::new();
        for batch in self.batches.lock().expect("锁可用").iter() {
            for issue in batch.issues() {
                let (Some(event_id), Some(session)) = (&issue.event_id, &issue.session) else {
                    continue;
                };
                position += 1;
                if issue.reason == MessageSyncIssueReason::Undecryptable
                    && !settled.contains(event_id)
                {
                    isolated.push((
                        issue.room_id.clone(),
                        event_id.clone(),
                        session.clone(),
                        position,
                    ));
                }
            }
        }
        isolated
    }
}

impl MessageTimelineProjectionStore for 记录存储 {
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        self.batches.lock().expect("锁可用").push(batch.clone());
        Box::pin(async { Ok(()) })
    }

    fn sync_cursor(
        &self,
    ) -> PortFuture<'_, Result<Option<MatrixSyncToken>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(None) })
    }

    fn room_has_messages<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<bool, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(false) })
    }

    fn known_events<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event_ids: &'a [MatrixEventId],
    ) -> PortFuture<'a, Result<Vec<MatrixEventId>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn pending_gaps(
        &self,
        _limit: u16,
    ) -> PortFuture<'_, Result<Vec<PendingTimelineGap>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn apply_backfill<'a>(
        &'a self,
        _batch: &'a MessageBackfillBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(()) })
    }

    fn undecryptable_sessions(
        &self,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<IsolatedSession>, MessageProjectionStoreFailure>> {
        let mut sessions = Vec::<IsolatedSession>::new();
        for (room_id, _, session, _) in self.isolated().into_iter().rev() {
            if !sessions.iter().any(|known| known.session == session) {
                sessions.push(IsolatedSession { room_id, session });
            }
        }
        sessions.truncate(usize::from(limit));
        Box::pin(async move { Ok(sessions) })
    }

    fn undecryptable_events<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        session_ids: &'a [String],
        after: u64,
        limit: u16,
    ) -> PortFuture<'a, Result<Vec<ReservedIsolatedEvent>, MessageProjectionStoreFailure>> {
        let events = self
            .isolated()
            .into_iter()
            .filter(|(room, _, session, position)| {
                room == room_id && session_ids.contains(&session.session_id) && *position > after
            })
            .take(usize::from(limit))
            .map(|(_, event_id, _, position)| ReservedIsolatedEvent { event_id, position })
            .collect();
        Box::pin(async move { Ok(events) })
    }

    fn apply_recovery<'a>(
        &'a self,
        batch: &'a MessageRecoveryBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        self.recoveries.lock().expect("锁可用").push(batch.clone());
        Box::pin(async { Ok(()) })
    }
}

/// 请求过的房间密钥：房间、发送者、发送设备和会话。
type 密钥请求 = (String, String, Option<String>, Vec<String>);

/// 按事件 ID 给出重读结果，并记下请求过哪些房间密钥。
#[derive(Default)]
struct 重读源 {
    events: Mutex<HashMap<String, MatrixResult<MatrixTimelineEvent>>>,
    fetched: Mutex<Vec<String>>,
    requests: Mutex<Vec<密钥请求>>,
}

impl MessageRecoverySource for 重读源 {
    fn fetch_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<MatrixTimelineEvent>> {
        self.fetched
            .lock()
            .expect("锁可用")
            .push(event_id.as_str().to_owned());
        let result = self
            .events
            .lock()
            .expect("锁可用")
            .remove(event_id.as_str())
            .unwrap_or_else(|| Err(failure(MatrixFailureKind::NotFound)));
        Box::pin(async move { result })
    }

    fn request_room_keys(
        &self,
        room_id: &MatrixRoomId,
        sender: &MatrixUserId,
        sender_device: Option<&str>,
        session_ids: &[String],
    ) {
        self.requests.lock().expect("锁可用").push((
            room_id.as_str().to_owned(),
            sender.as_str().to_owned(),
            sender_device.map(str::to_owned),
            session_ids.to_vec(),
        ));
    }
}

struct 放行认证器;

impl MessageEventAuthenticator for 放行认证器 {
    fn authenticate<'a>(
        &'a self,
        _agent_id: AgentId,
        _instance_id: AgentInstanceId,
        _origin_server_timestamp: UtcMillis,
        _canonical_event: &'a [u8],
        _signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<MessageAuthenticationDecision, MessageAuthenticationFailure>> {
        Box::pin(async { Ok(MessageAuthenticationDecision::Trusted) })
    }
}

struct 空提交仓库;

impl MessageSubmissionRepository for 空提交仓库 {
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
        not_found()
    }

    fn mark_accepted<'a>(
        &'a self,
        _submission_id: MessageSubmissionId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        not_found()
    }

    fn mark_bound(
        &self,
        _submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        not_found()
    }

    fn observe_transaction<'a>(
        &'a self,
        _transaction_id: &'a MatrixTransactionId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        Box::pin(async { Ok(None) })
    }
}

fn not_found<'a>() -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
    Box::pin(async { Err(MessageStoreFailure::new(MessageStoreFailureKind::NotFound)) })
}

fn failure(kind: MatrixFailureKind) -> MatrixFailure {
    MatrixFailure::new(MatrixOperation::Backfill, kind)
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!private:matrix.test").expect("房间标识有效")
}

fn event(event_id: &str, event_type: &str, content: serde_json::Value) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).expect("事件标识有效")),
        Some(MatrixUserId::new(HUMAN).expect("用户标识有效")),
        MatrixEventType::new(event_type).expect("事件类型有效"),
        None,
        None,
        Some(1_000),
        content,
    )
    .expect("时间线事件有效")
}

fn encrypted(event_id: &str) -> MatrixTimelineEvent {
    event(
        event_id,
        "m.room.encrypted",
        json!({
            "algorithm": "m.megolm.v1.aes-sha2",
            "ciphertext": "opaque",
            "device_id": HUMAN_DEVICE,
            "session_id": SESSION,
        }),
    )
}

fn human_chat(event_id: &str, text: &str) -> MatrixTimelineEvent {
    event(
        event_id,
        "io.github.rainyflash.agentroom.message.preview.v2",
        json!({
            "schemaVersion": "2.0",
            "eventType": "io.github.rainyflash.agentroom.message.preview.v2",
            "id": Uuid::now_v7(),
            "createdAt": "2026-09-29T08:00:00.000Z",
            "actor": {"kind": "human", "principalId": Uuid::now_v7(), "displayName": "小雨", "matrixUserId": HUMAN},
            "correlationId": Uuid::now_v7(),
            "roomId": room_id().as_str(),
            "preview": {
                "title": "聊天",
                "summary": text,
                "contentType": "text/plain",
                "language": "zh-CN",
                "sensitivity": "normal",
                "riskFlags": [],
                "conversation": {"text": text, "mentions": []}
            },
            "content": {
                "contentId": Uuid::now_v7(),
                "digestSha256": "11".repeat(32),
                "sizeBytes": 128,
                "mediaType": "text/plain",
                "fetchMode": "on_demand"
            }
        }),
    )
    .with_trusted_end_to_end_encryption()
}

fn service(store: &Arc<记录存储>) -> MessageSyncService {
    MessageSyncService::new(MessageSyncDependencies {
        authenticator: Arc::new(放行认证器),
        projections: store.clone(),
        submissions: Arc::new(空提交仓库),
    })
}

async fn sync_timeline(service: &MessageSyncService, events: Vec<MatrixTimelineEvent>) {
    service
        .process(&MatrixSyncBatch::new(
            MatrixSyncToken::new("sync-1").expect("同步游标有效"),
            vec![MatrixRoomSync::new(
                room_id(),
                MatrixRoomSyncKind::Joined,
                false,
                None,
                events,
                Vec::new(),
            )],
        ))
        .await
        .expect("可同步");
}

#[tokio::test]
async fn 解不开的事件记下会话和它在时间线里的位置() {
    let store = Arc::new(记录存储::default());
    sync_timeline(
        &service(&store),
        vec![
            encrypted("$before-join:matrix.test"),
            human_chat("$after-join:matrix.test", "加入后"),
            encrypted("$later:matrix.test"),
        ],
    )
    .await;

    let batches = store.batches.lock().expect("锁可用");
    let issues = batches[0].issues();
    assert_eq!(
        issues
            .iter()
            .map(|issue| issue.mutations_before)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        issues[0].session,
        Some(UndecryptableSession {
            sender: MatrixUserId::new(HUMAN).expect("用户标识有效"),
            sender_device: Some(HUMAN_DEVICE.to_owned()),
            session_id: SESSION.to_owned(),
        })
    );
}

#[tokio::test]
async fn 导入会话后重读用到它的隔离事件_读出来的写回_仍解不开的不动() {
    let store = Arc::new(记录存储::default());
    let service = service(&store);
    sync_timeline(
        &service,
        vec![
            encrypted("$recovered:matrix.test"),
            encrypted("$still-locked:matrix.test"),
            encrypted("$reaction:matrix.test"),
        ],
    )
    .await;
    let source = 重读源::default();
    source.events.lock().expect("锁可用").extend([
        (
            "$recovered:matrix.test".to_owned(),
            Ok(human_chat("$recovered:matrix.test", "加入前说的话")),
        ),
        (
            "$still-locked:matrix.test".to_owned(),
            Ok(encrypted("$still-locked:matrix.test")),
        ),
        (
            "$reaction:matrix.test".to_owned(),
            Ok(event("$reaction:matrix.test", "m.reaction", json!({}))),
        ),
    ]);

    let outcome = service
        .recover_isolated(&source, &[(room_id(), SESSION.to_owned())])
        .await
        .expect("可重读");

    assert_eq!(outcome.recovered_events, 1);
    assert_eq!(outcome.still_isolated, 1);
    assert!(outcome.deferred.is_empty());
    let recoveries = store.recoveries.lock().expect("锁可用");
    assert_eq!(recoveries.len(), 1);
    assert_eq!(
        recoveries[0].recovered()[0].event_id().as_str(),
        "$recovered:matrix.test"
    );
    assert_eq!(
        recoveries[0]
            .dismissed()
            .iter()
            .map(MatrixEventId::as_str)
            .collect::<Vec<_>>(),
        ["$reaction:matrix.test"],
        "解开了但不是消息的，只删掉隔离记录"
    );
}

#[tokio::test]
async fn 暂时读不到的会话留到下一轮_没关系的会话不读() {
    let store = Arc::new(记录存储::default());
    let service = service(&store);
    sync_timeline(&service, vec![encrypted("$flaky:matrix.test")]).await;
    let source = 重读源::default();
    source.events.lock().expect("锁可用").insert(
        "$flaky:matrix.test".to_owned(),
        Err(failure(MatrixFailureKind::Timeout)),
    );

    let unrelated = service
        .recover_isolated(&source, &[(room_id(), "another-session".to_owned())])
        .await
        .expect("可重读");
    assert_eq!(unrelated, MessageRecoveryOutcome::default());
    assert!(source.fetched.lock().expect("锁可用").is_empty());

    let outcome = service
        .recover_isolated(&source, &[(room_id(), SESSION.to_owned())])
        .await
        .expect("可重读");
    assert_eq!(outcome.deferred, [(room_id(), SESSION.to_owned())]);
    assert!(store.recoveries.lock().expect("锁可用").is_empty());
}

#[tokio::test]
async fn 启动后按发送者和设备重新请求仍解不开的会话() {
    let store = Arc::new(记录存储::default());
    let service = service(&store);
    sync_timeline(
        &service,
        vec![encrypted("$one:matrix.test"), encrypted("$two:matrix.test")],
    )
    .await;
    let source = 重读源::default();

    let requested = service.rerequest_isolated(&source).await.expect("可请求");

    assert_eq!(requested, 1, "两条事件用的是同一个会话");
    assert_eq!(
        *source.requests.lock().expect("锁可用"),
        [(
            room_id().as_str().to_owned(),
            HUMAN.to_owned(),
            Some(HUMAN_DEVICE.to_owned()),
            vec![SESSION.to_owned()],
        )]
    );
}
