use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentFailure, NetworkAgentFailureKind,
        NetworkAgentResult, NetworkAgentSession, NetworkAgentUseCases, NetworkAgentView,
    },
    persistence::RepositoryResult,
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRecord,
        AgentInstanceVerificationRepository, Clock, DeviceSignature, MatrixEventId,
        MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
        MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
        MatrixTimelineEvent, MatrixUserId, NetworkAgentAckOutcome, NetworkAgentInboxAppend,
        NetworkAgentInboxAppendOutcome, NetworkAgentInboxChange, NetworkAgentInboxEntry,
        NetworkAgentInboxPage, NetworkAgentInboxStore, NetworkAgentMatrixGateway,
        NetworkAgentSyncRequest, PortFuture, SecretValue,
    },
};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    ids::{AgentId, AgentInstanceId, MessageId, NetworkAgentId},
    time::UtcMillis,
};
use serde_json::{Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{
    NetworkAgentMessaging, NetworkGateway, NetworkGatewayDependencies, NetworkGatewayFailure,
};

const TOKEN: &str = "network-agent-token";
const ROOM: &str = "!lobby:matrix.test";
const NETWORK_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const OWN_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const OWN_INSTANCE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";
const OTHER_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e61";
const OTHER_INSTANCE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e62";
/// 验签替身把全是 0xFF 的签名当成伪造的。
const FORGED_SIGNATURE: [u8; 64] = [0xFF; 64];

// ---------- 假实现 ----------

struct FakeAgents;

impl NetworkAgentUseCases for FakeAgents {
    fn create(
        &self,
        _request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        unreachable!("网关不创建网络 Agent")
    }

    fn me<'a>(&'a self, _token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
        unreachable!("网关不查看自己")
    }

    fn disable<'a>(&'a self, _token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
        unreachable!("网关不停用")
    }

    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        let result = if token == TOKEN {
            Ok(NetworkAgentSession {
                network_agent_id: network_agent_id(),
                agent_id: agent(OWN_AGENT),
                agent_instance_id: instance(OWN_INSTANCE),
                display_name: "Scout".to_owned(),
                matrix_access_token: SecretValue::new("syt_scout").unwrap(),
                rooms: Vec::new(),
            })
        } else {
            Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ))
        };
        Box::pin(async move { result })
    }
}

/// 与 Postgres 实现同样语义的收件箱：按到达编号，确认到哪条就删到哪条。
#[derive(Default)]
struct MemoryInbox {
    state: Mutex<InboxState>,
}

#[derive(Default)]
struct InboxState {
    sync_token: Option<MatrixSyncToken>,
    sequence: u64,
    dropped: u64,
    entries: Vec<(u64, MatrixEventId, MessageId, String, Value)>,
}

impl MemoryInbox {
    fn sync_token(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .sync_token
            .as_ref()
            .map(|token| token.as_str().to_owned())
    }
}

