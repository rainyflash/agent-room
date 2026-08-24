use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};

use agent_room_application::ports::{
    Clock, DeviceProofVerifier, DeviceSignature, MatrixDeviceId, MatrixUserId, PortFuture,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    handoffs::{
        ApproveHandoffRequest, ConsumedHandoffContext, EncryptedHandoffToDeviceGateway,
        EncryptedHandoffToDeviceRequest, HandoffAuthorizationDecision, HandoffAuthorizationFailure,
        HandoffAuthorizationGateway, HandoffAuthorizationRequest, HandoffDeliveryDependencies,
        HandoffDeliveryFailureKind, HandoffDeliveryOutcome, HandoffDeviceAddress,
        HandoffDirectoryFailure, HandoffInstanceDirectory, HandoffRecordOutcome, HandoffStore,
        HandoffStoreCommand, HandoffStoreCommandOutcome, HandoffStoreFailure,
        HandoffStoreFailureKind, HandoffTransportFailure, HandoffTransportFailureKind,
        OneShotHandoffPackage,
    },
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        DeviceSigningIdentity,
    },
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    devices::DevicePublicSigningKey,
    handoff::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffPermission,
        HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
        HandoffSourceEventId, HandoffStatus,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    messages::{MessageProvenance, MessageRiskFlag, MessageRiskFlags},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use agent_room_identity_adapter::{Ed25519DeviceProofVerifier, Ed25519DeviceSigningKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use uuid::Uuid;

const PROTOCOL_SCHEMA: &str =
    include_str!("../../../packages/protocol/schema/v1/agent-room.schema.json");

struct 测试签名身份(Ed25519DeviceSigningKey);

impl 测试签名身份 {
    fn generate() -> Self {
        Self(Ed25519DeviceSigningKey::generate().expect("测试私钥可生成"))
    }
}

impl DeviceSigningIdentity for 测试签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        self.0
            .public_key()
            .map_err(|_| BridgeCredentialFailure::new(BridgeCredentialFailureKind::Corrupt))
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.0
            .sign(message)
            .map_err(|_| BridgeCredentialFailure::new(BridgeCredentialFailureKind::Corrupt))
    }
}

struct 固定时钟(AtomicI64);

impl 固定时钟 {
    fn new(value: i64) -> Self {
        Self(AtomicI64::new(value))
    }
}

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).expect("测试时间有效")
    }
}

struct 固定授权 {
    decision: HandoffAuthorizationDecision,
    calls: Mutex<Vec<HandoffAuthorizationRequest>>,
}

impl 固定授权 {
    fn new(decision: HandoffAuthorizationDecision) -> Self {
        Self {
            decision,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl HandoffAuthorizationGateway for 固定授权 {
    fn authorize<'a>(
        &'a self,
        request: &'a HandoffAuthorizationRequest,
    ) -> PortFuture<'a, Result<HandoffAuthorizationDecision, HandoffAuthorizationFailure>> {
        self.calls.lock().expect("授权记录锁可用").push(*request);
        Box::pin(async move { Ok(self.decision) })
    }
}

struct 固定目录 {
    address: HandoffDeviceAddress,
}

impl HandoffInstanceDirectory for 固定目录 {
    fn resolve(
        &self,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, Result<HandoffDeviceAddress, HandoffDirectoryFailure>> {
        let address = self.address.clone();
        Box::pin(async move { Ok(address) })
    }
}

#[derive(Default)]
struct 记录加密传输 {
    requests: Mutex<Vec<EncryptedHandoffToDeviceRequest>>,
    failure: Mutex<Option<HandoffTransportFailureKind>>,
}

impl 记录加密传输 {
    fn fail_with(&self, failure: Option<HandoffTransportFailureKind>) {
        *self.failure.lock().expect("传输状态锁可用") = failure;
    }
}

impl EncryptedHandoffToDeviceGateway for 记录加密传输 {
    fn send<'a>(
        &'a self,
        request: &'a EncryptedHandoffToDeviceRequest,
    ) -> PortFuture<'a, Result<(), HandoffTransportFailure>> {
        self.requests
            .lock()
            .expect("传输记录锁可用")
            .push(request.clone());
        let failure = *self.failure.lock().expect("传输状态锁可用");
        Box::pin(
            async move { failure.map_or(Ok(()), |kind| Err(HandoffTransportFailure::new(kind))) },
        )
    }
}

#[derive(Default)]
struct 内存交付存储 {
    handoff: Mutex<Option<ContextHandoff>>,
    package: Mutex<Option<OneShotHandoffPackage>>,
}

