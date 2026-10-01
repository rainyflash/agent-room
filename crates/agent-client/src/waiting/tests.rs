use std::{sync::Mutex, time::Duration};

use agent_room_bridge_ipc::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcConversationMessage,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
    IpcMethod, IpcResponse,
    wake::{WaitOptions, WaitRules, WakeReason},
};
use tokio::time::Instant;

use super::InboxWaiter;
use crate::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture, MessageWait};

const ME: &str = "@scout:room.test";
const ADA: &str = "@ada:room.test";
const NOVA: &str = "@nova:room.test";
const SESSION: &str = "01990d9e-8400-7000-8000-000000000001";

fn preview(from: &str, human: bool, text: &str, mentions: &[&str]) -> IpcMessagePreviewSummary {
    let actor = if human {
        IpcActorSummary::Human {
            principal_id: format!("principal-{from}"),
            display_name: from.to_owned(),
            matrix_user_id: from.to_owned(),
            avatar_url: None,
        }
    } else {
        IpcActorSummary::Agent {
            agent: IpcAgentSummary {
                agent_id: format!("agent-{from}"),
                display_name: from.to_owned(),
                matrix_user_id: from.to_owned(),
                avatar_url: None,
            },
            instance_id: "instance".to_owned(),
            provenance: IpcMessageProvenance::AutonomousAgent,
        }
    };
    IpcMessagePreviewSummary {
        conversation: Some(IpcConversationMessage {
            attachment_name: None,
            text: text.to_owned(),
            mentions: mentions.iter().map(|&person| person.to_owned()).collect(),
            truncated: false,
            full_length: None,
        }),
        reply_to_message_id: None,
        reply_to: None,
        message_id: format!("m-{text}"),
        event_id: format!("${text}:room.test"),
        room_id: "!lobby:room.test".to_owned(),
        actor,
        created_at_unix_ms: 1_000,
        title: text.to_owned(),
        summary: text.to_owned(),
        content: IpcContentReference {
            content_id: "content".to_owned(),
            digest_sha256: "0".repeat(64),
            media_type: "text/plain".to_owned(),
            size_bytes: 1,
        },
        language: None,
        sensitivity: IpcMessageSensitivity::Normal,
        risk_flags: Vec::new(),
        from_me: from == ME,
        mentions_me: mentions.contains(&ME),
    }
}

fn chatter(text: &str) -> IpcMessagePreviewSummary {
    preview(NOVA, false, text, &[])
}

fn named(text: &str) -> IpcMessagePreviewSummary {
    preview(NOVA, false, text, &[ME])
}

fn mine(text: &str, mentions: &[&str]) -> IpcMessagePreviewSummary {
    preview(ME, false, text, mentions)
}

/// 假的 Bridge：每条消息到了设定的时刻才出现，撤回的到时刻就不见了。
struct Room {
    started: Instant,
    messages: Vec<(Duration, IpcMessagePreviewSummary)>,
    redactions: Vec<(Duration, String)>,
    /// 每次调用要花的时间：真的 IPC 第一次轮询不会立刻就绪。
    latency: Duration,
    /// 认不认挂着等（`waitMs`）：不认的 Bridge 立刻空手返回。
    honors_wait: bool,
    calls: Mutex<Vec<(String, bool)>>,
}

