use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRecord,
        AgentInstanceVerificationRepository, Clock, DeviceSignature, MatrixEventId,
        MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
        MatrixRoomId, MatrixTimelineEvent, MatrixUserId, NetworkAgentLookup, PortFuture,
        PublicLobbyDirectoryEntry, PublicLobbyMatrixReader, PublicLobbyObservationRoom,
        RoomDirectory, RoomDirectoryQuery,
    },
};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility, RoomSlug,
    },
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{PublicWatch, PublicWatchDependencies, PublicWatchFailure};
use crate::features::public_watch::{PublicWatchHttpState, router};

/// 2026-10-20T08:00:00Z。
const NOW: i64 = 1_792_483_200_000;
const ROOM: &str = "!lobby:matrix.test";
const SERVICE_USER: &str = "@_agent_room:matrix.test";
const HUMAN: &str = "@xiaoyu:matrix.test";
const FORGED_SIGNATURE: [u8; 64] = [9; 64];

/// 测试里的一个 Agent：登记过的实例、Matrix 用户和名字。
struct Agent {
    id: Uuid,
    instance_id: Uuid,
    name: &'static str,
}

impl Agent {
    fn new(name: &'static str) -> Self {
        Self {
            id: Uuid::now_v7(),
            instance_id: Uuid::now_v7(),
            name,
        }
    }

    fn matrix_user(&self) -> String {
        format!("@_agent_{}:matrix.test", self.id.simple())
    }

    fn actor(&self) -> Value {
        json!({
            "agent": {
                "agentId": self.id,
                "displayName": self.name,
                "matrixUserId": self.matrix_user(),
            },
            "instanceId": self.instance_id,
            "provenance": "autonomous_agent",
        })
    }
}

/// 一段读到的大厅：消息事件（旧的在前）和房间状态。
#[derive(Clone, Default)]
struct Lobby {
    events: Vec<MatrixTimelineEvent>,
    state: Vec<MatrixTimelineEvent>,
}

#[derive(Default)]
struct FakeReader {
    lobby: Mutex<Lobby>,
    failing: Mutex<bool>,
    reads: AtomicUsize,
}

impl FakeReader {
    fn set(&self, lobby: Lobby) {
        *self.lobby.lock().unwrap() = lobby;
    }

    fn fail(&self, failing: bool) {
        *self.failing.lock().unwrap() = failing;
    }

    fn answer<T>(&self, pick: impl FnOnce(&Lobby) -> T) -> MatrixResult<T> {
        if *self.failing.lock().unwrap() {
            return Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::DependencyUnavailable,
            ));
        }
        Ok(pick(&self.lobby.lock().unwrap()))
    }
}

impl PublicLobbyMatrixReader for FakeReader {
    fn recent_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        limit: u16,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        assert_eq!(room_id.as_str(), ROOM);
        assert_eq!(limit, 60);
        self.reads.fetch_add(1, Ordering::SeqCst);
        let events = self.answer(|lobby| lobby.events.clone());
        Box::pin(async move { events })
    }

    fn current_state<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        assert_eq!(room_id.as_str(), ROOM);
        let state = self.answer(|lobby| lobby.state.clone());
        Box::pin(async move { state })
    }
}

struct FakeDirectory {
    lobbies: Vec<PublicLobbyDirectoryEntry>,
    /// 公开大厅有没有分片（有没有人进过）。
    instance: bool,
    listings: AtomicUsize,
}

impl FakeDirectory {
    fn new(slugs: &[&str]) -> Self {
        Self {
            lobbies: slugs.iter().map(|slug| lobby_entry(slug)).collect(),
            instance: true,
            listings: AtomicUsize::new(0),
        }
    }
}

impl RoomDirectory for FakeDirectory {
    fn list_public<'a>(
        &'a self,
        _query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        let lobbies = self.lobbies.clone();
        Box::pin(async move { Ok(lobbies) })
    }

    fn find_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
        unreachable!("围观不按编号查目录")
    }

    fn find_public_observation_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
        let room = self.instance.then(|| PublicLobbyObservationRoom {
            catalog_id,
            room_instance_id: RoomInstanceId::from_uuid(Uuid::now_v7()),
            matrix_room_id: MatrixRoomReference::new(ROOM.to_owned()).unwrap(),
        });
        Box::pin(async move { Ok(room) })
    }
}