impl 内存交付存储 {
    fn stored(&self) -> Option<ContextHandoff> {
        self.handoff.lock().expect("交付记录锁可用").clone()
    }
}

impl HandoffStore for 内存交付存储 {
    fn find(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<ContextHandoff>, HandoffStoreFailure>> {
        let handoff = self
            .handoff
            .lock()
            .expect("交付记录锁可用")
            .clone()
            .filter(|handoff| handoff.fields().id == handoff_id);
        Box::pin(async move { Ok(handoff) })
    }

    fn record_outgoing<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        let outcome = record(&self.handoff, handoff);
        Box::pin(async move { outcome })
    }

    fn accept_incoming<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
        package: &'a OneShotHandoffPackage,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        let outcome = record(&self.handoff, handoff);
        if outcome.is_ok() {
            let mut slot = self.package.lock().expect("上下文包锁可用");
            if let Some(existing) = slot.as_ref() {
                if existing != package {
                    return Box::pin(async {
                        Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict))
                    });
                }
            } else {
                *slot = Some(package.clone());
            }
        }
        Box::pin(async move { outcome })
    }

    fn apply(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
    ) -> PortFuture<'_, Result<HandoffStoreCommandOutcome, HandoffStoreFailure>> {
        let mut slot = self.handoff.lock().expect("交付记录锁可用");
        let Some(mut handoff) = slot.clone() else {
            return Box::pin(async {
                Err(HandoffStoreFailure::new(HandoffStoreFailureKind::NotFound))
            });
        };
        if handoff.fields().id != handoff_id {
            return Box::pin(async {
                Err(HandoffStoreFailure::new(HandoffStoreFailureKind::NotFound))
            });
        }
        let result = match command {
            HandoffStoreCommand::MarkDelivered { occurred_at } => {
                handoff.mark_delivered(occurred_at).map(|()| None)
            }
            HandoffStoreCommand::Consume {
                target_instance_id,
                occurred_at,
            } => {
                if handoff.fields().target_instance_id != target_instance_id {
                    return Box::pin(async {
                        Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict))
                    });
                }
                handoff
                    .consume(occurred_at)
                    .map(|()| self.package.lock().expect("上下文包锁可用").take())
            }
            HandoffStoreCommand::Decline {
                target_instance_id,
                occurred_at,
            } => {
                if handoff.fields().target_instance_id != target_instance_id {
                    return Box::pin(async {
                        Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict))
                    });
                }
                handoff.decline(occurred_at).map(|()| None)
            }
            HandoffStoreCommand::Revoke { occurred_at } => {
                handoff.revoke(occurred_at).map(|()| None)
            }
            HandoffStoreCommand::Expire { occurred_at } => {
                handoff.expire(occurred_at).map(|()| None)
            }
            HandoffStoreCommand::Fail { code, occurred_at } => {
                handoff.fail(code, occurred_at).map(|()| None)
            }
        };
        let Ok(package) = result else {
            return Box::pin(async {
                Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict))
            });
        };
        if matches!(
            handoff.status(),
            HandoffStatus::Declined
                | HandoffStatus::Revoked
                | HandoffStatus::Expired
                | HandoffStatus::Failed
        ) {
            self.package.lock().expect("上下文包锁可用").take();
        }
        *slot = Some(handoff.clone());
        let outcome = if let Some(package) = package {
            HandoffStoreCommandOutcome::Consumed(ConsumedHandoffContext::new(
                handoff,
                package.body().clone(),
            ))
        } else {
            HandoffStoreCommandOutcome::Updated(handoff)
        };
        Box::pin(async move { Ok(outcome) })
    }
}

fn record(
    slot: &Mutex<Option<ContextHandoff>>,
    handoff: &ContextHandoff,
) -> Result<HandoffRecordOutcome, HandoffStoreFailure> {
    let mut slot = slot.lock().expect("交付记录锁可用");
    match slot.as_ref() {
        Some(existing) if existing == handoff => {
            Ok(HandoffRecordOutcome::Existing(existing.clone()))
        }
        Some(_) => Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict)),
        None => {
            *slot = Some(handoff.clone());
            Ok(HandoffRecordOutcome::Created(handoff.clone()))
        }
    }
}