impl Room {
    fn new(messages: Vec<(u64, IpcMessagePreviewSummary)>) -> Self {
        Self {
            started: Instant::now(),
            messages: messages
                .into_iter()
                .map(|(millis, preview)| (Duration::from_millis(millis), preview))
                .collect(),
            redactions: Vec::new(),
            latency: Duration::ZERO,
            honors_wait: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn visible(&self) -> Vec<IpcMessagePreviewSummary> {
        let elapsed = self.started.elapsed();
        self.messages
            .iter()
            .filter(|(at, preview)| {
                *at <= elapsed
                    && !self
                        .redactions
                        .iter()
                        .any(|(gone, id)| *gone <= elapsed && *id == preview.event_id)
            })
            .map(|(_, preview)| preview.clone())
            .collect()
    }

    fn calls(&self) -> Vec<(String, bool)> {
        self.calls.lock().unwrap().clone()
    }

    fn page(&self, request: &IpcListPreviewsRequest, newest_first: bool) -> IpcResponse {
        let mut visible = self.visible();
        if newest_first {
            visible.reverse();
        }
        let start = request.after_event_id.as_ref().map_or(0, |after| {
            visible
                .iter()
                .position(|preview| preview.event_id == *after)
                .map_or(visible.len(), |index| index + 1)
        });
        let rest = &visible[start..];
        let limit = usize::from(request.limit);
        IpcResponse::MessagePreviews {
            previews: rest.iter().take(limit).cloned().collect(),
            next_cursor: (rest.len() > limit).then(|| rest[limit - 1].event_id.clone()),
        }
    }
}

impl BridgeToolClient for Room {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let IpcMethod::WithSession { session_id, method } = method else {
            panic!("等消息总是带着会话");
        };
        assert_eq!(session_id, SESSION);
        let blocking = match &*method {
            IpcMethod::WaitInbox(request) if self.honors_wait => request.wait_ms,
            _ => None,
        };
        let response = match *method {
            IpcMethod::WaitInbox(request) => {
                self.calls
                    .lock()
                    .unwrap()
                    .push(("wait_inbox".to_owned(), request.keep_waiting));
                if let Some(wait_ms) = blocking {
                    // 像 Bridge 一样挂着等：来了新消息就交，到点空手返回。
                    let request = request.clone();
                    let latency = self.latency;
                    return Box::pin(async move {
                        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms));
                        loop {
                            let page = self.page(&request, false);
                            let empty = matches!(&page, IpcResponse::MessagePreviews { previews, .. } if previews.is_empty());
                            if !empty || Instant::now() >= deadline {
                                if !latency.is_zero() {
                                    tokio::time::sleep(latency).await;
                                }
                                return Ok::<_, BridgeToolFailure>(page);
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    });
                }
                self.page(&request, false)
            }
            IpcMethod::ReadInbox(request) => {
                self.calls
                    .lock()
                    .unwrap()
                    .push(("read_inbox".to_owned(), request.keep_waiting));
                self.page(&request, false)
            }
            IpcMethod::ListPreviews(request) => {
                self.calls
                    .lock()
                    .unwrap()
                    .push(("list_previews".to_owned(), false));
                self.page(&request, true)
            }
            other => panic!("没想到会调 {}", other.name()),
        };
        let latency = self.latency;
        Box::pin(async move {
            if !latency.is_zero() {
                tokio::time::sleep(latency).await;
            }
            Ok::<_, BridgeToolFailure>(response)
        })
    }
}

fn waiter(rules: WaitRules) -> InboxWaiter {
    InboxWaiter::new(SESSION.to_owned(), None, None, 20, rules)
}

fn texts(previews: &[IpcMessagePreviewSummary]) -> Vec<&str> {
    previews
        .iter()
        .map(|preview| preview.conversation.as_ref().unwrap().text.as_str())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn 默认跟它有关的才交_防抖后连同之前的一起给() {
    let room = Room::new(vec![
        (0, chatter("我们先聊")),
        (2_000, named("Scout 你看呢")),
    ]);
    let started = Instant::now();
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(texts(&batch.previews), ["我们先聊", "Scout 你看呢"]);
    assert_eq!(batch.wake.reason, WakeReason::Messages);
    assert_eq!(batch.wake.event_ids, ["$Scout 你看呢:room.test"]);
    assert_eq!(batch.cursor.as_deref(), Some("$Scout 你看呢:room.test"));
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(7),
        "点名后等对话停 5 秒"
    );

    let calls = room.calls();
    let (last, holds) = calls.split_last().unwrap();
    assert!(
        holds
            .iter()
            .all(|(name, keep)| name == "wait_inbox" && *keep),
        "先不交的那几次照样算在等：{holds:?}"
    );
    assert_eq!(
        last,
        &("read_inbox".to_owned(), false),
        "交了就告诉 Bridge 不等了"
    );
}

#[tokio::test(start_paused = true)]
async fn 自己发的不叫醒也不交_等满时间空手返回() {
    let room = Room::new(vec![(1_000, mine("我先说", &[]))]);
    let mut waiter = waiter(WaitRules::default());
    let batch = waiter
        .next(&room, MessageWait::For(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(batch.previews.is_empty());
    assert_eq!(batch.wake.reason, WakeReason::Timeout);
    assert_eq!(
        batch.cursor.as_deref(),
        Some("$我先说:room.test"),
        "看过的只有自己发的，游标跟上"
    );
}

#[tokio::test(start_paused = true)]
async fn listen_没叫醒它的留着_下一轮有事时一起给() {
    let room = Room::new(vec![(1_000, chatter("闲聊")), (12_000, named("Scout？"))]);
    let mut waiter = waiter(WaitRules::default());
    let first = waiter
        .next(&room, MessageWait::Continuous(Duration::from_secs(10)))
        .await
        .unwrap();
    assert!(first.previews.is_empty());
    assert_eq!(first.remaining, 1, "闲聊还攒着");

    let second = waiter
        .next(&room, MessageWait::Continuous(Duration::from_secs(10)))
        .await
        .unwrap();
    assert_eq!(texts(&second.previews), ["闲聊", "Scout？"]);
    assert!(
        room.calls().iter().all(|(name, _)| name == "wait_inbox"),
        "listen 一直在等，不发“不等了”"
    );
}

#[tokio::test(start_paused = true)]
async fn 等满时间就空手返回_不拿过了期限的调用去截断() {
    let mut room = Room::new(vec![(0, chatter("闲聊"))]);
    room.latency = Duration::from_millis(30);
    let started = Instant::now();
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_secs(5)))
        .await
        .expect("到点空手返回，不报超时错误");
    assert!(batch.previews.is_empty());
    assert_eq!(batch.wake.reason, WakeReason::Timeout);
    assert_eq!(batch.remaining, 1, "闲聊还攒着");
    assert!(started.elapsed() < Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn 只看一眼_读一次_有什么给什么() {
    let room = Room::new(vec![(0, chatter("闲聊")), (0, mine("我说的", &[]))]);
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::ZERO))
        .await
        .unwrap();
    assert_eq!(texts(&batch.previews), ["闲聊"]);
    assert_eq!(batch.cursor.as_deref(), Some("$我说的:room.test"));
    assert_eq!(room.calls(), [("read_inbox".to_owned(), false)]);
}

#[tokio::test(start_paused = true)]
async fn 等上一条点到的人都说过话() {
    let room = Room::new(vec![
        (0, mine("Ada 你怎么看", &[ADA])),
        (1_000, chatter("插一句")),
        (3_000, preview(ADA, true, "我同意", &[])),
    ]);
    let rules = WaitRules {
        options: WaitOptions::default(),
        wait_for_mentioned: true,
    };
    let mut waiter = InboxWaiter::new(
        SESSION.to_owned(),
        None,
        Some("$Ada 你怎么看:room.test".to_owned()),
        20,
        rules.clone(),
    );
    let batch = waiter.next(&room, MessageWait::UntilMessage).await.unwrap();
    assert_eq!(batch.wake.reason, WakeReason::AllReplied);
    assert_eq!(texts(&batch.previews), ["插一句", "我同意"]);

    let silent = Room::new(vec![(0, chatter("闲聊"))]);
    let failure = InboxWaiter::new(SESSION.to_owned(), None, None, 20, rules)
        .next(&silent, MessageWait::UntilMessage)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "agent.inbox.wait_invalid");
    assert_eq!(
        failure.details()["field"],
        "waitFor",
        "没发过点名的话说不清等谁"
    );
}

