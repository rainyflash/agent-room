use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use agent_room_application::ports::{
    Clock, DeviceProofVerifier, DeviceSignature, MatrixEventId, MatrixEventType, MatrixRoomId,
    MatrixRoomStatePosition, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
    MatrixTimelineEvent, MatrixUserId, PortFuture,
};
use agent_room_bridge_core::{
    agent_verification::{
        AgentEventAuthenticationDecision, AgentEventAuthenticationFailure,
        AgentEventAuthenticationFailureKind, AgentEventAuthenticator,
    },
    presence::{
        PresenceLeasePolicy, PresenceProjectionBatch, PresenceProjectionFailure,
        PresenceProjectionRepository, PresenceQuery, PresenceRoomProjectionMode,
        PresenceSyncDependencies, PresenceSyncFailureKind, PresenceSyncIssueReason,
        PresenceSyncService,
    },
};
use agent_room_domain::{
    agent_status::AgentWorkStatus,
    devices::DevicePublicSigningKey,
    ids::{AgentId, AgentInstanceId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::{Ed25519DeviceProofVerifier, Ed25519DeviceSigningKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use uuid::Uuid;

const ACTOR_AGENT_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3";
const ACTOR_INSTANCE_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4";
const ACTOR_MATRIX_ID: &str = "@agent:matrix.test";
const NOW_UNIX_MS: i64 = 1_777_032_000_000;
const NOW_RFC3339: &str = "2026-04-24T12:00:00.000Z";
const EXPIRY_RFC3339: &str = "2026-04-24T12:05:00.000Z";

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(NOW_UNIX_MS).expect("测试时间有效")
    }
}

struct 验签认证器 {
    public_key: DevicePublicSigningKey,
    fail: AtomicBool,
    historical_revoked: AtomicBool,
}

impl AgentEventAuthenticator for 验签认证器 {
    fn authenticate<'a>(
        &'a self,
        _agent_id: AgentId,
        _instance_id: AgentInstanceId,
        _origin_server_timestamp: UtcMillis,
        canonical_event: &'a [u8],
        signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure>>
    {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(AgentEventAuthenticationFailure::new(
                    AgentEventAuthenticationFailureKind::Unavailable,
                ))
            });
        }
        let decision =
            if Ed25519DeviceProofVerifier.verify(&self.public_key, canonical_event, signature) {
                if self.historical_revoked.load(Ordering::SeqCst) {
                    AgentEventAuthenticationDecision::TrustedHistoricalRevoked
                } else {
                    AgentEventAuthenticationDecision::Trusted
                }
            } else {
                AgentEventAuthenticationDecision::InvalidSignature
            };
        Box::pin(async move { Ok(decision) })
    }
}

#[derive(Default)]
struct 记录投影仓库 {
    batches: Mutex<Vec<PresenceProjectionBatch>>,
}

