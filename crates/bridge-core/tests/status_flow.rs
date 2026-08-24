use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, Ordering},
};

use agent_room_application::ports::{
    Clock, DeviceSignature, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation,
    MatrixResult, MatrixRoomId, MatrixStateEvent, PortFuture,
};
use agent_room_bridge_core::{
    ports::{
        AgentStatusStatePublisher, BridgeCredentialResult, DeviceSigningIdentity,
        StatusEventIdentifierFactory,
    },
    status::{
        AgentStatusIdentity, AgentStatusIntent, AgentStatusLeasePolicy,
        AgentStatusPublicationDependencies, AgentStatusPublicationService, AgentStatusRoomTarget,
        HostAgentState, StatusPublicationFailureKind, StatusPublicationOutcome,
        StatusPublicationReason,
    },
};
use agent_room_domain::{
    agent_status::{
        AgentStatusDetails, AgentStatusVisibility, AgentTaskProgress, AgentTaskSummary,
        AgentWorkStatus,
    },
    devices::DevicePublicSigningKey,
    ids::{AgentId, AgentInstanceId},
    time::{DurationMillis, UtcMillis},
};
use serde_json::Value;
use uuid::Uuid;

const STATUS_SCHEMA: &str =
    include_str!("../../../packages/protocol/schema/v1/agent-room.schema.json");

struct 测试时钟(AtomicI64);

impl 测试时钟 {
    fn new(value: i64) -> Self {
        Self(AtomicI64::new(value))
    }

    fn set(&self, value: UtcMillis) {
        self.0.store(value.value(), Ordering::SeqCst);
    }
}

impl Clock for 测试时钟 {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).expect("测试时间有效")
    }
}

struct 测试签名身份 {
    messages: Mutex<Vec<Vec<u8>>>,
}

impl DeviceSigningIdentity for 测试签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        Ok(DevicePublicSigningKey::new(vec![8; 32]).expect("测试公钥有效"))
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.messages
            .lock()
            .expect("签名记录锁可用")
            .push(message.to_vec());
        Ok(DeviceSignature::new(vec![9; 64]).expect("测试签名有效"))
    }
}

#[derive(Default)]
struct 记录状态发布器 {
    events: Mutex<Vec<(MatrixRoomId, MatrixStateEvent)>>,
    fail_next: AtomicBool,
}

impl AgentStatusStatePublisher for 记录状态发布器 {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Box::pin(async {
                Err(MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::DependencyUnavailable,
                ))
            });
        }
        self.events
            .lock()
            .expect("事件记录锁可用")
            .push((room_id.clone(), event.clone()));
        Box::pin(async {
            MatrixEventId::new("$status:matrix.test").map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }
}

struct 版本七标识工厂;

impl StatusEventIdentifierFactory for 版本七标识工厂 {
    fn event_id(&self) -> Uuid {
        Uuid::now_v7()
    }