impl NetworkAgentInboxStore for MemoryInbox {
    fn pending(
        &self,
        _id: NetworkAgentId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentInboxPage>> {
        let state = self.state.lock().unwrap();
        let page = NetworkAgentInboxPage {
            sync_token: state.sync_token.clone(),
            entries: state
                .entries
                .iter()
                .take(usize::from(limit))
                .map(
                    |(sequence, event_id, _, _, preview)| NetworkAgentInboxEntry {
                        sequence: *sequence,
                        event_id: event_id.clone(),
                        preview: preview.clone(),
                    },
                )
                .collect(),
            pending: u64::try_from(state.entries.len()).unwrap(),
            dropped: state.dropped,
        };
        Box::pin(async move { Ok(page) })
    }

    fn append<'a>(
        &'a self,
        append: &'a NetworkAgentInboxAppend,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxAppendOutcome>> {
        let mut state = self.state.lock().unwrap();
        let outcome = if state.sync_token == append.expected_sync_token {
            let mut appended = 0;
            for change in &append.changes {
                match change {
                    NetworkAgentInboxChange::Message(message) => {
                        if state
                            .entries
                            .iter()
                            .any(|(_, event_id, ..)| *event_id == message.event_id)
                        {
                            continue;
                        }
                        state.sequence += 1;
                        let sequence = state.sequence;
                        state.entries.push((
                            sequence,
                            message.event_id.clone(),
                            message.message_id,
                            message.actor_key.clone(),
                            message.preview.clone(),
                        ));
                        appended += 1;
                    }
                    NetworkAgentInboxChange::Replace {
                        message_id,
                        actor_key,
                        patch,
                        ..
                    } => {
                        for (_, _, id, actor, preview) in &mut state.entries {
                            if id == message_id && actor == actor_key {
                                for (key, value) in patch.as_object().unwrap() {
                                    preview[key] = value.clone();
                                }
                            }
                        }
                    }
                    NetworkAgentInboxChange::Redact {
                        message_id,
                        actor_key,
                        ..
                    } => state
                        .entries
                        .retain(|(_, _, id, actor, _)| !(id == message_id && actor == actor_key)),
                }
            }
            let capacity = usize::try_from(append.capacity).unwrap();
            if state.entries.len() > capacity {
                let excess = state.entries.len() - capacity;
                state.entries.drain(..excess);
                state.dropped += u64::try_from(excess).unwrap();
            }
            state.sync_token = Some(append.next_sync_token.clone());
            NetworkAgentInboxAppendOutcome::Applied { appended }
        } else {
            NetworkAgentInboxAppendOutcome::Stale
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn acknowledge<'a>(
        &'a self,
        _id: NetworkAgentId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>> {
        let mut state = self.state.lock().unwrap();
        let sequence = state
            .entries
            .iter()
            .find(|(_, id, ..)| id == event_id)
            .map(|(sequence, ..)| *sequence);
        let outcome = if let Some(sequence) = sequence {
            state.entries.retain(|(entry, ..)| *entry > sequence);
            state.dropped = 0;
            NetworkAgentAckOutcome::Acknowledged {
                pending: u64::try_from(state.entries.len()).unwrap(),
            }
        } else {
            NetworkAgentAckOutcome::NotPending {
                pending: u64::try_from(state.entries.len()).unwrap(),
            }
        };
        Box::pin(async move { Ok(outcome) })
    }
}

enum Step {
    Batch(MatrixResult<MatrixSyncBatch>),
    /// 一直等到被通知才返回空批次，模拟 Matrix 长轮询。
    Block(Arc<Notify>),
}

/// 按顺序给出同步结果；用完之后按请求的超时等一会儿再返回空批次。
#[derive(Default)]
struct ScriptedMatrix {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<NetworkAgentSyncRequest>>,
    tokens: Mutex<Vec<String>>,
}

impl ScriptedMatrix {
    fn push(&self, step: Step) {
        self.steps.lock().unwrap().push_back(step);
    }

    fn requests(&self) -> Vec<NetworkAgentSyncRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl NetworkAgentMatrixGateway for ScriptedMatrix {
    fn sync<'a>(
        &'a self,
        access_token: &'a SecretValue,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        self.requests.lock().unwrap().push(request.clone());
        self.tokens
            .lock()
            .unwrap()
            .push(access_token.expose().to_owned());
        let step = self.steps.lock().unwrap().pop_front();
        let since = request
            .since
            .as_ref()
            .map_or_else(|| "s0".to_owned(), |token| token.as_str().to_owned());
        let timeout = request.timeout_millis;
        Box::pin(async move {
            match step {
                Some(Step::Batch(result)) => result,
                Some(Step::Block(notify)) => {
                    notify.notified().await;
                    Ok(batch(&format!("{since}+"), Vec::new()))
                }
                None => {
                    tokio::time::sleep(Duration::from_millis(timeout)).await;
                    Ok(batch(&since, Vec::new()))
                }
            }
        })
    }
}

/// 验签查到的实例：另一个 Agent 与自己的实例都登记过。
struct KnownInstances;

impl AgentInstanceVerificationRepository for KnownInstances {
    fn find_verification_record(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>> {
        let agent_id = if instance_id == instance(OTHER_INSTANCE) {
            Some(agent(OTHER_AGENT))
        } else if instance_id == instance(OWN_INSTANCE) {
            Some(agent(OWN_AGENT))
        } else {
            None
        };
        let record = agent_id.map(|agent_id| AgentInstanceVerificationRecord {
            instance_id,
            agent_id,
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

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_758_600_000_000).unwrap()
    }
}

// ---------- 组装与构造 ----------

struct Harness {
    gateway: NetworkGateway,
    inbox: Arc<MemoryInbox>,
    matrix: Arc<ScriptedMatrix>,
}

fn harness() -> Harness {
    let inbox = Arc::new(MemoryInbox::default());
    let matrix = Arc::new(ScriptedMatrix::default());
    let gateway = NetworkGateway::new(NetworkGatewayDependencies {
        agents: Arc::new(FakeAgents),
        inbox: inbox.clone(),
        matrix: matrix.clone(),
        verification: Arc::new(KnownInstances),
        signatures: Arc::new(FakeSignatures),
        clock: Arc::new(FixedClock),
    });
    Harness {
        gateway,
        inbox,
        matrix,
    }
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

fn network_agent_id() -> NetworkAgentId {
    NetworkAgentId::from_uuid(uuid(NETWORK_AGENT))
}

fn agent(value: &str) -> AgentId {
    AgentId::from_uuid(uuid(value))
}

fn instance(value: &str) -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid(value))
}

fn matrix_user(agent_id: &str) -> String {
    format!("@_agent_{}:matrix.test", uuid(agent_id).simple())
}

fn batch(next: &str, events: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
    let rooms = if events.is_empty() {
        Vec::new()
    } else {
        vec![MatrixRoomSync::new(
            MatrixRoomId::new(ROOM).unwrap(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            events,
            Vec::new(),
        )]
    };
    MatrixSyncBatch::new(MatrixSyncToken::new(next).unwrap(), rooms)
}

fn signature(bytes: [u8; 64]) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn actor(agent_id: &str, instance_id: &str) -> Value {
    json!({
        "agent": {
            "agentId": agent_id,
            "displayName": if agent_id == OWN_AGENT { "Scout" } else { "Ranger" },
            "matrixUserId": matrix_user(agent_id),
        },
        "instanceId": instance_id,
        "provenance": "autonomous_agent",
    })
}

/// 一条聊天消息的预览事件。
fn chat(
    event_id: &str,
    from: (&str, &str),
    message_id: Uuid,
    text: &str,
    signature_bytes: [u8; 64],
) -> MatrixTimelineEvent {
    let content = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.preview.v1",
        "id": message_id,
        "createdAt": "2026-09-23T12:00:00.000Z",
        "actor": actor(from.0, from.1),
        "correlationId": message_id,
        "roomId": ROOM,
        "preview": {
            "conversation": {"text": text, "mentions": []},
            "title": text,
            "summary": text,
            "contentType": "text/plain",
            "sensitivity": "normal",
            "riskFlags": [],
        },
        "content": {
            "contentId": Uuid::now_v7(),
            "digestSha256": "11".repeat(32),
            "sizeBytes": 16,
            "mediaType": "text/plain",
            "fetchMode": "on_demand",
        },
        "signature": signature(signature_bytes),
    });
    event(
        event_id,
        from.0,
        "io.github.rainyflash.agentroom.message.preview.v1",
        content,
    )
}

fn revision(event_id: &str, from: (&str, &str), target: Uuid, kind: &str) -> MatrixTimelineEvent {
    let id = Uuid::now_v7();
    let mut content = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.revision.v1",
        "id": id,
        "createdAt": "2026-09-23T12:01:00.000Z",
        "actor": actor(from.0, from.1),
        "correlationId": id,
        "roomId": ROOM,
        "targetMessageId": target,
        "kind": kind,
        "signature": signature([1; 64]),
    });
    if kind == "replace" {
        content["preview"] = json!({
            "conversation": {"text": "改过的话", "mentions": []},
            "title": "改过的话",
            "summary": "改过的话",
            "contentType": "text/plain",
            "sensitivity": "normal",
            "riskFlags": [],
        });
        content["content"] = json!({
            "contentId": Uuid::now_v7(),
            "digestSha256": "22".repeat(32),
            "sizeBytes": 12,
            "mediaType": "text/plain",
            "fetchMode": "on_demand",
        });
    }
    event(
        event_id,
        from.0,
        "io.github.rainyflash.agentroom.message.revision.v1",
        content,
    )
}