#[tokio::test(start_paused = true)]
async fn 交之前重读一遍_已经撤回的不交() {
    let mut room = Room::new(vec![(500, named("Scout 你看下"))]);
    room.redactions
        .push((Duration::from_secs(3), "$Scout 你看下:room.test".to_owned()));
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_secs(8)))
        .await
        .unwrap();
    assert!(batch.previews.is_empty(), "{:?}", texts(&batch.previews));
    assert_eq!(batch.wake.reason, WakeReason::Timeout);
}

#[tokio::test(start_paused = true)]
async fn 攒满以后丢掉最早的_交的时候算进跳过的() {
    let mut messages: Vec<_> = (0..250)
        .map(|index| (0, chatter(&format!("闲聊 {index}"))))
        .collect();
    messages.push((1_000, named("Scout 在吗")));
    let room = Room::new(messages);
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(batch.previews.len(), 20);
    assert_eq!(texts(&batch.previews).last(), Some(&"Scout 在吗"));
    assert_eq!(batch.skipped + batch.previews.len(), 251, "一条都没算丢");
}

#[tokio::test(start_paused = true)]
async fn 调用方自己判断哪条叫醒它() {
    let room = Room::new(vec![
        (0, chatter("闲聊")),
        (1_000, preview(ADA, true, "在吗", &[])),
    ]);
    let only_ada = |message: &IpcMessagePreviewSummary| matches!(&message.actor, IpcActorSummary::Human { matrix_user_id, .. } if matrix_user_id == ADA);
    let batch = waiter(WaitRules::default())
        .with_wakes(only_ada)
        .next(&room, MessageWait::For(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(texts(&batch.previews), ["闲聊", "在吗"]);
    assert_eq!(batch.wake.event_ids, ["$在吗:room.test"]);
}

#[tokio::test(start_paused = true)]
async fn 一直在丢最早的消息时_定时看一眼照样到点() {
    // 从第 1 秒起每秒来 60 条闲聊，攒满 200 条以后一直在丢。丢掉的最早那条到的时间留给剩下
    // 最早的一条，20 秒以后照样到点；不留的话最早一条永远只有 3 秒多，等满一分钟也不到点。
    let messages: Vec<_> = (0..4_000_u64)
        .map(|index| {
            (
                1_000 + index * 1_000 / 60,
                chatter(&format!("闲聊 {index}")),
            )
        })
        .collect();
    let room = Room::new(messages);
    let rules = WaitRules {
        options: WaitOptions {
            digest: Some(Duration::from_secs(20)),
            ..WaitOptions::default()
        },
        wait_for_mentioned: false,
    };
    let batch = waiter(rules)
        .next(&room, MessageWait::For(Duration::from_mins(1)))
        .await
        .unwrap();
    assert_eq!(batch.wake.reason, WakeReason::Digest);
    assert!(batch.skipped > 0, "攒满以后丢掉的算进跳过的");
}

#[tokio::test(start_paused = true)]
async fn 请_bridge_挂着等_不用每秒问一次() {
    let room = Room::new(vec![(20_000, preview(ADA, true, "在吗", &[]))]);
    let started = Instant::now();
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_mins(1)))
        .await
        .unwrap();
    assert_eq!(texts(&batch.previews), ["在吗"]);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(25),
        "消息第 20 秒到，再等对话停 5 秒"
    );
    let waits = room
        .calls()
        .iter()
        .filter(|(name, _)| name == "wait_inbox")
        .count();
    assert!(waits <= 8, "挂着等，25 秒只问了 {waits} 次，不是每秒一次");
}

#[tokio::test(start_paused = true)]
async fn bridge_不认挂着等时退回每秒问一次_不空转() {
    let mut room = Room::new(vec![(10_000, preview(ADA, true, "在吗", &[]))]);
    room.honors_wait = false;
    let batch = waiter(WaitRules::default())
        .next(&room, MessageWait::For(Duration::from_mins(1)))
        .await
        .unwrap();
    assert_eq!(texts(&batch.previews), ["在吗"]);
    let waits = room
        .calls()
        .iter()
        .filter(|(name, _)| name == "wait_inbox")
        .count();
    assert!(
        (10..=40).contains(&waits),
        "大约每秒问一次，问了 {waits} 次"
    );
}
