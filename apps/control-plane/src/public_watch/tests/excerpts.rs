//! 分享一段对话时往回读房间（[`ExcerptTimelineReader`]）：一页页读到那一句为止，最多 500 条；撤回、隐藏、
//! 验不过签名的不算；读不到那一句、房间加了密、聊天服务器不通各说各的。

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::ports::{
    ExcerptTimelineRead, MatrixBackfillToken, MatrixEventId, MatrixFailure, MatrixFailureKind,
    MatrixMessagePage, MatrixOperation, MatrixResult, MatrixRoomId, MatrixTimelineEvent,
    MatrixUserId, MatrixUserPresence, PortFuture, PublicLobbyMatrixReader, RoomExcerptTimeline,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    Agent, FORGED_SIGNATURE, FakeClock, FakeSignatures, KnownInstances, NOW, ROOM, SERVICE_USER,
    agent_chat, agent_message, human_chat, moderation_notice, preview, revision, room_state,
    timeline_event,
};
use crate::public_watch::{ExcerptTimelineDependencies, ExcerptTimelineReader};

/// 按页往回给：游标是“从第几条往前”，一页是游标前面最多 `limit` 条。打开分享的一段（`excerpt_pages.rs`）
/// 也用它。
#[derive(Default)]
pub(super) struct PagedReader {
    /// 旧的在前。
    pub(super) events: Mutex<Vec<MatrixTimelineEvent>>,
    pub(super) state: Mutex<Vec<MatrixTimelineEvent>>,
    pub(super) failing: Mutex<bool>,
    /// 每一页要了几条。
    limits: Mutex<Vec<u16>>,
    pub(super) pages: AtomicUsize,
}

impl PagedReader {
    fn read_events(&self) -> usize {
        let events = self.events.lock().unwrap().len();
        let mut end = events;
        let mut read = 0;
        for limit in self.limits.lock().unwrap().iter() {
            let start = end.saturating_sub(usize::from(*limit));
            read += end - start;
            end = start;
        }
        read
    }
}

impl PublicLobbyMatrixReader for PagedReader {
    fn recent_messages<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _limit: u16,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        unreachable!("分享时一页页往回读，不读最近的一批")
    }

    fn messages_before<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        from: Option<&'a MatrixBackfillToken>,
        limit: u16,
    ) -> PortFuture<'a, MatrixResult<MatrixMessagePage>> {
        assert_eq!(room_id.as_str(), ROOM);
        assert!((1..=100).contains(&limit), "{limit}");
        self.pages.fetch_add(1, Ordering::SeqCst);
        self.limits.lock().unwrap().push(limit);
        let answer = if *self.failing.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            let events = self.events.lock().unwrap();
            let end = from.map_or(events.len(), |token| token.as_str().parse().unwrap());
            let start = end.saturating_sub(usize::from(limit));
            let chunk = events[start..end].to_vec();
            Ok(MatrixMessagePage {
                read: chunk.len(),
                next: (!chunk.is_empty())
                    .then(|| MatrixBackfillToken::new(start.to_string()).unwrap()),
                events: chunk,
            })
        };
        Box::pin(async move { answer })
    }

    fn current_state<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        assert_eq!(room_id.as_str(), ROOM);
        let state = self.state.lock().unwrap().clone();
        Box::pin(async move { Ok(state) })
    }

    fn user_presence<'a>(
        &'a self,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixUserPresence>> {
        unreachable!("分享时不问在线")
    }
}

struct ExcerptHarness {
    reader: Arc<PagedReader>,
    timeline: ExcerptTimelineReader,
}

impl ExcerptHarness {
    fn new(agents: &[&Agent]) -> Self {
        let reader = Arc::new(PagedReader::default());
        *reader.state.lock().unwrap() = room_state(
            &agents
                .iter()
                .map(|agent| agent.matrix_user())
                .collect::<Vec<_>>(),
        );
        let timeline = ExcerptTimelineReader::new(ExcerptTimelineDependencies {
            reader: reader.clone(),
            verification: Arc::new(KnownInstances {
                agents: agents
                    .iter()
                    .map(|agent| (agent.id, agent.instance_id))
                    .collect(),
                lookups: AtomicUsize::new(0),
            }),
            signatures: Arc::new(FakeSignatures),
            clock: Arc::new(FakeClock(Mutex::new(NOW))),
        });
        Self { reader, timeline }
    }

    async fn read(&self, first: &str) -> ExcerptTimelineRead {
        self.timeline
            .visible_messages(
                &MatrixRoomId::new(ROOM).unwrap(),
                &MatrixEventId::new(first).unwrap(),
            )
            .await
            .expect("读得到")
    }
}

fn found(read: ExcerptTimelineRead) -> Vec<(String, i64)> {
    let ExcerptTimelineRead::Found(messages) = read else {
        panic!("应该读到了那一句");
    };
    messages
        .into_iter()
        .map(|message| {
            (
                message.event_id.as_str().to_owned(),
                message.sent_at.value(),
            )
        })
        .collect()
}