fn event(
    event_id: &str,
    sender_agent: &str,
    event_type: &str,
    content: Value,
) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new(matrix_user(sender_agent)).unwrap()),
        MatrixEventType::new(event_type).unwrap(),
        None,
        None,
        Some(1_758_600_000_000),
        content,
    )
    .unwrap()
}

fn other() -> (&'static str, &'static str) {
    (OTHER_AGENT, OTHER_INSTANCE)
}

fn own() -> (&'static str, &'static str) {
    (OWN_AGENT, OWN_INSTANCE)
}

fn texts(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["conversation"]["text"].as_str().unwrap())
        .collect()
}

// ---------- 用例 ----------

#[tokio::test(start_paused = true)]
async fn 第一次不等_带回最近的几条_自己发的不进收件箱_没确认前再取还是这些() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat(
                "$hello:matrix.test",
                other(),
                Uuid::now_v7(),
                "你好",
                [1; 64],
            ),
            chat(
                "$mine:matrix.test",
                own(),
                Uuid::now_v7(),
                "我自己说的",
                [1; 64],
            ),
        ],
    ))));

    let first = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .expect("取到消息");

    assert_eq!(texts(&first.messages), ["你好"]);
    assert_eq!(first.pending, 1);
    let message = &first.messages[0];
    assert_eq!(message["eventId"], "$hello:matrix.test");
    assert_eq!(message["roomId"], ROOM);
    assert_eq!(message["actor"]["kind"], "agent");
    assert_eq!(message["actor"]["agent"]["displayName"], "Ranger");
    assert_eq!(message["actor"]["provenance"], "autonomous_agent");

    let requests = harness.matrix.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].since, None);
    assert_eq!(requests[0].timeout_millis, 0);
    assert_eq!(requests[0].timeline_limit, 20);
    assert_eq!(*harness.matrix.tokens.lock().unwrap(), ["syt_scout"]);
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("s1"));

    // 没确认，再取还是这一条，不必再问 Matrix。
    let again = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .expect("再取");
    assert_eq!(texts(&again.messages), ["你好"]);
    assert_eq!(harness.matrix.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn 确认之后不再收到_没有新消息时等满给的时间再空手返回() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat("$one:matrix.test", other(), Uuid::now_v7(), "一", [1; 64]),
            chat("$two:matrix.test", other(), Uuid::now_v7(), "二", [1; 64]),
        ],
    ))));
    let first = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();
    assert_eq!(texts(&first.messages), ["一", "二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$one:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    let rest = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();
    assert_eq!(texts(&rest.messages), ["二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 0 }
    );
    // 确认过的再确认一次不报错，只说明它不在收件箱里。
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::NotPending { pending: 0 }
    );

    let started = tokio::time::Instant::now();
    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(5), 20)
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    let requests = harness.matrix.requests();
    assert_eq!(requests[1].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[1].timeout_millis, 5_000);
    assert_eq!(requests[1].timeline_limit, 50);
}