#[tokio::test]
async fn 用户批准后只向精确设备发送签名交付请求() {
    let fixture = 测试夹具::new(HandoffAuthorizationDecision::Allowed);

    let outcome = fixture
        .service()
        .approve_and_send(fixture.request())
        .await
        .expect("交付请求应成功");

    assert!(matches!(
        outcome,
        HandoffDeliveryOutcome::Submitted { reused: false, .. }
    ));
    assert_eq!(
        fixture.store.stored().map(|handoff| handoff.status()),
        Some(HandoffStatus::Approved)
    );
    let requests = fixture.transport.requests.lock().expect("传输记录锁可用");
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.target(), &fixture.target_address);
    assert_eq!(
        sent.event().event_type().as_str(),
        "org.agentroom.handoff.request.v1"
    );
    assert_eq!(
        sent.event().content()["actor"]["provenance"],
        "human_confirmed_agent"
    );
    assert_eq!(
        sent.event().content()["source"]["actor"]["provenance"],
        "autonomous_agent"
    );
    assert_protocol_event(sent.event().content());
    assert_valid_signature(fixture.signer.as_ref(), sent.event().content());
}

#[tokio::test]
async fn 未经授权不会落库也不会碰网络() {
    let fixture = 测试夹具::new(HandoffAuthorizationDecision::Denied);

    let failure = fixture
        .service()
        .approve_and_send(fixture.request())
        .await
        .expect_err("拒绝授权必须失败");

    assert_eq!(failure.kind(), HandoffDeliveryFailureKind::Unauthorized);
    assert!(fixture.store.stored().is_none());
    assert!(
        fixture
            .transport
            .requests
            .lock()
            .expect("传输记录锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 未知提交保留批准态并以同一事务标识安全重试() {
    let fixture = 测试夹具::new(HandoffAuthorizationDecision::Allowed);
    fixture
        .transport
        .fail_with(Some(HandoffTransportFailureKind::UnknownCommit));

    let first = fixture
        .service()
        .approve_and_send(fixture.request())
        .await
        .expect("未知提交是可对账结果");
    assert!(matches!(
        first,
        HandoffDeliveryOutcome::DeliveryUncertain { .. }
    ));
    fixture.transport.fail_with(None);

    let second = fixture
        .service()
        .approve_and_send(fixture.request())
        .await
        .expect("同一请求可安全重试");
    assert!(matches!(
        second,
        HandoffDeliveryOutcome::Submitted { reused: true, .. }
    ));
    let requests = fixture.transport.requests.lock().expect("传输记录锁可用");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].event().transaction_id(),
        requests[1].event().transaction_id()
    );
}

#[tokio::test]
async fn 目录返回错实例时记录稳定失败且不发送() {
    let mut fixture = 测试夹具::new(HandoffAuthorizationDecision::Allowed);
    fixture.target_address = HandoffDeviceAddress::new(
        fixture.target_agent_id,
        AgentInstanceId::from_uuid(Uuid::now_v7()),
        MatrixUserId::new("@target:matrix.test").expect("Matrix 用户标识有效"),
        MatrixDeviceId::new("TARGET-DEVICE").expect("Matrix 设备标识有效"),
    );

    let outcome = fixture
        .service()
        .approve_and_send(fixture.request())
        .await
        .expect("错实例形成稳定失败结果");

    assert!(matches!(outcome, HandoffDeliveryOutcome::Failed { .. }));
    assert_eq!(
        fixture.store.stored().map(|handoff| handoff.status()),
        Some(HandoffStatus::Failed)
    );
    assert!(
        fixture
            .transport
            .requests
            .lock()
            .expect("传输记录锁可用")
            .is_empty()
    );
}

struct 测试夹具 {
    requester_identity: BridgeAgentIdentity,
    source_identity: BridgeAgentIdentity,
    target_agent_id: AgentId,
    target_instance_id: AgentInstanceId,
    target_address: HandoffDeviceAddress,
    principal_id: PrincipalId,
    handoff_id: HandoffId,
    message_id: MessageId,
    content_id: ContentId,
    signer: Arc<测试签名身份>,
    clock: Arc<固定时钟>,
    authorization: Arc<固定授权>,
    transport: Arc<记录加密传输>,
    store: Arc<内存交付存储>,
}