/// 登记过的实例：`agents` 里的都在，公钥随便给，验签由 [`FakeSignatures`] 决定。
struct KnownInstances {
    agents: Vec<(Uuid, Uuid)>,
    lookups: AtomicUsize,
}

impl AgentInstanceVerificationRepository for KnownInstances {
    fn find_verification_record(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        let record = self
            .agents
            .iter()
            .find(|(_, instance)| *instance == instance_id.as_uuid())
            .map(|(agent, _)| AgentInstanceVerificationRecord {
                instance_id,
                agent_id: AgentId::from_uuid(*agent),
                public_signing_key: AgentInstancePublicSigningKey::new(vec![7; 32]).unwrap(),
                registered_at: UtcMillis::new(1).unwrap(),
                invalidated_at: None,
            });
        Box::pin(async move { Ok(record) })
    }
}

struct FakeSignatures;

impl AgentInstanceSignatureVerifier for FakeSignatures {
    fn verify(
        &self,
        _public_key: &AgentInstancePublicSigningKey,
        _signed_message: &[u8],
        signature: &DeviceSignature,
    ) -> bool {
        signature.as_bytes() != &FORGED_SIGNATURE
    }
}

struct FakeLookup {
    network: Vec<Uuid>,
    failing: bool,
}

impl NetworkAgentLookup for FakeLookup {
    fn network_agent_ids<'a>(
        &'a self,
        candidates: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<AgentId>>> {
        let found = if self.failing {
            Err(RepositoryError::new(
                "network_agent.lookup",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(candidates
                .iter()
                .filter(|id| self.network.contains(&id.as_uuid()))
                .copied()
                .collect())
        };
        Box::pin(async move { found })
    }
}

struct FakeClock(Mutex<i64>);

impl FakeClock {
    fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(*self.0.lock().unwrap()).unwrap()
    }
}

struct Harness {
    watch: Arc<PublicWatch>,
    reader: Arc<FakeReader>,
    directory: Arc<FakeDirectory>,
    clock: Arc<FakeClock>,
}

struct Setup {
    enabled: bool,
    directory: FakeDirectory,
    agents: Vec<(Uuid, Uuid)>,
    network: Vec<Uuid>,
    lookup_failing: bool,
}

impl Setup {
    fn new(agents: &[&Agent]) -> Self {
        Self {
            enabled: true,
            directory: FakeDirectory::new(&["agent-room-global"]),
            agents: agents
                .iter()
                .map(|agent| (agent.id, agent.instance_id))
                .collect(),
            network: Vec::new(),
            lookup_failing: false,
        }
    }

    fn build(self) -> Harness {
        let reader = Arc::new(FakeReader::default());
        let directory = Arc::new(self.directory);
        let clock = Arc::new(FakeClock(Mutex::new(NOW)));
        let watch = PublicWatch::new(PublicWatchDependencies {
            enabled: self.enabled,
            directory: directory.clone(),
            reader: reader.clone(),
            verification: Arc::new(KnownInstances {
                agents: self.agents,
                lookups: AtomicUsize::new(0),
            }),
            signatures: Arc::new(FakeSignatures),
            network_agents: Arc::new(FakeLookup {
                network: self.network,
                failing: self.lookup_failing,
            }),
            secrets: Arc::new(SecureSecretFactory),
            clock: clock.clone(),
        })
        .unwrap();
        Harness {
            watch: Arc::new(watch),
            reader,
            directory,
            clock,
        }
    }
}

impl Harness {
    async fn view(&self, slug: &str) -> Value {
        let body = self.watch.watch(slug).await.expect("看得到这个大厅");
        serde_json::from_slice(&body).expect("快照是 JSON")
    }
}

fn lobby_entry(slug: &str) -> PublicLobbyDirectoryEntry {
    PublicLobbyDirectoryEntry {
        catalog: RoomCatalog::new(
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            RoomCatalogFields {
                kind: RoomCatalogKind::PublicLobby,
                slug: Some(RoomSlug::new(slug).unwrap()),
                name: if slug == "agent-room-global" {
                    "Agent Room Global".to_owned()
                } else {
                    format!("Lobby {slug}")
                },
                description: String::new(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: None,
                visibility: RoomCatalogVisibility::Public,
                retention_days: Some(30),
                status: RoomCatalogStatus::Active,
            },
        )
        .unwrap(),
        active_instance_count: 1,
        online_agent_count: 0,
        activity_score_millis: 0,
    }
}

fn signature(bytes: [u8; 64]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn timeline_event(
    event_id: &str,
    sender: &str,
    event_type: &str,
    state_key: Option<&str>,
    seconds_ago: i64,
    content: Value,
) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new(sender).unwrap()),
        MatrixEventType::new(event_type).unwrap(),
        state_key.map(str::to_owned),
        None,
        Some(u64::try_from(NOW - seconds_ago * 1_000).unwrap()),
        content,
    )
    .unwrap()
}

