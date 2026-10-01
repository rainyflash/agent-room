use agent_room_agent_client::{
    BridgeToolClient, BridgeToolFailure, BridgeToolFuture, MessageReadMode, MessageWait,
    reception::{CheckpointFailure, ReceptionCheckpoint, ReceptionPolicy},
    wait_for_messages,
};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcConversationMessage,
    IpcErrorCategory, IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance,
    IpcMessageSensitivity, IpcMethod, IpcResponse,
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
            truncated: false,
            full_length: None,
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
        reply_to: None,
        from_me: false,
        mentions_me: false,
        room_name: None,
        before_join: false,
    }
}
fn page(messages: Vec<IpcMessagePreviewSummary>) -> IpcResponse {
    IpcResponse::MessagePreviews {
        previews: messages,
        next_cursor: None,
        typing: Vec::new(),
    }
}
fn request() -> IpcListPreviewsRequest {
    IpcListPreviewsRequest {
        room_id: Some("!room:test".into()),
        after_event_id: Some("$previous".into()),
        before_event_id: None,
        limit: 20,
        keep_waiting: false,
        wait_ms: None,
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
        ["wait_inbox"],
        "持续监听马上开始下一轮，窗口结束不撤销等待状态，免得房间里等待信号一闪一闪"
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
fn 待处理状态必须在下一批投递前明确完成且可持久化恢复() {
    let mut checkpoint = ReceptionCheckpoint::Ready {
        after_event_id: None,
    };
    checkpoint.begin("$one").unwrap();
    let serialized = serde_json::to_string(&checkpoint).unwrap();
    let mut restored: ReceptionCheckpoint = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        restored.begin("$two"),
        Err(CheckpointFailure::PendingReviewRequired)
    );
    assert_eq!(
        restored.complete("$two"),
        Err(CheckpointFailure::EventMismatch)
    );
    restored.complete("$one").unwrap();
    assert_eq!(restored.cursor(), Some("$one"));
}

fn policy() -> ReceptionPolicy {
    ReceptionPolicy {
        room_id: "!room:test".into(),
        allowed_principal_id: SESSION.into(),
        digest_minutes: None,
    }
}

#[test]
fn 后台回复只有主人和私人房间里点名它的人叫得醒() {
    let policy = policy();
    // 主人说的、跟它有关的话：没点别人也算。
    let mut owner = preview("$owner");
    if let Some(chat) = &mut owner.conversation {
        chat.mentions.clear();
    }
    assert!(policy.wakes(&owner, false));
    // 主人点了别人、没点它的不算。
    let mut to_other = owner.clone();
    if let Some(chat) = &mut to_other.conversation {
        chat.mentions = vec!["@other:test".into()];
    }
    assert!(!policy.wakes(&to_other, false));

    // 别人：私人房间里点名或回复它才算，公开大厅里不算。
    let mut stranger = preview("$stranger");
    stranger.mentions_me = true;
    if let IpcActorSummary::Human { principal_id, .. } = &mut stranger.actor {
        *principal_id = "another-person".into();
    }
    assert!(policy.wakes(&stranger, true));
    assert!(!policy.wakes(&stranger, false));
    stranger.mentions_me = false;
    assert!(!policy.wakes(&stranger, true));

    // 别的房间、自己发的、别的 Agent 都叫不醒它。
    let mut elsewhere = owner.clone();
    elsewhere.room_id = "!other:test".into();
    assert!(!policy.wakes(&elsewhere, false));
    let mut mine = owner.clone();
    mine.from_me = true;
    assert!(!policy.wakes(&mine, false));
    let mut agent = owner;
    agent.mentions_me = true;
    agent.actor = IpcActorSummary::Agent {
        agent: IpcAgentSummary {
            agent_id: SESSION.into(),
            display_name: "Other agent".into(),
            matrix_user_id: "@other-agent:test".into(),
            avatar_url: None,
        },
        instance_id: SESSION.into(),
        provenance: IpcMessageProvenance::AutonomousAgent,
    };
    assert!(!policy.wakes(&agent, true));
}

#[test]
fn 后台的定时看一眼按主人的设置_旧版存的策略照样能读() {
    let mut policy = policy();
    assert_eq!(policy.wait_rules().options.digest, None);
    policy.digest_minutes = Some(60);
    let rules = policy.wait_rules();
    assert_eq!(rules.options.digest, Some(Duration::from_hours(1)));
    assert_eq!(rules.options.room_id.as_deref(), Some("!room:test"));

    let old: ReceptionPolicy =
        serde_json::from_str(r#"{"roomId":"!room:test","allowedPrincipalId":"x"}"#).unwrap();
    assert_eq!(old.digest_minutes, None);
    assert!(
        !serde_json::to_string(&old)
            .unwrap()
            .contains("digestMinutes"),
        "没设时不写出来，旧版也读得回去"
    );
}