impl 测试夹具 {
    fn new(decision: HandoffAuthorizationDecision) -> Self {
        let requester_agent_id = AgentId::from_uuid(Uuid::now_v7());
        let requester_instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        let source_agent_id = AgentId::from_uuid(Uuid::now_v7());
        let source_instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        let target_agent_id = AgentId::from_uuid(Uuid::now_v7());
        let target_instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        Self {
            requester_identity: BridgeAgentIdentity::new(
                requester_agent_id,
                "本地 Codex Agent",
                "@requester:matrix.test",
                requester_instance_id,
            )
            .expect("请求方身份有效"),
            source_identity: BridgeAgentIdentity::new(
                source_agent_id,
                "远端研究 Agent",
                "@source:matrix.test",
                source_instance_id,
            )
            .expect("来源身份有效"),
            target_agent_id,
            target_instance_id,
            target_address: HandoffDeviceAddress::new(
                target_agent_id,
                target_instance_id,
                MatrixUserId::new("@target:matrix.test").expect("Matrix 用户标识有效"),
                MatrixDeviceId::new("TARGET-DEVICE").expect("Matrix 设备标识有效"),
            ),
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            handoff_id: HandoffId::from_uuid(Uuid::now_v7()),
            message_id: MessageId::from_uuid(Uuid::now_v7()),
            content_id: ContentId::from_uuid(Uuid::now_v7()),
            signer: Arc::new(测试签名身份::generate()),
            clock: Arc::new(固定时钟::new(1_100)),
            authorization: Arc::new(固定授权::new(decision)),
            transport: Arc::new(记录加密传输::default()),
            store: Arc::new(内存交付存储::default()),
        }
    }

    fn service(&self) -> agent_room_bridge_core::handoffs::HandoffDeliveryService {
        agent_room_bridge_core::handoffs::HandoffDeliveryService::new(HandoffDeliveryDependencies {
            identity: self.requester_identity.clone(),
            signer: self.signer.clone(),
            clock: self.clock.clone(),
            authorization: self.authorization.clone(),
            directory: Arc::new(固定目录 {
                address: self.target_address.clone(),
            }),
            transport: self.transport.clone(),
            store: self.store.clone(),
        })
    }

    fn request(&self) -> ApproveHandoffRequest {
        ApproveHandoffRequest::new(
            ContextHandoff::propose(ContextHandoffFields {
                id: self.handoff_id,
                requester_agent_id: self.requester_identity.agent_id(),
                requester_instance_id: self.requester_identity.agent_instance_id(),
                source: HandoffSource::new(
                    MatrixRoomReference::new("!builders:matrix.test").expect("房间标识有效"),
                    HandoffSourceEventId::new("$source-event:matrix.test").expect("事件标识有效"),
                    self.message_id,
                    HandoffSourceActor::new(
                        self.source_identity.agent_id(),
                        self.source_identity.agent_instance_id(),
                        MessageProvenance::AutonomousAgent,
                    ),
                ),
                target_agent_id: self.target_agent_id,
                target_instance_id: self.target_instance_id,
                content: HandoffContentReference::new(
                    self.content_id,
                    Sha256Digest::from_bytes([7; 32]),
                    ContentByteLength::new(128).expect("正文长度有效"),
                    ContentMediaType::new("text/markdown").expect("媒体类型有效"),
                ),
                permissions: HandoffPermissions::new([
                    HandoffPermission::ReadText,
                    HandoffPermission::IncludeMetadata,
                ])
                .expect("内容范围有效"),
                purpose: HandoffPurpose::Summarize,
                risk_flags: MessageRiskFlags::new([
                    MessageRiskFlag::new("untrusted_instructions").expect("风险标签有效")
                ])
                .expect("风险集合有效"),
                proposed_at: UtcMillis::new(1_000).expect("提案时间有效"),
                expires_at: UtcMillis::new(2_000).expect("过期时间有效"),
            })
            .expect("交付提案有效"),
            self.source_identity.clone(),
            self.principal_id,
        )
        .expect("交付请求有效")
    }
}

fn assert_protocol_event(content: &Value) {
    let schema: Value = serde_json::from_str(PROTOCOL_SCHEMA).expect("协议 Schema 有效");
    let validator = jsonschema::validator_for(&schema).expect("协议 Schema 可编译");
    let errors = validator
        .iter_errors(content)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "交付事件违反协议：{errors:?}");
}

fn assert_valid_signature(signer: &测试签名身份, content: &Value) {
    let encoded = content["signature"].as_str().expect("签名存在");
    let signature = DeviceSignature::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("签名使用 Base64URL 编码"),
    )
    .expect("签名长度有效");
    let mut unsigned = content.clone();
    unsigned
        .as_object_mut()
        .expect("事件为对象")
        .remove("signature");
    let canonical = serde_jcs::to_vec(&unsigned).expect("事件可规范化");
    let public_key = signer.public_key().expect("测试公钥可读取");
    assert!(
        Ed25519DeviceProofVerifier.verify(&public_key, &canonical, &signature),
        "签名必须覆盖除 signature 外的完整 JCS 载荷"
    );
}
