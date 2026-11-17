//! 打开分享的一段（[`RoomExcerptPages`]，3P-7a-2）：只给存着的那几句，撤回、隐藏、早于开实况的不给，改过的给
//! 改后的；读不到第一句、那几句都没了是“看不了了”；查不到、加了密和没有一样；读到的留 60 秒。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ExcerptPageRecord, MatrixEventId, MatrixRoomId, MatrixTimelineEvent, PortFuture,
        RoomExcerptPageLookup,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId, room_excerpts::RoomExcerptCode, room_listing::RoomShareCode,
    time::UtcMillis,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    Agent, FakeClock, FakeLookup, FakeSignatures, KnownInstances, NOW, SERVICE_USER, agent_chat,
    excerpts::PagedReader, human_chat, live_rooms::FakePeople, moderation_notice, revision,
    room_state, timeline_event,
};
use crate::public_watch::{
    ExcerptAuthor, ExcerptPage, ExcerptPageFailure, RoomExcerptPageSource, RoomExcerptPages,
    RoomExcerptPagesDependencies, ShownExcerpt,
};

const SHARE_CODE: &str = "Liv3Ab";
const EXCERPT_CODE: &str = "Seg8Ab3x";
/// 实况是 100 秒前打开的。
const ENABLED_SECONDS_AGO: i64 = 100;

/// 库里的一段：按分享码和段码，查到的就是 `record`。
#[derive(Default)]
struct FakeExcerpts {
    record: Mutex<Option<ExcerptPageRecord>>,
    failing: Mutex<bool>,
    lookups: AtomicUsize,
}