    fn correlation_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

#[test]
fn 宿主状态完整映射到公开工作状态() {
    let cases = [
        (HostAgentState::Disconnected, AgentWorkStatus::Offline),
        (HostAgentState::Available, AgentWorkStatus::Idle),
        (HostAgentState::Running, AgentWorkStatus::Working),
        (HostAgentState::AwaitingInput, AgentWorkStatus::WaitingInput),
        (HostAgentState::Blocked, AgentWorkStatus::Blocked),
        (HostAgentState::Succeeded, AgentWorkStatus::Completed),
    ];

    for (host, expected) in cases {
        assert_eq!(host.work_status(), expected);
    }
}

#[tokio::test]
async fn 首次发布后重复调用不会制造续租风暴() {
    let fixture = fixture();
    let mut service = fixture.service();
    let target = target(AgentStatusVisibility::Coarse);
    let intent = detailed_intent(HostAgentState::Running, 2_500);

    let first = service
        .publish_if_due(&target, &intent, 0)
        .await
        .expect("首次发布成功");
    let duplicate = service
        .publish_if_due(&target, &intent, u64::MAX)
        .await
        .expect("重复检查成功");

    let StatusPublicationOutcome::Published {
        reason, renew_at, ..
    } = first
    else {
        panic!("首次调用必须发布");
    };
    assert_eq!(reason, StatusPublicationReason::Initial);
    assert_eq!(renew_at, time(106_000));
    assert_eq!(
        duplicate,
        StatusPublicationOutcome::NotDue {
            renew_at: time(106_000),
            lease_expires_at: time(301_000),
        }
    );

    let events = fixture.publisher.events.lock().expect("事件记录锁可用");
    assert_eq!(events.len(), 1);
    let event = &events[0].1;
    assert_eq!(event.state_key().as_str(), instance_id().to_string());
    assert_eq!(event.content()["status"], "working");
    assert_eq!(event.content()["visibility"], "coarse");
    assert!(event.content().get("taskSummary").is_none());
    assert!(event.content().get("startedAt").is_none());
    assert!(event.content().get("progress").is_none());
    assert!(event.content()["signature"].as_str().is_some());
    assert_protocol_event(event.content());

    let mut unsigned = event.content().clone();
    unsigned
        .as_object_mut()
        .expect("事件内容是对象")
        .remove("signature");
    let canonical = serde_jcs::to_vec(&unsigned).expect("载荷可规范化");
    assert_eq!(
        fixture
            .signer
            .messages
            .lock()
            .expect("签名记录锁可用")
            .as_slice(),
        [canonical]
    );
}

#[tokio::test]
async fn 状态与可见性变化立即发布而详情变化等待续租() {
    let fixture = fixture();
    let mut service = fixture.service();
    let coarse = target(AgentStatusVisibility::Coarse);
    let detailed = target(AgentStatusVisibility::Detailed);

    service
        .publish_if_due(&coarse, &detailed_intent(HostAgentState::Running, 1_000), 0)
        .await
        .expect("初始发布成功");
    fixture.clock.set(time(2_000));
    let transition = service
        .publish_if_due(
            &coarse,
            &detailed_intent(HostAgentState::AwaitingInput, 2_000),
            0,
        )
        .await
        .expect("状态转换发布成功");
    assert_published_reason(&transition, StatusPublicationReason::StatusChanged);

    fixture.clock.set(time(3_000));
    let visibility = service
        .publish_if_due(
            &detailed,
            &detailed_intent(HostAgentState::AwaitingInput, 3_000),
            0,
        )
        .await
        .expect("可见性变化发布成功");
    let renew_at = assert_published_reason(&visibility, StatusPublicationReason::VisibilityChanged);

    fixture.clock.set(time(4_000));
    let details_only = service
        .publish_if_due(
            &detailed,
            &detailed_intent(HostAgentState::AwaitingInput, 9_000),
            0,
        )
        .await
        .expect("详情检查成功");
    assert_eq!(
        details_only,
        StatusPublicationOutcome::NotDue {
            renew_at,
            lease_expires_at: time(303_000),
        }
    );

    fixture.clock.set(renew_at);
    let renewal = service
        .publish_if_due(
            &detailed,
            &detailed_intent(HostAgentState::AwaitingInput, 9_000),
            u64::MAX,
        )
        .await
        .expect("到期续租成功");
    assert_published_reason(&renewal, StatusPublicationReason::Renewal);
    let duplicate = service
        .publish_if_due(
            &detailed,
            &detailed_intent(HostAgentState::AwaitingInput, 9_000),
            u64::MAX,
        )
        .await
        .expect("续租后重复检查成功");
    assert!(matches!(duplicate, StatusPublicationOutcome::NotDue { .. }));

    let events = fixture.publisher.events.lock().expect("事件记录锁可用");
    assert_eq!(events.len(), 4);
    let detailed_event = &events[2].1.content();
    assert_eq!(detailed_event["taskSummary"], "正在执行已脱敏任务");
    assert_eq!(detailed_event["progress"], 0.3);
    assert_eq!(events[3].1.content()["progress"], 0.9);
}

#[tokio::test]
async fn 发布失败不会错误推进房间续租游标() {
    let fixture = fixture();
    fixture.publisher.fail_next.store(true, Ordering::SeqCst);
    let mut service = fixture.service();
    let target = target(AgentStatusVisibility::Coarse);
    let intent = AgentStatusIntent::new(HostAgentState::Available, None);

    let failure = service
        .publish_if_due(&target, &intent, 0)
        .await
        .expect_err("Matrix 失败必须上抛");
    assert_eq!(failure.kind(), StatusPublicationFailureKind::Matrix);
    assert_eq!(
        failure.matrix_failure().expect("保留 Matrix 错误").kind(),
        MatrixFailureKind::DependencyUnavailable
    );

    let retry = service
        .publish_if_due(&target, &intent, 0)
        .await
        .expect("确定失败后允许重试");
    assert_published_reason(&retry, StatusPublicationReason::Initial);
    assert_eq!(
        fixture
            .publisher
            .events
            .lock()
            .expect("事件记录锁可用")
            .len(),
        1
    );
}

#[test]
fn 续租策略拒绝可能晚于租约到期的配置() {
    assert!(
        AgentStatusLeasePolicy::new(duration(300_000), duration(290_000), duration(15_000))
            .is_err()
    );
    assert!(
        AgentStatusLeasePolicy::new(duration(300_000), duration(120_000), duration(15_000)).is_ok()
    );
}

fn assert_published_reason(
    outcome: &StatusPublicationOutcome,
    expected: StatusPublicationReason,
) -> UtcMillis {
    let StatusPublicationOutcome::Published {
        reason, renew_at, ..
    } = outcome
    else {
        panic!("本次调用必须发布");
    };
    assert_eq!(*reason, expected);
    *renew_at
}

fn detailed_intent(state: HostAgentState, basis_points: u16) -> AgentStatusIntent {
    AgentStatusIntent::new(
        state,
        Some(AgentStatusDetails::new(
            Some(AgentTaskSummary::new("正在执行已脱敏任务").expect("摘要有效")),
            Some(time(500)),
            Some(AgentTaskProgress::from_basis_points(basis_points).expect("进度有效")),
        )),
    )
}

fn target(visibility: AgentStatusVisibility) -> AgentStatusRoomTarget {
    AgentStatusRoomTarget::new(
        MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效"),
        visibility,
    )
}

struct 测试夹具 {
    clock: Arc<测试时钟>,
    signer: Arc<测试签名身份>,
    publisher: Arc<记录状态发布器>,
}

impl 测试夹具 {
    fn service(&self) -> AgentStatusPublicationService {
        AgentStatusPublicationService::new(
            AgentStatusPublicationDependencies {
                identity: AgentStatusIdentity::new(
                    agent_id(),
                    "构建助手",
                    "@build-agent:matrix.test",
                    instance_id(),
                )
                .expect("状态身份有效"),
                signer: self.signer.clone(),
                publisher: self.publisher.clone(),
                identifiers: Arc::new(版本七标识工厂),
                clock: self.clock.clone(),
            },
            AgentStatusLeasePolicy::new(duration(300_000), duration(120_000), duration(15_000))
                .expect("租约策略有效"),
        )
    }
}

fn fixture() -> 测试夹具 {
    测试夹具 {
        clock: Arc::new(测试时钟::new(1_000)),
        signer: Arc::new(测试签名身份 {
            messages: Mutex::new(Vec::new()),
        }),
        publisher: Arc::new(记录状态发布器::default()),
    }
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(
        Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a4").expect("UUID 有效"),
    )
}

fn duration(value: u64) -> DurationMillis {
    DurationMillis::new(value).expect("测试时长有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}

fn assert_protocol_event(content: &Value) {
    let schema: Value = serde_json::from_str(STATUS_SCHEMA).expect("协议 Schema 有效");
    let validator = jsonschema::validator_for(&schema).expect("协议 Schema 可编译");
    let errors = validator
        .iter_errors(content)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "状态事件违反协议：{errors:?}");
}
