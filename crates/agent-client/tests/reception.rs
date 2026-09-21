use agent_room_agent_client::{
    BridgeToolClient, BridgeToolFailure, BridgeToolFuture, MessageReadMode, MessageWait,
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
            attachment_name: None,
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
        MessageWait::UntilMessage,
    )
    .await
    .unwrap();
    assert_eq!(result, page(vec![preview("$next")]));
    tokio::time::sleep(Duration::from_mins(1)).await;
    let calls = backend.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
    assert_eq!(calls[0].name(), "wait_inbox");
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
                MessageWait::UntilMessage
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
            MessageWait::UntilMessage
        )
        .await
        .unwrap_err()
        .code(),
        "test.denied"
    );
}

#[tokio::test(start_paused = true)]
async fn 默认等待五分钟不返回空结果并在有消息时完成同一次调用() {
    let mut replies = vec![Ok(page(vec![])); 301];
    replies.push(Ok(page(vec![preview("$next")])));
    let backend = Backend::new(replies);
    let waiting = wait_for_messages(
        &backend,
        SESSION.into(),
        request(),
        MessageReadMode::Inbox,
        MessageWait::UntilMessage,
    );
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_mins(5), &mut waiting)
            .await
            .is_err()
    );
    assert_eq!(waiting.await.unwrap(), page(vec![preview("$next")]));
    let calls = backend.calls.lock().unwrap();
    assert_eq!(calls.len(), 302);
    assert!(calls.iter().all(|method| method == &calls[0]));
}

#[tokio::test(start_paused = true)]
async fn 只有显式期限会返回空页且允许超过二十五秒() {
    let backend = Backend::new(vec![Ok(page(vec![])); 91]);
    let start = tokio::time::Instant::now();
    let response = wait_for_messages(
        &backend,
        SESSION.into(),
        request(),
        MessageReadMode::Inbox,
        MessageWait::For(Duration::from_secs(90)),
    )
    .await
    .unwrap();
    assert_eq!(response, page(vec![]));
    assert_eq!(start.elapsed(), Duration::from_secs(90));
    {
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 91);
        assert!(calls[..90].iter().all(|method| method == &calls[0]));
        assert_eq!(calls[0].name(), "wait_inbox");
        assert_eq!(
            calls[90].name(),
            "read_inbox",
            "期限结束必须立即撤销等待状态"
        );
    }
    let backend = Backend::new(vec![Ok(page(vec![]))]);
    assert_eq!(
        wait_for_messages(
            &backend,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::For(Duration::ZERO)
        )
        .await
        .unwrap(),
        page(vec![])
    );
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    assert_eq!(backend.calls.lock().unwrap()[0].name(), "read_inbox");
}

#[tokio::test(start_paused = true)]
async fn 曾经空闲不代表后续连接失败或挂起可以当作空页() {
    struct StalledBackend(std::sync::atomic::AtomicBool);
    impl BridgeToolClient for StalledBackend {
        fn invoke(&self, _: IpcMethod) -> BridgeToolFuture<'_> {
            if self.0.swap(true, std::sync::atomic::Ordering::SeqCst) {
                Box::pin(std::future::pending())
            } else {
                Box::pin(async { Ok(page(vec![])) })
            }
        }
    }
    assert_eq!(
        wait_for_messages(
            &StalledBackend(std::sync::atomic::AtomicBool::new(false)),
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::For(Duration::from_secs(2))
        )
        .await
        .unwrap_err()
        .code(),
        "agent.inbox.timeout"
    );
    let backend = Backend::new(vec![
        Ok(page(vec![])),
        Err(BridgeToolFailure::new(
            "test.disconnected",
            IpcErrorCategory::DependencyUnavailable,
            true,
            BTreeMap::new(),
        )),
    ]);
    assert_eq!(
        wait_for_messages(
            &backend,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::UntilMessage
        )
        .await
        .unwrap_err()
        .code(),
        "test.disconnected"
    );
}

/// 等待调用慢于调用方窗口的后端；期限结束时的收尾读取照常立即返回。
struct SlowBackend {
    message: Option<IpcMessagePreviewSummary>,
    calls: Mutex<Vec<IpcMethod>>,
}
impl SlowBackend {
    const DELAY: Duration = Duration::from_secs(3);
    fn new(message: Option<IpcMessagePreviewSummary>) -> Self {
        Self {
            message,
            calls: Mutex::new(vec![]),
        }
    }
    fn names(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|method| method.name().to_owned())
            .collect()
    }
}
impl BridgeToolClient for SlowBackend {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.clone());
        let delay = if method.name() == "wait_inbox" {
            Self::DELAY
        } else {
            Duration::ZERO
        };
        let message = self.message.clone();
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Ok(page(message.into_iter().collect()))
        })
    }
}

/// 单次往返慢于窗口时，一次性读取欠调用方一个答复，持续监听不欠：窗口只结束这一轮。
#[tokio::test(start_paused = true)]
async fn 慢于窗口的往返只结束持续监听的一轮而不是整个等待() {
    assert_eq!(
        wait_for_messages(
            &SlowBackend::new(None),
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::For(Duration::from_secs(1))
        )
        .await
        .unwrap_err()
        .code(),
        "agent.inbox.timeout",
        "一次性读取必须在期限内答复，卡住的 Bridge 不能伪装成空房间"
    );

    let listening = SlowBackend::new(None);
    let start = tokio::time::Instant::now();
    assert_eq!(
        wait_for_messages(
            &listening,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::Continuous(Duration::from_secs(1))
        )
        .await
        .unwrap(),
        page(vec![]),
        "持续监听的窗口到期不是失败，调用方继续等待"
    );
    assert_eq!(start.elapsed(), SlowBackend::DELAY, "在途请求跑完才收尾");
    assert_eq!(
        listening.names(),
        ["wait_inbox", "read_inbox"],
        "窗口结束仍然撤销等待状态"
    );
}

/// 慢往返带回的消息照常投递；真实连接错误仍然终止持续监听。
#[tokio::test(start_paused = true)]
async fn 持续监听交付慢往返的消息但连接错误仍然终止() {
    let delivering = SlowBackend::new(Some(preview("$next")));
    assert_eq!(
        wait_for_messages(
            &delivering,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::Continuous(Duration::from_secs(1))
        )
        .await
        .unwrap(),
        page(vec![preview("$next")])
    );
    assert_eq!(delivering.names(), ["wait_inbox"]);

    let broken = Backend::new(vec![Err(BridgeToolFailure::new(
        "test.disconnected",
        IpcErrorCategory::DependencyUnavailable,
        true,
        BTreeMap::new(),
    ))]);
    assert_eq!(
        wait_for_messages(
            &broken,
            SESSION.into(),
            request(),
            MessageReadMode::Inbox,
            MessageWait::Continuous(Duration::from_secs(1))
        )
        .await
        .unwrap_err()
        .code(),
        "test.disconnected",
        "真实连接错误仍然终止持续监听"
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