impl RoomExcerptPageLookup for FakeExcerpts {
    fn excerpt_page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<ExcerptPageRecord>>> {
        assert_eq!(share_code.as_str(), SHARE_CODE);
        assert_eq!(code.as_str(), EXCERPT_CODE);
        self.lookups.fetch_add(1, Ordering::SeqCst);
        let answer = if *self.failing.lock().unwrap() {
            Err(RepositoryError::new(
                "room_excerpt.page",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self.record.lock().unwrap().clone())
        };
        Box::pin(async move { answer })
    }
}

struct PageHarness {
    pages: RoomExcerptPages,
    reader: Arc<PagedReader>,
    excerpts: Arc<FakeExcerpts>,
    clock: Arc<FakeClock>,
}

impl PageHarness {
    /// `owners` 里的本机 Agent 有主人；`network` 里的是网络 Agent。
    fn new(agents: &[&Agent], owners: &[(&Agent, &'static str)], network: &[&Agent]) -> Self {
        let reader = Arc::new(PagedReader::default());
        *reader.state.lock().unwrap() = room_state(
            &agents
                .iter()
                .map(|agent| agent.matrix_user())
                .collect::<Vec<_>>(),
        );
        let excerpts = Arc::new(FakeExcerpts::default());
        let clock = Arc::new(FakeClock(Mutex::new(NOW)));
        let pages = RoomExcerptPages::new(RoomExcerptPagesDependencies {
            lookup: excerpts.clone(),
            people: Arc::new(FakePeople {
                owners: owners
                    .iter()
                    .map(|(agent, owner)| (agent.id, *owner))
                    .collect::<HashMap<_, _>>(),
                asks: Mutex::default(),
            }),
            network_agents: Arc::new(FakeLookup {
                network: network.iter().map(|agent| agent.id).collect(),
                failing: false,
            }),
            reader: reader.clone(),
            verification: Arc::new(KnownInstances {
                agents: agents
                    .iter()
                    .map(|agent| (agent.id, agent.instance_id))
                    .collect(),
                lookups: AtomicUsize::new(0),
            }),
            signatures: Arc::new(FakeSignatures),
            clock: clock.clone(),
        });
        Self {
            pages,
            reader,
            excerpts,
            clock,
        }
    }

    /// 库里记着这一段：这几句，房间“Rust 新手村”。
    fn share(&self, event_ids: &[&str]) {
        *self.excerpts.record.lock().unwrap() = Some(ExcerptPageRecord {
            catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(7)),
            room_name: "Rust 新手村".to_owned(),
            matrix_room_id: MatrixRoomId::new(super::ROOM).unwrap(),
            enabled_at: UtcMillis::new(NOW - ENABLED_SECONDS_AGO * 1_000).unwrap(),
            event_ids: event_ids
                .iter()
                .map(|id| MatrixEventId::new(*id).unwrap())
                .collect(),
            first_event_at: UtcMillis::new(NOW - 90_000).unwrap(),
        });
    }

    async fn open(&self) -> Result<Arc<ExcerptPage>, ExcerptPageFailure> {
        self.pages
            .page(
                &RoomShareCode::parse(SHARE_CODE).unwrap(),
                &RoomExcerptCode::parse(EXCERPT_CODE).unwrap(),
            )
            .await
    }

    async fn shown(&self) -> ShownExcerpt {
        match self.open().await.expect("打得开").as_ref() {
            ExcerptPage::Shown(excerpt) => excerpt.clone(),
            ExcerptPage::Gone { .. } => panic!("应该看得到"),
        }
    }

    fn pages_read(&self) -> usize {
        self.reader.pages.load(Ordering::SeqCst)
    }
}

/// 一段热闹的房间：开实况前一句，那一段（人、本机 Agent、网络 Agent、撤回的、改过的、被隐藏的），之后一句。
fn busy_room(harness: &PageHarness, scout: &Agent, ranger: &Agent) {
    let withdrawn = Uuid::now_v7();
    let edited = Uuid::now_v7();
    *harness.reader.events.lock().unwrap() = vec![
        human_chat("$before:matrix.test", Uuid::now_v7(), "之前", None, 300),
        human_chat("$h1:matrix.test", Uuid::now_v7(), "<b>你好</b>", None, 90),
        agent_chat("$s1:matrix.test", scout, Uuid::now_v7(), "我是 Scout", 80),
        agent_chat("$r1:matrix.test", ranger, Uuid::now_v7(), "我是网络的", 70),
        agent_chat("$w1:matrix.test", scout, withdrawn, "撤回的", 60),
        agent_chat("$e1:matrix.test", scout, edited, "改之前", 50),
        agent_chat("$x1:matrix.test", scout, Uuid::now_v7(), "被隐藏", 45),
        revision("$rd:matrix.test", scout, withdrawn, "redact", 40),
        revision("$rp:matrix.test", scout, edited, "replace", 39),
        human_chat("$after:matrix.test", Uuid::now_v7(), "之后", None, 30),
    ];
    harness
        .reader
        .state
        .lock()
        .unwrap()
        .push(moderation_notice(SERVICE_USER, "$x1:matrix.test"));
}

#[tokio::test]
async fn 只给存着的那几句_撤回隐藏的不给_改过的给改后的_写上是谁的_agent() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #1000");
    let harness = PageHarness::new(&[&scout, &ranger], &[(&scout, "阿明")], &[&ranger]);
    busy_room(&harness, &scout, &ranger);
    harness.share(&[
        "$h1:matrix.test",
        "$s1:matrix.test",
        "$r1:matrix.test",
        "$w1:matrix.test",
        "$e1:matrix.test",
        "$x1:matrix.test",
    ]);

    let excerpt = harness.shown().await;