#[tokio::test(start_paused = true)]
async fn 只看一眼时有位置就不问_matrix() {
    let harness = harness();
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    assert_eq!(harness.matrix.requests().len(), 1, "第一次总要同步一次");

    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(harness.matrix.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn 伪造签名与冒名的事件被隔离_不进收件箱() {
    let harness = harness();
    let mut impostor = chat(
        "$impostor:matrix.test",
        other(),
        Uuid::now_v7(),
        "冒名",
        [1; 64],
    );
    // 自称另一个 Agent，但 Matrix 发送者是自己。
    impostor = MatrixTimelineEvent::new(
        impostor.event_id().cloned(),
        Some(MatrixUserId::new(matrix_user(OWN_AGENT)).unwrap()),
        impostor.event_type().clone(),
        None,
        None,
        impostor.origin_server_timestamp(),
        impostor.content().clone(),
    )
    .unwrap();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat(
                "$forged:matrix.test",
                other(),
                Uuid::now_v7(),
                "伪造",
                FORGED_SIGNATURE,
            ),
            impostor,
            chat(
                "$real:matrix.test",
                other(),
                Uuid::now_v7(),
                "真的",
                [1; 64],
            ),
        ],
    ))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();

    assert_eq!(texts(&received.messages), ["真的"]);
}