fn preview(text: &str) -> Value {
    json!({
        "conversation": {"text": text, "mentions": []},
        "title": text.chars().take(120).collect::<String>(),
        "summary": text.chars().take(500).collect::<String>(),
        "contentType": "text/plain",
        "sensitivity": "normal",
        "riskFlags": [],
    })
}

fn content_ref(media_type: &str) -> Value {
    json!({
        "contentId": Uuid::now_v7(),
        "digestSha256": "11".repeat(32),
        "sizeBytes": 16,
        "mediaType": media_type,
        "fetchMode": "on_demand",
    })
}

/// Agent 发的一条聊天消息（签了名）。
fn agent_chat(
    event_id: &str,
    from: &Agent,
    id: Uuid,
    text: &str,
    seconds_ago: i64,
) -> MatrixTimelineEvent {
    agent_message(event_id, from, id, &preview(text), seconds_ago, [1; 64])
}

fn agent_message(
    event_id: &str,
    from: &Agent,
    id: Uuid,
    preview: &Value,
    seconds_ago: i64,
    signature_bytes: [u8; 64],
) -> MatrixTimelineEvent {
    let media_type = preview["contentType"].as_str().unwrap().to_owned();
    timeline_event(
        event_id,
        &from.matrix_user(),
        "io.github.rainyflash.agentroom.message.preview.v1",
        None,
        seconds_ago,
        json!({
            "schemaVersion": "1.0",
            "eventType": "io.github.rainyflash.agentroom.message.preview.v1",
            "id": id,
            "createdAt": "2026-10-20T07:59:00.000Z",
            "actor": from.actor(),
            "correlationId": id,
            "roomId": ROOM,
            "preview": preview,
            "content": content_ref(&media_type),
            "signature": signature(signature_bytes),
        }),
    )
}

/// 网页里的人发的一条聊天消息，可以回复某一条。
fn human_chat(
    event_id: &str,
    id: Uuid,
    text: &str,
    reply_to: Option<Uuid>,
    seconds_ago: i64,
) -> MatrixTimelineEvent {
    let mut content = json!({
        "schemaVersion": "2.0",
        "eventType": "io.github.rainyflash.agentroom.message.preview.v2",
        "id": id,
        "createdAt": "2026-10-20T07:59:00.000Z",
        "actor": {
            "kind": "human",
            "principalId": Uuid::now_v7(),
            "displayName": "小雨",
            "matrixUserId": HUMAN,
        },
        "correlationId": id,
        "roomId": ROOM,
        "preview": preview(text),
        "content": content_ref("text/plain"),
    });
    if let Some(target) = reply_to {
        content["relation"] = json!({"kind": "reply", "targetMessageId": target});
    }
    timeline_event(
        event_id,
        HUMAN,
        "io.github.rainyflash.agentroom.message.preview.v2",
        None,
        seconds_ago,
        content,
    )
}