impl PresenceProjectionRepository for 记录投影仓库 {
    fn apply<'a>(
        &'a self,
        batch: &'a PresenceProjectionBatch,
    ) -> PortFuture<'a, Result<(), PresenceProjectionFailure>> {
        self.batches
            .lock()
            .expect("Presence 投影记录锁可用")
            .push(batch.clone());
        Box::pin(async { Ok(()) })
    }

    fn list<'a>(
        &'a self,
        _query: &'a PresenceQuery,
    ) -> PortFuture<
        'a,
        Result<
            Vec<agent_room_bridge_core::presence::PresenceObservation>,
            PresenceProjectionFailure,
        >,
    > {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct 测试夹具 {
    signing_key: Ed25519DeviceSigningKey,
    authenticator: Arc<验签认证器>,
    projections: Arc<记录投影仓库>,
}

impl 测试夹具 {
    fn new() -> Self {
        let signing_key = Ed25519DeviceSigningKey::generate().expect("测试私钥可生成");
        let public_key = signing_key.public_key().expect("测试公钥可读取");
        Self {
            signing_key,
            authenticator: Arc::new(验签认证器 {
                public_key,
                fail: AtomicBool::new(false),
                historical_revoked: AtomicBool::new(false),
            }),
            projections: Arc::new(记录投影仓库::default()),
        }
    }

    fn service(&self) -> PresenceSyncService {
        PresenceSyncService::new(
            PresenceSyncDependencies {
                authenticator: self.authenticator.clone(),
                projections: self.projections.clone(),
                clock: Arc::new(固定时钟),
            },
            PresenceLeasePolicy::new(
                DurationMillis::new(300_000).expect("最大租约有效"),
                DurationMillis::new(15_000).expect("时钟偏差有效"),
            )
            .expect("Presence 策略有效"),
        )
    }
}

#[tokio::test]
async fn 只有已入房且签名可信的短租约状态进入投影() {
    let fixture = 测试夹具::new();
    let sync = sync_with_state(vec![
        membership_event("join"),
        signed_status_event(
            &fixture.signing_key,
            "$status:matrix.test",
            "working",
            NOW_RFC3339,
            EXPIRY_RFC3339,
            10,
        ),
    ]);

    let outcome = fixture
        .service()
        .process(&sync, true)
        .await
        .expect("可信状态可进入投影");

    assert_eq!(outcome.accepted_statuses(), 1);
    assert_eq!(outcome.membership_changes(), 1);
    assert!(outcome.issues().is_empty());
    let batches = fixture
        .projections
        .batches
        .lock()
        .expect("Presence 投影记录锁可用");
    let room = &batches[0].rooms()[0];
    assert_eq!(room.mode(), PresenceRoomProjectionMode::Replace);
    assert!(room.memberships()[0].joined());
    assert_eq!(room.presences()[0].status(), AgentWorkStatus::Working);
    assert_eq!(
        room.presences()[0].identity().matrix_user_id().as_str(),
        ACTOR_MATRIX_ID
    );
    assert_eq!(
        room.presences()[0].lease_expires_at().value(),
        NOW_UNIX_MS + 315_000
    );
}

#[tokio::test]
async fn 篡改签名与超长租约被逐条隔离而不污染投影() {
    let fixture = 测试夹具::new();
    let mut tampered_payload = status_payload("working", NOW_RFC3339, EXPIRY_RFC3339);
    sign_payload(&fixture.signing_key, &mut tampered_payload);
    tampered_payload["status"] = json!("blocked");
    let tampered = status_timeline_event("$tampered:matrix.test", tampered_payload, 10);
    let overlong = signed_status_event(
        &fixture.signing_key,
        "$overlong:matrix.test",
        "working",
        NOW_RFC3339,
        "2026-04-24T12:05:00.001Z",
        11,
    );
    let sync = sync_with_state(vec![membership_event("join"), tampered, overlong]);

    let outcome = fixture
        .service()
        .process(&sync, true)
        .await
        .expect("坏事件只应被隔离");

    assert_eq!(outcome.accepted_statuses(), 0);
    assert_eq!(
        outcome
            .issues()
            .iter()
            .map(|issue| issue.reason)
            .collect::<Vec<_>>(),
        vec![
            PresenceSyncIssueReason::InvalidSignature,
            PresenceSyncIssueReason::InvalidLease,
        ]
    );
    let batches = fixture
        .projections
        .batches
        .lock()
        .expect("Presence 投影记录锁可用");
    assert!(batches[0].rooms()[0].presences().is_empty());
}

#[tokio::test]
async fn 状态段位于时间线之后时不会让旧时间线状态覆盖当前状态() {
    let fixture = 测试夹具::new();
    let timeline = vec![signed_status_event(
        &fixture.signing_key,
        "$stale:matrix.test",
        "working",
        NOW_RFC3339,
        EXPIRY_RFC3339,
        9,
    )];
    let state = vec![
        membership_event("join"),
        signed_status_event(
            &fixture.signing_key,
            "$current:matrix.test",
            "idle",
            NOW_RFC3339,
            EXPIRY_RFC3339,
            10,
        ),
    ];
    let room = MatrixRoomSync::new(
        room_id(),
        MatrixRoomSyncKind::Joined,
        false,
        None,
        timeline,
        state,
    )
    .with_state_position(MatrixRoomStatePosition::AfterTimeline);
    let sync = MatrixSyncBatch::new(
        MatrixSyncToken::new("after-timeline-state").expect("同步游标有效"),
        vec![room],
    );

    let outcome = fixture
        .service()
        .process(&sync, false)
        .await
        .expect("当前状态可投影");

    assert_eq!(outcome.accepted_statuses(), 1);
    let batches = fixture
        .projections
        .batches
        .lock()
        .expect("Presence 投影记录锁可用");
    assert_eq!(
        batches[0].rooms()[0].presences()[0].status(),
        AgentWorkStatus::Idle
    );
}

#[tokio::test]
async fn 验签依赖不可用时不提交任何_presence_投影() {
    let fixture = 测试夹具::new();
    fixture.authenticator.fail.store(true, Ordering::SeqCst);
    let sync = sync_with_state(vec![
        membership_event("join"),
        signed_status_event(
            &fixture.signing_key,
            "$status:matrix.test",
            "working",
            NOW_RFC3339,
            EXPIRY_RFC3339,
            10,
        ),
    ]);

    let failure = fixture
        .service()
        .process(&sync, true)
        .await
        .expect_err("验签基础设施故障必须中止批次");

    assert_eq!(failure.kind(), PresenceSyncFailureKind::Authentication);
    assert!(
        fixture
            .projections
            .batches
            .lock()
            .expect("Presence 投影记录锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 撤销前的可信历史状态被强制降级为离线() {
    let fixture = 测试夹具::new();
    fixture
        .authenticator
        .historical_revoked
        .store(true, Ordering::SeqCst);
    let sync = sync_with_state(vec![
        membership_event("join"),
        signed_status_event(
            &fixture.signing_key,
            "$historical:matrix.test",
            "working",
            NOW_RFC3339,
            EXPIRY_RFC3339,
            10,
        ),
    ]);

    fixture
        .service()
        .process(&sync, true)
        .await
        .expect("撤销前签名仍可鉴别");

    let batches = fixture
        .projections
        .batches
        .lock()
        .expect("Presence 投影记录锁可用");
    let presence = &batches[0].rooms()[0].presences()[0];
    assert_eq!(presence.status(), AgentWorkStatus::Offline);
    assert_eq!(presence.lease_expires_at(), presence.observed_at());
}

fn sync_with_state(state: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
    MatrixSyncBatch::new(
        MatrixSyncToken::new("presence-sync").expect("同步游标有效"),
        vec![MatrixRoomSync::new(
            room_id(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            Vec::new(),
            state,
        )],
    )
}

fn membership_event(membership: &str) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new("$member:matrix.test").expect("事件标识有效")),
        Some(MatrixUserId::new(ACTOR_MATRIX_ID).expect("用户标识有效")),
        MatrixEventType::new("m.room.member").expect("事件类型有效"),
        Some(ACTOR_MATRIX_ID.to_owned()),
        None,
        Some(1),
        json!({"membership": membership}),
    )
    .expect("成员事件有效")
}

fn signed_status_event(
    signing_key: &Ed25519DeviceSigningKey,
    event_id: &str,
    status: &str,
    created_at: &str,
    lease_expires_at: &str,
    origin_server_timestamp: u64,
) -> MatrixTimelineEvent {
    let mut payload = status_payload(status, created_at, lease_expires_at);
    sign_payload(signing_key, &mut payload);
    status_timeline_event(event_id, payload, origin_server_timestamp)
}

fn status_payload(status: &str, created_at: &str, lease_expires_at: &str) -> Value {
    json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.agent.status.v1",
        "id": Uuid::now_v7(),
        "createdAt": created_at,
        "actor": {
            "agent": {
                "agentId": ACTOR_AGENT_ID,
                "displayName": "Presence Agent",
                "matrixUserId": ACTOR_MATRIX_ID,
                "avatarUrl": "https://example.test/avatar.png"
            },
            "instanceId": ACTOR_INSTANCE_ID,
            "provenance": "autonomous_agent"
        },
        "correlationId": Uuid::now_v7(),
        "status": status,
        "visibility": "coarse",
        "leaseExpiresAt": lease_expires_at
    })
}

fn status_timeline_event(
    event_id: &str,
    payload: Value,
    origin_server_timestamp: u64,
) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).expect("事件标识有效")),
        Some(MatrixUserId::new(ACTOR_MATRIX_ID).expect("用户标识有效")),
        MatrixEventType::new("io.github.rainyflash.agentroom.agent.status.v1")
            .expect("事件类型有效"),
        Some(ACTOR_INSTANCE_ID.to_owned()),
        None,
        Some(origin_server_timestamp),
        payload,
    )
    .expect("状态事件有效")
}

fn sign_payload(signing_key: &Ed25519DeviceSigningKey, payload: &mut Value) {
    let canonical = serde_jcs::to_vec(payload).expect("状态事件可规范化");
    let signature = signing_key.sign(&canonical).expect("状态事件可签名");
    payload.as_object_mut().expect("状态事件是对象").insert(
        "signature".to_owned(),
        Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
    );
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}
