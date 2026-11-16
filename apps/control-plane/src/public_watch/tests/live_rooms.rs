//! 分享页的实况（[`RoomLiveWatch`]）：按分享码找房间，只给开实况以后说的，几样不满足就不放。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        LiveRoomLookup, LiveRoomRecord, MatrixRoomId, MatrixTimelineEvent, PortFuture,
        PublicRoomRecord, RoomAgentKind, RoomAgentOwnerRecord, RoomPeopleDirectory, RoomPersonName,
    },
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicDirectoryResult, PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomOnline,
        PublicRoomPage, PublicRoomView,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    Agent, FakeClock, FakeLookup, FakeReader, FakeSignatures, HUMAN, KnownInstances, Lobby, NOW,
    ROOM, SERVICE_USER, agent_chat, human_chat, moderation_notice, revision, room_state, texts,
    timeline_event,
};
use crate::public_watch::{
    RoomLiveFailure, RoomLiveSource, RoomLiveWatch, RoomLiveWatchDependencies,
};

const CODE: &str = "Liv3Ab";
/// 实况是 100 秒前打开的。
const ENABLED_SECONDS_AGO: i64 = 100;

fn code() -> RoomShareCode {
    RoomShareCode::parse(CODE).unwrap()
}

fn live_room(catalog: u128, enabled_seconds_ago: i64) -> LiveRoomRecord {
    LiveRoomRecord {
        catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(catalog)),
        matrix_room_id: MatrixRoomId::new(ROOM).unwrap(),
        enabled_at: UtcMillis::new(NOW - enabled_seconds_ago * 1_000).unwrap(),
    }
}

/// 放着实况的房间，按分享码；查不到的是不放。
#[derive(Default)]
struct FakeLiveRooms {
    rooms: Mutex<HashMap<String, LiveRoomRecord>>,
    asks: AtomicUsize,
    failing: Mutex<bool>,
}

impl FakeLiveRooms {
    fn set(&self, code: &str, room: Option<LiveRoomRecord>) {
        let mut rooms = self.rooms.lock().unwrap();
        match room {
            Some(room) => rooms.insert(code.to_owned(), room),
            None => rooms.remove(code),
        };
    }

    fn asks(&self) -> usize {
        self.asks.load(Ordering::SeqCst)
    }
}

impl LiveRoomLookup for FakeLiveRooms {
    fn live_room<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<LiveRoomRecord>>> {
        self.asks.fetch_add(1, Ordering::SeqCst);
        let answer = if *self.failing.lock().unwrap() {
            Err(RepositoryError::new(
                "room_live.lookup",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self.rooms.lock().unwrap().get(share_code.as_str()).cloned())
        };
        Box::pin(async move { answer })
    }
}

/// 分享页卡片上的那份目录：在线数；`gone` 时说没有这间房（不公开了）。
#[derive(Default)]
struct FakeRoomDirectory {
    online: Mutex<Option<PublicRoomOnline>>,
    gone: Mutex<bool>,
    asks: AtomicUsize,
}

impl PublicRoomDirectoryUseCases for FakeRoomDirectory {
    fn browse(
        &self,
        _request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        unreachable!("实况不翻目录")
    }

    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        assert!(viewer.is_none(), "实况不看登录");
        self.asks.fetch_add(1, Ordering::SeqCst);
        let answer = if *self.gone.lock().unwrap() {
            Err(PublicDirectoryFailure::new(
                "test",
                PublicDirectoryFailureKind::NotFound,
            ))
        } else {
            Ok(PublicRoomDetailView {
                room: PublicRoomView {
                    room: PublicRoomRecord {
                        catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(1)),
                        name: "rainyflash/agent-room".to_owned(),
                        description: String::new(),
                        owner_display_name: "阿青".to_owned(),
                        member_count: 3,
                        agent_count: 2,
                        join_mode: RoomJoinMode::Open,
                        end_to_end_encrypted: false,
                        share_code: RoomShareCode::parse(&share_code).unwrap(),
                        created_at: UtcMillis::new(NOW - 86_400_000).unwrap(),
                        last_message_at: None,
                    },
                    viewer: None,
                },
                admin_display_names: Vec::new(),
                online: *self.online.lock().unwrap(),
            })
        };
        Box::pin(async move { answer })
    }
}

/// 本机 Agent 是谁的：`owners` 里的在房间里，别的不回答。打开分享的一段（`excerpt_pages.rs`）也用它。
#[derive(Default)]
pub(super) struct FakePeople {
    pub(super) owners: HashMap<Uuid, &'static str>,
    pub(super) asks: Mutex<Vec<Vec<AgentId>>>,
}