/// Agent 改自己（或想改别人）的一条：`replace` 带新正文，`redact` 撤回。
fn revision(
    event_id: &str,
    from: &Agent,
    target: Uuid,
    kind: &str,
    seconds_ago: i64,
) -> MatrixTimelineEvent {
    let id = Uuid::now_v7();
    let mut content = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.revision.v1",
        "id": id,
        "createdAt": "2026-10-20T07:59:30.000Z",
        "actor": from.actor(),
        "correlationId": id,
        "roomId": ROOM,
        "targetMessageId": target,
        "kind": kind,
        "signature": signature([1; 64]),
    });
    if kind == "replace" {
        content["preview"] = preview("改过的话");
        content["content"] = content_ref("text/plain");
    }
    timeline_event(
        event_id,
        &from.matrix_user(),
        "io.github.rainyflash.agentroom.message.revision.v1",
        None,
        seconds_ago,
        content,
    )
}

/// 一分钟前写的在线状态，租约 5 分钟。
fn status(agent: &Agent) -> MatrixTimelineEvent {
    status_as(agent, "idle")
}

fn status_as(agent: &Agent, work: &str) -> MatrixTimelineEvent {
    timeline_event(
        &format!("$status-{}:matrix.test", agent.instance_id.simple()),
        &agent.matrix_user(),
        "io.github.rainyflash.agentroom.agent.status.v1",
        Some(&agent.instance_id.to_string()),
        60,
        json!({
            "schemaVersion": "1.0",
            "eventType": "io.github.rainyflash.agentroom.agent.status.v1",
            "id": Uuid::now_v7(),
            "createdAt": "2026-10-20T07:59:00.000Z",
            "actor": agent.actor(),
            "correlationId": Uuid::now_v7(),
            "status": work,
            "visibility": "coarse",
            "leaseExpiresAt": "2026-10-20T08:04:00.000Z",
            "signature": signature([1; 64]),
        }),
    )
}

fn member(user: &str, membership: &str) -> MatrixTimelineEvent {
    timeline_event(
        &format!("$member-{}:matrix.test", Uuid::now_v7().simple()),
        user,
        "m.room.member",
        Some(user),
        3_600,
        json!({"membership": membership}),
    )
}

/// 建大厅的应用服务账号，加上在大厅里的人。
fn room_state(joined: &[String]) -> Vec<MatrixTimelineEvent> {
    let mut state = vec![
        timeline_event(
            "$create:matrix.test",
            SERVICE_USER,
            "m.room.create",
            Some(""),
            86_400,
            json!({"room_version": "11"}),
        ),
        member(SERVICE_USER, "join"),
    ];
    state.extend(joined.iter().map(|user| member(user, "join")));
    state
}

fn moderation_notice(sender: &str, target_event_id: &str) -> MatrixTimelineEvent {
    timeline_event(
        &format!("$notice-{}:matrix.test", Uuid::now_v7().simple()),
        sender,
        "io.github.rainyflash.agentroom.moderation.notice.v1",
        Some(target_event_id),
        10,
        json!({
            "schemaVersion": "1.0",
            "eventType": "io.github.rainyflash.agentroom.moderation.notice.v1",
            "actionId": Uuid::now_v7(),
            "targetEventId": target_event_id,
            "hidden": true,
            "reasonCode": "malicious_content",
        }),
    )
}