/// 一条人说的话，第 `index` 条，越往后越新。
fn chatter(index: usize, total: usize) -> MatrixTimelineEvent {
    let seconds_ago = i64::try_from(total - index).unwrap();
    human_chat(
        &format!("$chat-{index}:matrix.test"),
        Uuid::now_v7(),
        &format!("第 {index} 句"),
        None,
        seconds_ago,
    )
}

#[tokio::test]
async fn 一页页往回读到那一句为止_交出看得见的每一句的事件和时间() {
    let scout = Agent::new("Scout #2316");
    let harness = ExcerptHarness::new(&[&scout]);
    let withdrawn = Uuid::now_v7();
    let edited = Uuid::now_v7();
    let mut events: Vec<MatrixTimelineEvent> = (0..150).map(|index| chatter(index, 400)).collect();
    events.extend([
        agent_chat("$first:matrix.test", &scout, Uuid::now_v7(), "开头", 240),
        agent_chat("$hidden:matrix.test", &scout, Uuid::now_v7(), "被隐藏", 230),
        agent_chat("$withdrawn:matrix.test", &scout, withdrawn, "撤回的", 220),
        agent_message(
            "$forged:matrix.test",
            &scout,
            Uuid::now_v7(),
            &preview("冒名的"),
            215,
            FORGED_SIGNATURE,
        ),
        agent_chat("$edited:matrix.test", &scout, edited, "改之前", 210),
        revision("$redact:matrix.test", &scout, withdrawn, "redact", 205),
        revision("$replace:matrix.test", &scout, edited, "replace", 204),
    ]);
    events.extend((150..300).map(|index| chatter(index, 400)));
    *harness.reader.events.lock().unwrap() = events;
    harness
        .reader
        .state
        .lock()
        .unwrap()
        .push(moderation_notice(SERVICE_USER, "$hidden:matrix.test"));

    let messages = found(harness.read("$first:matrix.test").await);

    // 共 307 条，从最新往回一页 100 条：第二页（第 107 到 206 条）里有那一句，读到这一页就停。
    assert_eq!(*harness.reader.limits.lock().unwrap(), [100, 100]);
    assert_eq!(
        messages[0].0, "$chat-107:matrix.test",
        "那一句所在的整页都交出去，用例自己从那一句数起"
    );
    let start = messages
        .iter()
        .position(|(id, _)| id == "$first:matrix.test")
        .unwrap();
    assert_eq!(start, 43);
    assert_eq!(
        messages[start..start + 2],
        [
            ("$first:matrix.test".to_owned(), NOW - 240_000),
            ("$edited:matrix.test".to_owned(), NOW - 210_000),
        ],
        "隐藏、撤回、验不过签名的不算；改过的还是原来那一句"
    );
    assert_eq!(messages.len() - start, 152);
    assert_eq!(messages[start + 2].0, "$chat-150:matrix.test");
    assert_eq!(messages.last().unwrap().0, "$chat-299:matrix.test");
}

#[tokio::test]
async fn 最近_500_条里没有那一句就说读不到() {
    let harness = ExcerptHarness::new(&[]);
    let mut events = vec![human_chat(
        "$old:matrix.test",
        Uuid::now_v7(),
        "很早以前",
        None,
        3_000,
    )];
    events.extend((0..600).map(|index| chatter(index, 1_000)));
    *harness.reader.events.lock().unwrap() = events;

    assert_eq!(
        harness.read("$old:matrix.test").await,
        ExcerptTimelineRead::NotReachable
    );
    assert_eq!(harness.reader.read_events(), 500, "最多读 500 条");
    assert_eq!(harness.reader.pages.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn 读到房间开头还没有那一句也说读不到() {
    let harness = ExcerptHarness::new(&[]);
    *harness.reader.events.lock().unwrap() = (0..30).map(|index| chatter(index, 100)).collect();

    assert_eq!(
        harness.read("$elsewhere:matrix.test").await,
        ExcerptTimelineRead::NotReachable
    );
    assert_eq!(
        harness.reader.pages.load(Ordering::SeqCst),
        2,
        "第二页什么也没有，就不再往回读"
    );
}

#[tokio::test]
async fn 聊天服务器上加了密说加了密_读不出来说暂时不行() {
    let harness = ExcerptHarness::new(&[]);
    *harness.reader.events.lock().unwrap() = vec![chatter(0, 1)];
    harness.reader.state.lock().unwrap().push(timeline_event(
        "$encryption:matrix.test",
        SERVICE_USER,
        "m.room.encryption",
        Some(""),
        30,
        json!({"algorithm": "m.megolm.v1.aes-sha2"}),
    ));
    assert_eq!(
        harness.read("$chat-0:matrix.test").await,
        ExcerptTimelineRead::Encrypted
    );

    let harness = ExcerptHarness::new(&[]);
    *harness.reader.failing.lock().unwrap() = true;
    assert!(
        harness
            .timeline
            .visible_messages(
                &MatrixRoomId::new(ROOM).unwrap(),
                &MatrixEventId::new("$chat-0:matrix.test").unwrap(),
            )
            .await
            .is_err()
    );
}