#[tokio::test(start_paused = true)]
async fn 作者修改或撤回还没确认的消息_收件箱跟着变_别人不能改() {
    let harness = harness();
    let edited = Uuid::now_v7();
    let withdrawn = Uuid::now_v7();
    let untouched = Uuid::now_v7();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat("$edited:matrix.test", other(), edited, "原话", [1; 64]),
            chat(
                "$withdrawn:matrix.test",
                other(),
                withdrawn,
                "撤回的话",
                [1; 64],
            ),
            chat(
                "$untouched:matrix.test",
                other(),
                untouched,
                "别人改不了",
                [1; 64],
            ),
            revision("$replace:matrix.test", other(), edited, "replace"),
            revision("$redact:matrix.test", other(), withdrawn, "redact"),
            // 自己发的修订对不上作者，什么也不改。
            revision("$hijack:matrix.test", own(), untouched, "redact"),
        ],
    ))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();

    assert_eq!(texts(&received.messages), ["改过的话", "别人改不了"]);
    assert_eq!(received.messages[0]["eventId"], "$edited:matrix.test");
    assert_eq!(received.messages[0]["messageId"], edited.to_string());
    assert_eq!(received.messages[0]["title"], "改过的话");
}

#[tokio::test(start_paused = true)]
async fn 新的长轮询让旧的立刻空手返回() {
    let harness = Arc::new(harness());
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    let blocked = Arc::new(Notify::new());
    harness.matrix.push(Step::Block(blocked.clone()));

    let old = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .gateway
                .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
                .await
        })
    };
    tokio::task::yield_now().await;
    while harness.matrix.requests().len() < 2 {
        tokio::task::yield_now().await;
    }
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$new:matrix.test",
            other(),
            Uuid::now_v7(),
            "新消息",
            [1; 64],
        )],
    ))));
    let new = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();

    let old = old.await.unwrap().unwrap();
    assert!(old.messages.is_empty(), "旧的空手返回");
    assert_eq!(texts(&new.messages), ["新消息"]);
    blocked.notify_waiters();
}

#[tokio::test(start_paused = true)]
async fn 令牌不对按网络_agent_的错误回答_matrix_失败算依赖不可用() {
    let harness = harness();
    assert_eq!(
        harness
            .gateway
            .wait_for_messages("wrong", Duration::from_secs(1), 20)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::Unauthorized
        ))
    );
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "not an event id")
            .await
            .unwrap_err(),
        NetworkGatewayFailure::InvalidEvent
    );

    harness.matrix.push(Step::Batch(Err(MatrixFailure::new(
        MatrixOperation::Sync,
        MatrixFailureKind::DependencyUnavailable,
    ))));
    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, Duration::from_secs(1), 20)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert_eq!(harness.inbox.sync_token(), None, "失败的同步不推进位置");
}

#[tokio::test(start_paused = true)]
async fn 收件箱满了丢掉最早的并告诉_agent_丢了几条() {
    let harness = harness();
    let events = (0..205)
        .map(|index| {
            chat(
                &format!("$m{index}:matrix.test"),
                other(),
                Uuid::now_v7(),
                &format!("第 {index} 条"),
                [1; 64],
            )
        })
        .collect();
    harness.matrix.push(Step::Batch(Ok(batch("s1", events))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 50)
        .await
        .unwrap();

    assert_eq!(received.messages.len(), 50);
    assert_eq!(received.pending, 200);
    assert_eq!(received.dropped, 5);
    assert_eq!(received.messages[0]["conversation"]["text"], "第 5 条");
}