impl RoomPeopleDirectory for FakePeople {
    fn member_names(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomPersonName>>> {
        unreachable!("实况不列成员")
    }

    fn agent_owners<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        agent_ids: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<RoomAgentOwnerRecord>>> {
        self.asks.lock().unwrap().push(agent_ids.to_vec());
        let records = agent_ids
            .iter()
            .filter_map(|id| {
                self.owners
                    .get(&id.as_uuid())
                    .map(|owner| RoomAgentOwnerRecord {
                        agent_id: *id,
                        kind: RoomAgentKind::Local,
                        owner: Some(RoomPersonName {
                            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
                            display_name: (*owner).to_owned(),
                        }),
                        admitted_by: None,
                    })
            })
            .collect();
        Box::pin(async move { Ok(records) })
    }
}

struct LiveHarness {
    watch: Arc<RoomLiveWatch>,
    reader: Arc<FakeReader>,
    rooms: Arc<FakeLiveRooms>,
    directory: Arc<FakeRoomDirectory>,
    people: Arc<FakePeople>,
    clock: Arc<FakeClock>,
}

impl LiveHarness {
    /// `agents` 是登记过的实例，其中 `network` 是网络 Agent；`owners` 是本机 Agent 的主人。
    fn new(agents: &[&Agent], network: &[&Agent], owners: &[(&Agent, &'static str)]) -> Self {
        let reader = Arc::new(FakeReader::default());
        let rooms = Arc::new(FakeLiveRooms::default());
        rooms.set(CODE, Some(live_room(1, ENABLED_SECONDS_AGO)));
        let directory = Arc::new(FakeRoomDirectory::default());
        let people = Arc::new(FakePeople {
            owners: owners
                .iter()
                .map(|(agent, owner)| (agent.id, *owner))
                .collect(),
            asks: Mutex::new(Vec::new()),
        });
        let clock = Arc::new(FakeClock(Mutex::new(NOW)));
        let watch = RoomLiveWatch::new(RoomLiveWatchDependencies {
            rooms: rooms.clone(),
            directory: directory.clone(),
            people: people.clone(),
            reader: reader.clone(),
            verification: Arc::new(KnownInstances {
                agents: agents
                    .iter()
                    .map(|agent| (agent.id, agent.instance_id))
                    .collect(),
                lookups: AtomicUsize::new(0),
            }),
            signatures: Arc::new(FakeSignatures),
            network_agents: Arc::new(FakeLookup {
                network: network.iter().map(|agent| agent.id).collect(),
                failing: false,
            }),
            secrets: Arc::new(SecureSecretFactory),
            clock: clock.clone(),
        })
        .unwrap();
        Self {
            watch: Arc::new(watch),
            reader,
            rooms,
            directory,
            people,
            clock,
        }
    }

    async fn view(&self) -> Value {
        let body = self.watch.snapshot(&code()).await.expect("放着实况");
        serde_json::from_slice(&body).expect("快照是 JSON")
    }

    async fn refused(&self) -> RoomLiveFailure {
        self.watch
            .snapshot(&code())
            .await
            .expect_err("这次不给快照")
    }

    fn reads(&self) -> usize {
        self.reader.reads.load(Ordering::SeqCst)
    }

    fn state_reads(&self) -> usize {
        self.reader.state_reads.load(Ordering::SeqCst)
    }
}

fn encryption_event() -> MatrixTimelineEvent {
    timeline_event(
        "$encryption:matrix.test",
        SERVICE_USER,
        "m.room.encryption",
        Some(""),
        30,
        json!({"algorithm": "m.megolm.v1.aes-sha2"}),
    )
}

#[tokio::test]
async fn 只给开实况以后说的_最近_20_句_每句最多_280_个字() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[]);
    let mut events = vec![agent_chat(
        "$before:matrix.test",
        &scout,
        Uuid::now_v7(),
        "开实况以前说的",
        ENABLED_SECONDS_AGO + 50,
    )];
    for number in 1..=20_i64 {
        events.push(agent_chat(
            &format!("$after-{number}:matrix.test"),
            &scout,
            Uuid::now_v7(),
            &format!("第 {number} 句"),
            91 - number,
        ));
    }
    events.push(agent_chat(
        "$long:matrix.test",
        &scout,
        Uuid::now_v7(),
        &"长".repeat(300),
        10,
    ));
    harness.reader.set(Lobby {
        events,
        state: room_state(&[scout.matrix_user()]),
    });

    let view = harness.view().await;