fn texts(view: &Value) -> Vec<&str> {
    view["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["text"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn 消息_在线的_agent_和说过话的人拼成快照_不带_matrix_id() {
    let scout = Agent::new("Scout");
    let ranger = Agent::new("Ranger");
    let mut setup = Setup::new(&[&scout, &ranger]);
    setup.network = vec![scout.id];
    let harness = setup.build();
    let hello = Uuid::now_v7();
    let mut state = room_state(&[scout.matrix_user(), ranger.matrix_user(), HUMAN.to_owned()]);
    state.push(status_as(&scout, "working"));
    harness.reader.set(Lobby {
        events: vec![
            agent_chat("$ranger:matrix.test", &ranger, Uuid::now_v7(), "早上好", 30),
            agent_chat(
                "$hello:matrix.test",
                &scout,
                hello,
                "大家好，我是 Scout。",
                20,
            ),
            human_chat(
                "$reply:matrix.test",
                Uuid::now_v7(),
                "你好呀",
                Some(hello),
                10,
            ),
        ],
        state,
    });

    let view = harness.view("default").await;

    assert_eq!(view["schemaVersion"], 1);
    assert_eq!(
        view["lobby"],
        json!({
            "catalogId": harness.directory.lobbies[0].catalog.id().to_string(),
            "name": "Agent Room Global",
            "slug": "agent-room-global",
        })
    );
    assert_eq!(view["updatedAtUnixMs"], NOW);
    let participants = view["participants"].as_array().unwrap();
    let summary: Vec<(&str, &str, bool, Option<&str>)> = participants
        .iter()
        .map(|person| {
            (
                person["name"].as_str().unwrap(),
                person["kind"].as_str().unwrap(),
                person["online"].as_bool().unwrap(),
                person["status"].as_str(),
            )
        })
        .collect();
    // 先列在线的 Agent（带它报的状态），再按最近说话的先后列别人。
    assert_eq!(
        summary,
        [
            ("Scout", "networkAgent", true, Some("working")),
            ("小雨", "person", false, None),
            ("Ranger", "agent", false, None),
        ]
    );
    assert!(participants[1]["status"].is_null());
    assert_eq!(texts(&view), ["早上好", "大家好，我是 Scout。", "你好呀"]);
    let messages = view["messages"].as_array().unwrap();
    assert_eq!(messages[1]["author"], participants[0]["key"]);
    assert_eq!(messages[2]["author"], participants[1]["key"]);
    assert_eq!(messages[2]["replyTo"], messages[1]["key"]);
    assert_eq!(messages[0]["replyTo"], Value::Null);
    assert_eq!(messages[2]["sentAtUnixMs"], NOW - 10_000);
    assert_eq!(messages[0]["truncated"], false);
    assert_eq!(messages[0]["withheld"], false);
    assert_eq!(messages[0]["attachment"], false);
    assert_eq!(messages[0]["edited"], false);
    // 不出现 Matrix 的房间、用户、事件 ID，也不出现 Agent 的编号。
    let body = view.to_string();
    for private in [
        "matrix.test",
        "$hello",
        &scout.id.to_string(),
        &hello.to_string(),
    ] {
        assert!(!body.contains(private), "快照里不该有 {private}");
    }
}

#[tokio::test]
async fn 被隐藏和撤回的不给_作者改过的给新内容_别人改不了() {
    let scout = Agent::new("Scout");
    let ranger = Agent::new("Ranger");
    let harness = Setup::new(&[&scout, &ranger]).build();
    let (retracted, edited) = (Uuid::now_v7(), Uuid::now_v7());
    let mut state = room_state(&[scout.matrix_user(), ranger.matrix_user()]);
    state.push(moderation_notice(SERVICE_USER, "$hidden:matrix.test"));
    // 不是建大厅的账号写的隐藏不算。
    state.push(moderation_notice(
        &ranger.matrix_user(),
        "$kept:matrix.test",
    ));
    harness.reader.set(Lobby {
        events: vec![
            agent_chat("$retracted:matrix.test", &scout, retracted, "说错了", 50),
            agent_chat("$hidden:matrix.test", &scout, Uuid::now_v7(), "广告", 40),
            agent_chat("$kept:matrix.test", &scout, Uuid::now_v7(), "留着", 35),
            agent_chat("$edited:matrix.test", &ranger, edited, "原来的话", 30),
            revision("$redact:matrix.test", &scout, retracted, "redact", 20),
            revision("$replace:matrix.test", &ranger, edited, "replace", 15),
            // Scout 撤不了 Ranger 的话。
            revision("$foreign:matrix.test", &scout, edited, "redact", 10),
        ],
        state,
    });

    let view = harness.view("agent-room-global").await;

    assert_eq!(texts(&view), ["留着", "改过的话"]);
    assert_eq!(view["messages"][1]["edited"], true);
    assert_eq!(view["messages"][0]["edited"], false);
}

#[tokio::test]
async fn 敏感的只说有一条_长的截断_带文件的只说带了() {
    let scout = Agent::new("Scout");
    let harness = Setup::new(&[&scout]).build();
    let mut sensitive = preview("我的地址是……");
    sensitive["sensitivity"] = json!("sensitive");
    let mut picture = preview("看这张图");
    picture["contentType"] = json!("image/png");
    picture["conversation"]["attachmentName"] = json!("家里的照片.png");
    let long = "长".repeat(1_500);
    harness.reader.set(Lobby {
        events: vec![
            agent_message(
                "$sensitive:matrix.test",
                &scout,
                Uuid::now_v7(),
                &sensitive,
                30,
                [1; 64],
            ),
            agent_message(
                "$picture:matrix.test",
                &scout,
                Uuid::now_v7(),
                &picture,
                20,
                [1; 64],
            ),
            agent_chat("$long:matrix.test", &scout, Uuid::now_v7(), &long, 10),
        ],
        state: room_state(&[scout.matrix_user()]),
    });

    let view = harness.view("default").await;
    let messages = view["messages"].as_array().unwrap();

    assert_eq!(messages[0]["withheld"], true);
    assert_eq!(messages[0]["text"], "");
    assert_eq!(messages[1]["attachment"], true);
    assert_eq!(messages[1]["text"], "看这张图");
    assert_eq!(messages[2]["truncated"], true);
    assert_eq!(messages[2]["text"].as_str().unwrap().chars().count(), 1_000);
    let body = view.to_string();
    assert!(!body.contains("家里的照片"), "不给文件名");
    assert!(!body.contains("我的地址"), "不给敏感的正文");
}

#[tokio::test]
async fn 验不过签和没登记的实例发的不显示() {
    let scout = Agent::new("Scout");
    let stranger = Agent::new("Stranger");
    let harness = Setup::new(&[&scout]).build();
    harness.reader.set(Lobby {
        events: vec![
            agent_message(
                "$forged:matrix.test",
                &scout,
                Uuid::now_v7(),
                &preview("假的"),
                30,
                FORGED_SIGNATURE,
            ),
            agent_chat(
                "$stranger:matrix.test",
                &stranger,
                Uuid::now_v7(),
                "没登记",
                20,
            ),
            agent_chat("$real:matrix.test", &scout, Uuid::now_v7(), "真的", 10),
        ],
        state: room_state(&[scout.matrix_user(), stranger.matrix_user()]),
    });

    let view = harness.view("default").await;

    assert_eq!(texts(&view), ["真的"]);
    assert_eq!(view["participants"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn 只留最近_50_条() {
    let scout = Agent::new("Scout");
    let harness = Setup::new(&[&scout]).build();
    let events = (0..60)
        .map(|index| {
            agent_chat(
                &format!("$m{index}:matrix.test"),
                &scout,
                Uuid::now_v7(),
                &format!("第 {index} 条"),
                100 - index,
            )
        })
        .collect();
    harness.reader.set(Lobby {
        events,
        state: room_state(&[scout.matrix_user()]),
    });

    let view = harness.view("default").await;
    let texts = texts(&view);

    assert_eq!(texts.len(), 50);
    assert_eq!(texts[0], "第 10 条");
    assert_eq!(texts[49], "第 59 条");
}

/// 换掉一条事件的服务器时间或内容，别的不变。
fn rewritten(event: &MatrixTimelineEvent, seconds_ago: i64, content: Value) -> MatrixTimelineEvent {
    timeline_event(
        event.event_id().unwrap().as_str(),
        event.sender().unwrap().as_str(),
        event.event_type().as_str(),
        event.state_key(),
        seconds_ago,
        content,
    )
}

#[tokio::test]
async fn 不在大厅里的_过期的_没签对的在线状态都不算在线() {
    let present = Agent::new("Present");
    let departed = Agent::new("Departed");
    let expired = Agent::new("Expired");
    let impostor = Agent::new("Impostor");
    let harness = Setup::new(&[&present, &departed, &expired, &impostor]).build();
    let mut state = room_state(&[
        present.matrix_user(),
        expired.matrix_user(),
        impostor.matrix_user(),
    ]);
    state.push(member(&departed.matrix_user(), "leave"));
    state.push(status(&present));
    state.push(status(&departed));
    // 十分钟前写的状态（内容里的租约照抄），早该过期了。
    let old = status(&expired);
    state.push(rewritten(&old, 600, old.content().clone()));
    let forged = status(&impostor);
    let mut content = forged.content().clone();
    content["signature"] = json!(signature(FORGED_SIGNATURE));
    state.push(rewritten(&forged, 60, content));
    harness.reader.set(Lobby {
        events: Vec::new(),
        state,
    });

    let view = harness.view("default").await;
    let names: Vec<&str> = view["participants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["name"].as_str().unwrap())
        .collect();

    assert_eq!(names, ["Present"]);
}

#[tokio::test]
async fn 编号在两次快照之间不变_新消息接在后面() {
    let scout = Agent::new("Scout");
    let harness = Setup::new(&[&scout]).build();
    let first = agent_chat("$first:matrix.test", &scout, Uuid::now_v7(), "第一句", 20);
    harness.reader.set(Lobby {
        events: vec![first.clone()],
        state: room_state(&[scout.matrix_user()]),
    });
    let before = harness.view("default").await;

    harness.clock.advance(3_000);
    harness.reader.set(Lobby {
        events: vec![
            first,
            agent_chat("$second:matrix.test", &scout, Uuid::now_v7(), "第二句", 1),
        ],
        state: room_state(&[scout.matrix_user()]),
    });
    let after = harness.view("default").await;

    assert_eq!(texts(&after), ["第一句", "第二句"]);
    assert_eq!(after["messages"][0]["key"], before["messages"][0]["key"]);
    assert_eq!(
        after["participants"][0]["key"],
        before["participants"][0]["key"]
    );
    assert_ne!(after["messages"][1]["key"], after["messages"][0]["key"]);
}

#[tokio::test]
async fn 三秒内再来直接给快照_过了三秒才重读() {
    let harness = Setup::new(&[]).build();
    harness.reader.set(Lobby {
        events: Vec::new(),
        state: room_state(&[]),
    });

    harness.view("default").await;
    harness.clock.advance(2_999);
    harness.view("default").await;
    assert_eq!(harness.reader.reads.load(Ordering::SeqCst), 1);

    harness.clock.advance(1);
    harness.view("default").await;
    assert_eq!(harness.reader.reads.load(Ordering::SeqCst), 2);
    // 目录 30 秒内只查一次。
    assert_eq!(harness.directory.listings.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 读不到时先给不太旧的快照_太旧了照实说看不了() {
    let scout = Agent::new("Scout");
    let harness = Setup::new(&[&scout]).build();
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$hello:matrix.test",
            &scout,
            Uuid::now_v7(),
            "在吗",
            5,
        )],
        state: room_state(&[scout.matrix_user()]),
    });
    let fresh = harness.watch.watch("default").await.unwrap();

    harness.reader.fail(true);
    harness.clock.advance(3_000);
    assert_eq!(harness.watch.watch("default").await.unwrap(), fresh);

    harness.clock.advance(60_000);
    assert_eq!(
        harness.watch.watch("default").await,
        Err(PublicWatchFailure::Unavailable)
    );

    // 失败以后同样隔 3 秒再读，不追着 Synapse 问。
    let reads = harness.reader.reads.load(Ordering::SeqCst);
    harness.reader.fail(false);
    assert_eq!(
        harness.watch.watch("default").await,
        Err(PublicWatchFailure::Unavailable)
    );
    assert_eq!(harness.reader.reads.load(Ordering::SeqCst), reads);
    harness.clock.advance(3_000);
    assert!(harness.watch.watch("default").await.is_ok());
}

#[tokio::test]
async fn 还没人进过的大厅人和消息都是空的() {
    let mut setup = Setup::new(&[]);
    setup.directory.instance = false;
    let harness = setup.build();

    let view = harness.view("default").await;

    assert_eq!(view["participants"], json!([]));
    assert_eq!(view["messages"], json!([]));
    assert_eq!(harness.reader.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 按_slug_找大厅_default_是默认大厅() {
    let mut setup = Setup::new(&[]);
    setup.directory = FakeDirectory::new(&["night-owls", "agent-room-global"]);
    let harness = setup.build();
    harness.reader.set(Lobby {
        events: Vec::new(),
        state: room_state(&[]),
    });

    assert_eq!(
        harness.view("default").await["lobby"]["slug"],
        "agent-room-global"
    );
    assert_eq!(
        harness.view("night-owls").await["lobby"]["slug"],
        "night-owls"
    );
    assert_eq!(
        harness.watch.watch("nobody-here").await,
        Err(PublicWatchFailure::LobbyNotFound)
    );
    // 不像 slug 的不去碰目录。
    let listings = harness.directory.listings.load(Ordering::SeqCst);
    assert_eq!(
        harness.watch.watch("Not A Slug").await,
        Err(PublicWatchFailure::LobbyNotFound)
    );
    assert_eq!(harness.directory.listings.load(Ordering::SeqCst), listings);

    // 没有叫 agent-room-global 的大厅时，默认是目录里的第一间。
    let mut setup = Setup::new(&[]);
    setup.directory = FakeDirectory::new(&["night-owls", "early-birds"]);
    let harness = setup.build();
    harness.reader.set(Lobby::default());
    assert_eq!(harness.view("default").await["lobby"]["slug"], "night-owls");
}

#[tokio::test]
async fn 查不了是不是网络_agent_时先当普通_agent() {
    let scout = Agent::new("Scout");
    let mut setup = Setup::new(&[&scout]);
    setup.network = vec![scout.id];
    setup.lookup_failing = true;
    let harness = setup.build();
    harness.reader.set(Lobby {
        events: vec![agent_chat(
            "$hello:matrix.test",
            &scout,
            Uuid::now_v7(),
            "在吗",
            5,
        )],
        state: room_state(&[scout.matrix_user()]),
    });

    let view = harness.view("default").await;

    assert_eq!(view["participants"][0]["kind"], "agent");
}

#[tokio::test]
async fn 总开关关着时说没开放_不碰目录() {
    let mut setup = Setup::new(&[]);
    setup.enabled = false;
    let harness = setup.build();

    assert_eq!(
        harness.watch.watch("default").await,
        Err(PublicWatchFailure::Disabled)
    );
    assert_eq!(harness.directory.listings.load(Ordering::SeqCst), 0);
}

async fn call(harness: &Harness, path: &str) -> axum::response::Response {
    router(PublicWatchHttpState {
        watch: harness.watch.clone(),
    })
    .layer(middleware::from_fn(crate::correlation::attach))
    .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
    .await
    .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[tokio::test]
async fn 接口不要登录_快照可以缓存三秒() {
    let harness = Setup::new(&[]).build();
    harness.reader.set(Lobby {
        events: Vec::new(),
        state: room_state(&[]),
    });

    let response = call(&harness, "/public-lobbies/default/watch").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=3"
    );
    assert_eq!(
        json_body(response).await["lobby"]["slug"],
        "agent-room-global"
    );
}

#[tokio::test]
async fn 接口照实说没开放_没有这个大厅_暂时看不了() {
    let mut setup = Setup::new(&[]);
    setup.enabled = false;
    let disabled = call(&setup.build(), "/public-lobbies/default/watch").await;
    assert_eq!(disabled.status(), StatusCode::NOT_FOUND);
    assert_eq!(disabled.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(json_body(disabled).await["code"], "public_watch.disabled");

    let harness = Setup::new(&[]).build();
    let missing = call(&harness, "/public-lobbies/nobody-here/watch").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(missing).await["code"],
        "public_watch.lobby_not_found"
    );

    harness.reader.fail(true);
    let unavailable = call(&harness, "/public-lobbies/default/watch").await;
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unavailable.headers()[header::RETRY_AFTER], "5");
    assert_eq!(
        json_body(unavailable).await["code"],
        "public_watch.unavailable"
    );
}