    assert_eq!(excerpt.room_name, "Rust 新手村");
    let lines: Vec<(&str, &ExcerptAuthor, &str, bool, i64)> = excerpt
        .lines
        .iter()
        .map(|line| {
            (
                line.name.as_str(),
                &line.author,
                line.text.as_str(),
                line.edited,
                line.sent_at_unix_ms,
            )
        })
        .collect();
    let scout_of_aming = ExcerptAuthor::Agent {
        owner: Some("阿明".to_owned()),
    };
    assert_eq!(
        lines,
        [
            (
                "小雨",
                &ExcerptAuthor::Person,
                "<b>你好</b>",
                false,
                NOW - 90_000
            ),
            (
                "Scout #2316",
                &scout_of_aming,
                "我是 Scout",
                false,
                NOW - 80_000
            ),
            (
                "Ranger #1000",
                &ExcerptAuthor::NetworkAgent,
                "我是网络的",
                false,
                NOW - 70_000
            ),
            (
                "Scout #2316",
                &scout_of_aming,
                "改过的话",
                true,
                NOW - 50_000
            ),
        ],
        "原样交出去（转义是页面的事）；时间是原来那一句的"
    );
}

#[tokio::test]
async fn 读到的留_60_秒_库里查到的留_5_秒() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #1000");
    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);
    harness.share(&["$h1:matrix.test", "$s1:matrix.test"]);

    assert_eq!(harness.shown().await.lines.len(), 2);
    assert_eq!(harness.shown().await.lines.len(), 2);
    assert_eq!(harness.pages_read(), 1);
    assert_eq!(harness.excerpts.lookups.load(Ordering::SeqCst), 1);

    harness.clock.advance(6_000);
    harness.shown().await;
    assert_eq!(
        harness.excerpts.lookups.load(Ordering::SeqCst),
        2,
        "过了 5 秒再查一次库：关了实况马上就打不开"
    );
    assert_eq!(harness.pages_read(), 1, "读到的还在 60 秒里");

    *harness.excerpts.record.lock().unwrap() = None;
    harness.clock.advance(6_000);
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::NotFound
    );

    harness.share(&["$h1:matrix.test", "$s1:matrix.test"]);
    harness.clock.advance(60_000);
    harness.shown().await;
    assert_eq!(harness.pages_read(), 2, "过了 60 秒重读");
}

#[tokio::test]
async fn 读不到第一句了_那几句都没了_是看不了了() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #1000");
    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);

    harness.share(&["$long-gone:matrix.test", "$h1:matrix.test"]);
    let page = harness.open().await.unwrap();
    assert_eq!(
        *page,
        ExcerptPage::Gone {
            room_name: "Rust 新手村".to_owned()
        }
    );

    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);
    harness.share(&["$w1:matrix.test", "$x1:matrix.test"]);
    assert!(
        matches!(*harness.open().await.unwrap(), ExcerptPage::Gone { .. }),
        "撤回的和被隐藏的都不算"
    );
}

#[tokio::test]
async fn 查不到_加了密都和没有一样_早于开实况的话不给() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #1000");
    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::NotFound
    );
    assert_eq!(harness.pages_read(), 0, "查不到就不读聊天服务器");

    harness.share(&["$before:matrix.test", "$h1:matrix.test"]);
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::NotFound,
        "查不到也记 5 秒"
    );
    harness.clock.advance(6_000);
    let texts: Vec<String> = harness
        .shown()
        .await
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert_eq!(texts, ["<b>你好</b>"], "开实况以前说的不给");

    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);
    harness.reader.state.lock().unwrap().push(encryption());
    harness.share(&["$h1:matrix.test"]);
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::NotFound
    );
}

#[tokio::test]
async fn 库和聊天服务器不通时暂时打不开_通了照常() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #1000");
    let harness = PageHarness::new(&[&scout, &ranger], &[], &[]);
    busy_room(&harness, &scout, &ranger);
    harness.share(&["$h1:matrix.test"]);

    *harness.excerpts.failing.lock().unwrap() = true;
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::Unavailable
    );
    *harness.excerpts.failing.lock().unwrap() = false;

    *harness.reader.failing.lock().unwrap() = true;
    assert_eq!(
        harness.open().await.unwrap_err(),
        ExcerptPageFailure::Unavailable
    );
    *harness.reader.failing.lock().unwrap() = false;
    assert_eq!(harness.shown().await.lines.len(), 1, "没读成的不记着");
}

fn encryption() -> MatrixTimelineEvent {
    timeline_event(
        "$encryption:matrix.test",
        SERVICE_USER,
        "m.room.encryption",
        Some(""),
        30,
        json!({"algorithm": "m.megolm.v1.aes-sha2"}),
    )
}