    let shown = texts(&view);
    assert_eq!(shown.len(), 20);
    assert_eq!(shown[0], "第 2 句", "最早的那句挤掉了");
    assert!(!shown.contains(&"开实况以前说的"), "开实况以前说的不给");
    assert_eq!(shown[19].chars().count(), 280);
    assert_eq!(view["messages"][19]["truncated"], true);
    assert_eq!(view["messages"][0]["truncated"], false);
    assert_eq!(view["schemaVersion"], 1);
    assert_eq!(view["updatedAtUnixMs"], NOW);
}

#[tokio::test]
async fn 说话的写名字_本机_agent_写是谁的_网络_agent_不写_带在线数_不带_matrix_id() {
    let scout = Agent::new("Scout #2316");
    let ranger = Agent::new("Ranger #0042");
    let stray = Agent::new("Stray #7777");
    let harness = LiveHarness::new(
        &[&scout, &ranger, &stray],
        &[&ranger],
        &[(&scout, "阿青@example.com")],
    );
    *harness.directory.online.lock().unwrap() = Some(PublicRoomOnline {
        count: 7,
        at_least: false,
    });
    harness.reader.set(Lobby {
        events: vec![
            agent_chat("$scout:matrix.test", &scout, Uuid::now_v7(), "早", 50),
            agent_chat("$ranger:matrix.test", &ranger, Uuid::now_v7(), "好", 40),
            agent_chat("$stray:matrix.test", &stray, Uuid::now_v7(), "嗨", 30),
            human_chat("$human:matrix.test", Uuid::now_v7(), "大家好", None, 20),
        ],
        state: room_state(&[scout.matrix_user(), ranger.matrix_user(), HUMAN.to_owned()]),
    });

    let view = harness.view().await;

    assert_eq!(view["online"], json!({"count": 7, "atLeast": false}));
    let participants: Vec<(&str, &str, Value)> = view["participants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|participant| {
            (
                participant["name"].as_str().unwrap(),
                participant["kind"].as_str().unwrap(),
                participant["owner"].clone(),
            )
        })
        .collect();
    assert_eq!(
        participants,
        [
            ("小雨", "person", Value::Null),
            ("Stray #7777", "agent", Value::Null),
            ("Ranger #0042", "networkAgent", Value::Null),
            ("Scout #2316", "agent", json!("阿青")),
        ],
        "新说话的在前；主人的名字只留 @ 前面那段；问不到主人的不写"
    );
    let body = view.to_string();
    for hidden in [
        "matrix.test",
        &scout.id.to_string(),
        &scout.instance_id.to_string(),
    ] {
        assert!(!body.contains(hidden), "快照里不该有 {hidden}");
    }
    let mut local = vec![AgentId::from_uuid(scout.id), AgentId::from_uuid(stray.id)];
    local.sort_unstable();
    assert_eq!(
        *harness.people.asks.lock().unwrap(),
        [local],
        "网络 Agent 不问主人"
    );
}

#[tokio::test]
async fn 被隐藏和撤回的不给() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[]);
    let withdrawn = Uuid::now_v7();
    let mut state = room_state(&[scout.matrix_user()]);
    state.push(moderation_notice(SERVICE_USER, "$hidden:matrix.test"));
    harness.reader.set(Lobby {
        events: vec![
            agent_chat(
                "$hidden:matrix.test",
                &scout,
                Uuid::now_v7(),
                "被隐藏的",
                50,
            ),
            agent_chat("$withdrawn:matrix.test", &scout, withdrawn, "撤回的", 40),
            agent_chat("$kept:matrix.test", &scout, Uuid::now_v7(), "留着的", 30),
            revision("$redact:matrix.test", &scout, withdrawn, "redact", 20),
        ],
        state,
    });

    assert_eq!(texts(&harness.view().await), ["留着的"]);
}

#[tokio::test]
async fn 不放实况的码说不放_查过的五秒内不再查_查不了的不记() {
    let harness = LiveHarness::new(&[], &[], &[]);
    harness.rooms.set(CODE, None);

    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert!(harness.watch.knows(&code()));
    assert!(!harness.watch.is_live(&code()).await);
    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert_eq!(harness.rooms.asks(), 1, "五秒内用上次查到的");
    harness.clock.advance(5_000);
    assert!(!harness.watch.knows(&code()));
    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert_eq!(harness.rooms.asks(), 2);
    assert_eq!(harness.reads(), 0, "不放的房间不读聊天服务器");

    harness.clock.advance(5_000);
    *harness.rooms.failing.lock().unwrap() = true;
    assert_eq!(harness.refused().await, RoomLiveFailure::Unavailable);
    assert!(!harness.watch.knows(&code()), "查不了的不记，下次再查");
}

#[tokio::test]
async fn 关了以后五秒内就停_再开是新的一刻() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[]);
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$first:matrix.test",
            &scout,
            Uuid::now_v7(),
            "开着时说的",
            90,
        )],
        state: room_state(&[scout.matrix_user()]),
    });
    assert_eq!(texts(&harness.view().await), ["开着时说的"]);

    harness.rooms.set(CODE, None);
    harness.clock.advance(4_999);
    assert!(harness.watch.is_live(&code()).await, "五秒内还用上次查到的");
    harness.clock.advance(1);
    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);

    // 再开：那一刻是现在，以前说的不再给。
    harness.rooms.set(CODE, Some(live_room(1, -5)));
    harness.reader.set(Lobby {
        events: vec![
            agent_chat(
                "$first:matrix.test",
                &scout,
                Uuid::now_v7(),
                "开着时说的",
                90,
            ),
            agent_chat(
                "$second:matrix.test",
                &scout,
                Uuid::now_v7(),
                "再开以后说的",
                -6,
            ),
        ],
        state: room_state(&[scout.matrix_user()]),
    });
    harness.clock.advance(5_000);
    assert!(harness.watch.is_live(&code()).await);
    assert_eq!(texts(&harness.view().await), ["再开以后说的"]);
}

