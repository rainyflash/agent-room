use agent_room_agent_client::{
    BridgeToolClient, BridgeToolFailure, BridgeToolFuture, MessageReadMode,
    reception::{CheckpointFailure, DeliveryDecision, ReceptionCheckpoint, ReceptionPolicy},
    wait_for_messages,
};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcContentReference, IpcConversationMessage, IpcErrorCategory,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageSensitivity, IpcMethod,
    IpcResponse,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
    time::Duration,
};

const SESSION: &str = "01990d9e-8400-7000-8000-000000000010";
fn preview(event: &str) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        conversation: Some(IpcConversationMessage {
            text: "hello".into(),
            mentions: vec!["@agent:test".into()],
        }),
        reply_to_message_id: None,
        message_id: SESSION.into(),
        event_id: event.into(),
        room_id: "!room:test".into(),
        actor: IpcActorSummary::Human {
            principal_id: SESSION.into(),
            display_name: "Owner".into(),
            matrix_user_id: "@owner:test".into(),
            avatar_url: None,
        },
        created_at_unix_ms: 1,
        title: "test".into(),
        summary: "test".into(),
        content: IpcContentReference {
            content_id: SESSION.into(),
            digest_sha256: "0".repeat(64),
            media_type: "text/plain".into(),
            size_bytes: 5,
        },
        language: None,
        sensitivity: IpcMessageSensitivity::Normal,
        risk_flags: vec![],
    }
}
fn page(messages: Vec<IpcMessagePreviewSummary>) -> IpcResponse {
    IpcResponse::MessagePreviews {
        previews: messages,
        next_cursor: None,
    }
}
fn request() -> IpcListPreviewsRequest {
    IpcListPreviewsRequest {
        room_id: Some("!room:test".into()),
        after_event_id: Some("$previous".into()),
        before_event_id: None,
        limit: 20,
    }
}

struct Backend {
    replies: Mutex<VecDeque<Result<IpcResponse, BridgeToolFailure>>>,
    calls: Mutex<Vec<IpcMethod>>,
}
impl Backend {
    fn new(replies: Vec<Result<IpcResponse, BridgeToolFailure>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(vec![]),
        }
    }
}
impl BridgeToolClient for Backend {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra poll");
        Box::pin(async move { reply })
    }
}

#[tokio::test(start_paused = true)]
async fn 等待复用同一游标并且返回后不再后台轮询() {
    let backend = Backend::new(vec![Ok(page(vec![])), Ok(page(vec![preview("$next")]))]);
    let result = wait_for_messages(
        &backend,
        SESSION.into(),
        request(),
        MessageReadMode::Inbox,
        25,
    )
    .await
    .unwrap();
    assert_eq!(result, page(vec![preview("$next")]));
    tokio::time::sleep(Duration::from_mins(1)).await;
    let calls = backend.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
    assert_eq!(calls[0].name(), "read_inbox");
}

#[tokio::test(start_paused = true)]
async fn 取消等待停止轮询且失败不会被当作空房间() {
    let backend = Backend::new(vec![Ok(page(vec![]))]);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(500),
            wait_for_messages(
                &backend,
                SESSION.into(),
                request(),
                MessageReadMode::Inbox,
                25
            )
        )
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_mins(1)).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    let backend = Backend::new(vec![Err(BridgeToolFailure::new(
        "test.denied",
        IpcErrorCategory::Authorization,
        false,
        BTreeMap::new(),
    ))]);
    assert_eq!(
        wait_for_messages(
            &backend,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            25
        )
        .await
        .unwrap_err()
        .code(),
        "test.denied"
    );
}

#[test]
fn 待处理状态必须在下一条投递前明确完成且可持久化恢复() {
    let policy = ReceptionPolicy {
        room_id: "!room:test".into(),
        allowed_principal_id: SESSION.into(),
    };
    let mut checkpoint = ReceptionCheckpoint::Ready {
        after_event_id: None,
    };
    assert_eq!(
        checkpoint
            .prepare(&preview("$one"), &policy, "@agent:test")
            .unwrap(),
        DeliveryDecision::Deliver
    );
    let serialized = serde_json::to_string(&checkpoint).unwrap();
    let mut restored: ReceptionCheckpoint = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        restored.prepare(&preview("$two"), &policy, "@agent:test"),
        Err(CheckpointFailure::PendingReviewRequired)
    );
    assert_eq!(
        restored.complete("$two"),
        Err(CheckpointFailure::EventMismatch)
    );
    restored.complete("$one").unwrap();
    assert_eq!(restored.cursor(), Some("$one"));
    assert_eq!(
        restored
            .prepare(&preview("$one"), &policy, "@agent:test")
            .unwrap(),
        DeliveryDecision::Skip
    );
}

#[test]
fn 非允许发信人或未提及自身的消息不能启动宿主任务() {
    let policy = ReceptionPolicy {
        room_id: "!room:test".into(),
        allowed_principal_id: SESSION.into(),
    };
    let mut message = preview("$test");
    assert!(policy.accepts(&message, "@agent:test"));
    assert!(!policy.accepts(&message, "@different:test"));
    message.room_id = "!other:test".into();
    assert!(!policy.accepts(&message, "@agent:test"));
    message.room_id = policy.room_id.clone();
    if let IpcActorSummary::Human { principal_id, .. } = &mut message.actor {
        *principal_id = "another-owner".into();
    }
    assert!(!policy.accepts(&message, "@agent:test"));
    let mut checkpoint = ReceptionCheckpoint::Ready {
        after_event_id: None,
    };
    assert_eq!(
        checkpoint
            .prepare(&message, &policy, "@agent:test")
            .unwrap(),
        DeliveryDecision::Skip
    );
    assert_eq!(checkpoint.cursor(), Some("$test"));
}