#[tokio::test]
async fn 聊天服务器上加了密就不放_以后也不再读() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[]);
    let mut state = room_state(&[scout.matrix_user()]);
    state.push(encryption_event());
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$secret:matrix.test",
            &scout,
            Uuid::now_v7(),
            "加密以后说的",
            10,
        )],
        state,
    });

    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert!(!harness.watch.is_live(&code()).await);
    harness.clock.advance(10_000);
    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert!(!harness.watch.is_live(&code()).await);
    assert_eq!(harness.state_reads(), 1, "认出加了密以后不再读");
}

#[tokio::test]
async fn 目录说房间不公开了就不放() {
    let harness = LiveHarness::new(&[], &[], &[]);
    harness.reader.set(Lobby {
        events: Vec::new(),
        state: room_state(&[]),
    });
    *harness.directory.gone.lock().unwrap() = true;

    assert_eq!(harness.refused().await, RoomLiveFailure::NotLive);
    assert!(harness.watch.knows(&code()));
    assert!(!harness.watch.is_live(&code()).await);
}

#[tokio::test]
async fn 三秒内直接给快照_房间状态和主人三十秒才再问() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[(&scout, "阿青")]);
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$hi:matrix.test",
            &scout,
            Uuid::now_v7(),
            "早",
            10,
        )],
        state: room_state(&[scout.matrix_user()]),
    });
    let asked = |harness: &LiveHarness| {
        (
            harness.reads(),
            harness.state_reads(),
            harness.directory.asks.load(Ordering::SeqCst),
            harness.people.asks.lock().unwrap().len(),
        )
    };

    harness.view().await;
    assert_eq!(asked(&harness), (1, 1, 1, 1));
    harness.clock.advance(2_999);
    harness.view().await;
    assert_eq!(asked(&harness), (1, 1, 1, 1), "三秒内直接给快照");
    harness.clock.advance(1);
    harness.view().await;
    assert_eq!(
        asked(&harness),
        (2, 1, 1, 1),
        "消息三秒一读，房间状态和主人记着"
    );
    harness.clock.advance(30_000);
    let view = harness.view().await;
    assert_eq!(
        asked(&harness),
        (3, 2, 2, 2),
        "三十秒以后房间状态和主人一起再问"
    );
    assert_eq!(view["participants"][0]["owner"], "阿青");
}

#[tokio::test]
async fn 读不到时旧快照最多再给一分钟() {
    let scout = Agent::new("Scout #2316");
    let harness = LiveHarness::new(&[&scout], &[], &[]);
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$hi:matrix.test",
            &scout,
            Uuid::now_v7(),
            "早",
            10,
        )],
        state: room_state(&[scout.matrix_user()]),
    });
    harness.view().await;
    harness.reader.fail(true);

    harness.clock.advance(3_000);
    assert_eq!(texts(&harness.view().await), ["早"]);
    harness.clock.advance(57_000);
    assert_eq!(texts(&harness.view().await), ["早"]);
    harness.clock.advance(1);
    assert_eq!(harness.refused().await, RoomLiveFailure::Unavailable);
}

#[tokio::test]
async fn 最多留_200_间房的快照_久没人看的先扔() {
    let harness = LiveHarness::new(&[], &[], &[]);
    harness.reader.set(Lobby {
        events: Vec::new(),
        state: room_state(&[]),
    });
    let codes: Vec<String> = (0..=200).map(|number| format!("Rm{number:04}")).collect();
    for (number, room_code) in (1_u128..).zip(&codes) {
        harness.rooms.set(
            room_code,
            Some(live_room(number + 100, ENABLED_SECONDS_AGO)),
        );
    }
    let look = |room_code: &str| {
        let watch = harness.watch.clone();
        let room_code = RoomShareCode::parse(room_code).unwrap();
        async move { watch.snapshot(&room_code).await.expect("放着实况") }
    };

    for room_code in &codes {
        look(room_code).await;
    }
    assert_eq!(harness.reads(), 201);
    look(&codes[200]).await;
    assert_eq!(harness.reads(), 201, "最近看过的还留着");
    look(&codes[0]).await;
    assert_eq!(harness.reads(), 202, "最早看的那间扔掉了，要重读");
}
