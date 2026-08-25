use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

use agent_room_application::ports::{
    Clock, DeviceProofVerifier, DeviceSignature, MatrixDeviceId, MatrixEventType, PortFuture,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{
        AgentEventAuthenticationDecision, AgentEventAuthenticationFailure, AgentEventAuthenticator,
    },
    handoffs::{
        ConsumedHandoffContext, DecryptedHandoffToDeviceEvent, EncryptedHandoffToDeviceGateway,
        EncryptedHandoffToDeviceRequest, HandoffAuthorizationDecision, HandoffAuthorizationFailure,
        HandoffAuthorizationGateway, HandoffAuthorizationRequest, HandoffContentFailure,
        HandoffContentGateway, HandoffContentRead, HandoffDeviceAddress, HandoffDirectoryFailure,
        HandoffInstanceDirectory, HandoffReceiptDelivery, HandoffReceiptRecord,
        HandoffReceptionDependencies, HandoffReceptionFailureKind, HandoffReceptionOutcome,
        HandoffRecordOutcome, HandoffStore, HandoffStoreCommand, HandoffStoreCommandOutcome,
        HandoffStoreFailure, HandoffStoreFailureKind, HandoffTransportFailure,
        OneShotHandoffPackage,
    },
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        DeviceSigningIdentity,
    },
};
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    devices::DevicePublicSigningKey,
    handoff::{ContextHandoff, HandoffStatus},
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    time::UtcMillis,
};
use agent_room_identity_adapter::{Ed25519DeviceProofVerifier, Ed25519DeviceSigningKey};
use agent_room_protocol_conformance::generated::{
    ActorRef, AgentRef, ContentRef, HandoffPermission, HandoffPurpose, HandoffRequestEvent,
    HandoffSource, Provenance,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
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

struct 可调时钟(AtomicI64);

impl 可调时钟 {
    fn new(value: i64) -> Self {
        Self(AtomicI64::new(value))
    }

    fn set(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

impl Clock for 可调时钟 {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).expect("测试时间有效")
    }
}

struct 验签器 {
    public_key: DevicePublicSigningKey,
}

impl AgentEventAuthenticator for 验签器 {
    fn authenticate<'a>(
        &'a self,
        _agent_id: AgentId,
        _instance_id: AgentInstanceId,
        _observed_at: UtcMillis,
        canonical_event: &'a [u8],
        signature: &'a DeviceSignature,
    ) -> PortFuture<'a, Result<AgentEventAuthenticationDecision, AgentEventAuthenticationFailure>>
    {
        let decision =
            if Ed25519DeviceProofVerifier.verify(&self.public_key, canonical_event, signature) {
                AgentEventAuthenticationDecision::Trusted
            } else {
                AgentEventAuthenticationDecision::InvalidSignature
            };
        Box::pin(async move { Ok(decision) })
    }
}

struct 固定授权(HandoffAuthorizationDecision);

impl HandoffAuthorizationGateway for 固定授权 {
    fn authorize<'a>(
        &'a self,
        _request: &'a HandoffAuthorizationRequest,
    ) -> PortFuture<'a, Result<HandoffAuthorizationDecision, HandoffAuthorizationFailure>> {
        Box::pin(async move { Ok(self.0) })
    }
}

struct 固定目录(HandoffDeviceAddress);

impl HandoffInstanceDirectory for 固定目录 {
    fn resolve(
        &self,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, Result<HandoffDeviceAddress, HandoffDirectoryFailure>> {
        let address = self.0.clone();
        Box::pin(async move { Ok(address) })
    }
}

struct 固定正文 {
    body: Arc<[u8]>,
    media_type: ContentMediaType,
    reads: AtomicUsize,
}

impl HandoffContentGateway for 固定正文 {
    fn read<'a>(
        &'a self,
        _handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffContentRead, HandoffContentFailure>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let content = HandoffContentRead {
            body: self.body.clone(),
            media_type: self.media_type.clone(),
        };
        Box::pin(async move { Ok(content) })
    }
}

#[derive(Default)]
struct 记录传输(Mutex<Vec<EncryptedHandoffToDeviceRequest>>);

impl EncryptedHandoffToDeviceGateway for 记录传输 {
    fn send<'a>(
        &'a self,
        request: &'a EncryptedHandoffToDeviceRequest,
    ) -> PortFuture<'a, Result<(), HandoffTransportFailure>> {
        self.0.lock().expect("传输记录锁可用").push(request.clone());
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct 内存状态 {
    handoffs: BTreeMap<HandoffId, ContextHandoff>,
    packages: BTreeMap<HandoffId, OneShotHandoffPackage>,
}

#[derive(Default)]
struct 内存交付存储(Mutex<内存状态>);

impl 内存交付存储 {
    fn status(&self, handoff_id: HandoffId) -> Option<HandoffStatus> {
        self.0
            .lock()
            .expect("存储锁可用")
            .handoffs
            .get(&handoff_id)
            .map(ContextHandoff::status)
    }
}

impl HandoffStore for 内存交付存储 {
    fn find(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<ContextHandoff>, HandoffStoreFailure>> {
        let handoff = self
            .0
            .lock()
            .expect("存储锁可用")
            .handoffs
            .get(&handoff_id)
            .cloned();
        Box::pin(async move { Ok(handoff) })
    }

    fn record_outgoing<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        let outcome = record(&mut self.0.lock().expect("存储锁可用"), handoff, None);
        Box::pin(async move { outcome })
    }

    fn accept_incoming<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
        package: &'a OneShotHandoffPackage,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        let outcome = record(
            &mut self.0.lock().expect("存储锁可用"),
            handoff,
            Some(package),
        );
        Box::pin(async move { outcome })
    }

    fn apply(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
    ) -> PortFuture<'_, Result<HandoffStoreCommandOutcome, HandoffStoreFailure>> {
        let outcome = apply_command(&mut self.0.lock().expect("存储锁可用"), handoff_id, command);
        Box::pin(async move { outcome })
    }

    fn apply_receipt<'a>(
        &'a self,
        receipt: &'a HandoffReceiptRecord,
    ) -> PortFuture<'a, Result<ContextHandoff, HandoffStoreFailure>> {
        let mut state = self.0.lock().expect("存储锁可用");
        let result = state
            .handoffs
            .get_mut(&receipt.handoff_id())
            .ok_or_else(|| HandoffStoreFailure::new(HandoffStoreFailureKind::NotFound))
            .and_then(|handoff| {
                receipt
                    .apply_to(handoff)
                    .map_err(|_| HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict))?;
                Ok(handoff.clone())
            });
        Box::pin(async move { result })
    }
}

fn record(
    state: &mut 内存状态,
    handoff: &ContextHandoff,
    package: Option<&OneShotHandoffPackage>,
) -> Result<HandoffRecordOutcome, HandoffStoreFailure> {
    let id = handoff.fields().id;
    if let Some(existing) = state.handoffs.get(&id) {
        if existing.fields() != handoff.fields()
            || existing.approved_at() != handoff.approved_at()
            || existing.approved_by_principal_id() != handoff.approved_by_principal_id()
        {
            return Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict));
        }
        return Ok(HandoffRecordOutcome::Existing(existing.clone()));
    }
    state.handoffs.insert(id, handoff.clone());
    if let Some(package) = package {
        state.packages.insert(id, package.clone());
    }
    Ok(HandoffRecordOutcome::Created(handoff.clone()))
}

fn apply_command(
    state: &mut 内存状态,
    handoff_id: HandoffId,
    command: HandoffStoreCommand,
) -> Result<HandoffStoreCommandOutcome, HandoffStoreFailure> {
    let mut handoff = state
        .handoffs
        .get(&handoff_id)
        .cloned()
        .ok_or_else(|| HandoffStoreFailure::new(HandoffStoreFailureKind::NotFound))?;
    let package = match command {
        HandoffStoreCommand::MarkDelivered { occurred_at } => {
            handoff.mark_delivered(occurred_at).map(|()| None)
        }
        HandoffStoreCommand::Consume {
            target_instance_id,
            occurred_at,
        } => {
            if handoff.fields().target_instance_id != target_instance_id {
                return Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict));
            }
            handoff
                .consume(occurred_at)
                .map(|()| state.packages.remove(&handoff_id))
        }
        HandoffStoreCommand::Decline {
            target_instance_id,
            occurred_at,
        } => {
            if handoff.fields().target_instance_id != target_instance_id {
                return Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict));
            }
            handoff.decline(occurred_at).map(|()| None)
        }
        HandoffStoreCommand::Revoke {
            target_instance_id,
            occurred_at,
        } => {
            if handoff.fields().target_instance_id != target_instance_id {
                return Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict));
            }
            handoff.revoke(occurred_at).map(|()| None)
        }
        HandoffStoreCommand::Expire {
            target_instance_id,
            occurred_at,
        } => {
            if handoff.fields().target_instance_id != target_instance_id {
                return Err(HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict));
            }
            handoff.expire(occurred_at).map(|()| None)
        }
        HandoffStoreCommand::Fail { code, occurred_at } => {
            handoff.fail(code, occurred_at).map(|()| None)
        }
    }
    .map_err(|_| HandoffStoreFailure::new(HandoffStoreFailureKind::AlreadyResolved))?;
    if matches!(
        handoff.status(),
        HandoffStatus::Declined
            | HandoffStatus::Revoked
            | HandoffStatus::Expired
            | HandoffStatus::Failed
    ) {
        state.packages.remove(&handoff_id);
    }
    state.handoffs.insert(handoff_id, handoff.clone());
    if let Some(package) = package {
        Ok(HandoffStoreCommandOutcome::Consumed(
            ConsumedHandoffContext::new(handoff, package.body().clone()),
        ))
    } else {
        Ok(HandoffStoreCommandOutcome::Updated(handoff))
    }
}

#[tokio::test]
async fn 合法交付校验正文后原子落库并发送已送达回执() {
    let fixture = 测试夹具::new();
    let event = fixture.request_event(fixture.target_identity.agent_instance_id());

    let outcome = fixture
        .service()
        .receive(&event)
        .await
        .expect("合法请求应被接收");

    assert!(matches!(
        outcome,
        HandoffReceptionOutcome::Delivered {
            replayed: false,
            receipt: HandoffReceiptDelivery::Confirmed,
            ..
        }
    ));
    assert_eq!(
        fixture.store.status(fixture.handoff_id),
        Some(HandoffStatus::Delivered)
    );
    assert_eq!(fixture.content.reads.load(Ordering::SeqCst), 1);
    let receipts = fixture.transport.0.lock().expect("回执记录锁可用");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].event().content()["status"], "delivered");
    assert_eq!(receipts[0].target(), &fixture.requester_address);
    assert_protocol_event(receipts[0].event().content());
    assert_valid_signature(
        fixture.target_signer.as_ref(),
        receipts[0].event().content(),
    );
}

#[tokio::test]
async fn 错实例与篡改签名都不能触发正文下载() {
    let fixture = 测试夹具::new();
    let wrong_target = fixture.request_event(AgentInstanceId::from_uuid(Uuid::now_v7()));
    let wrong_target_failure = fixture
        .service()
        .receive(&wrong_target)
        .await
        .expect_err("错实例必须失败");
    assert_eq!(
        wrong_target_failure.kind(),
        HandoffReceptionFailureKind::WrongTarget
    );

    let valid = fixture.request_event(fixture.target_identity.agent_instance_id());
    let mut tampered_content = valid.content().clone();
    tampered_content["purpose"] = Value::String("reply_draft".to_owned());
    let tampered = DecryptedHandoffToDeviceEvent::new(
        valid.sender().clone(),
        valid.event_type().clone(),
        tampered_content,
    )
    .expect("篡改后的对象仍是结构化事件");
    let tampered_failure = fixture
        .service()
        .receive(&tampered)
        .await
        .expect_err("签名覆盖字段被篡改后必须失败");
    assert_eq!(
        tampered_failure.kind(),
        HandoffReceptionFailureKind::UntrustedSender
    );
    assert_eq!(fixture.content.reads.load(Ordering::SeqCst), 0);
    assert!(fixture.store.status(fixture.handoff_id).is_none());
}

#[tokio::test]
async fn 下载正文摘要不符时不得建立上下文包() {
    let mut fixture = 测试夹具::new();
    fixture.content = Arc::new(固定正文 {
        body: Arc::from(&b"tampered"[..]),
        media_type: ContentMediaType::new("text/markdown").expect("媒体类型有效"),
        reads: AtomicUsize::new(0),
    });
    let event = fixture.request_event(fixture.target_identity.agent_instance_id());

    let failure = fixture
        .service()
        .receive(&event)
        .await
        .expect_err("正文摘要不符必须失败");

    assert_eq!(
        failure.kind(),
        HandoffReceptionFailureKind::IntegrityMismatch
    );
    assert!(fixture.store.status(fixture.handoff_id).is_none());
}

#[tokio::test]
async fn 重放不重复下载且消费正文严格只成功一次() {
    let fixture = 测试夹具::new();
    let event = fixture.request_event(fixture.target_identity.agent_instance_id());
    let service = fixture.service();
    service.receive(&event).await.expect("首次接收成功");

    let replay = service
        .receive(&event)
        .await
        .expect("同一签名请求可幂等重放");
    assert!(matches!(
        replay,
        HandoffReceptionOutcome::Delivered { replayed: true, .. }
    ));
    assert_eq!(fixture.content.reads.load(Ordering::SeqCst), 1);

    let pending = service
        .inspect_pending(fixture.handoff_id)
        .await
        .expect("消费前可读取经过目标校验的元数据");
    assert_eq!(pending.status(), HandoffStatus::Delivered);

    let consumed = service
        .consume(fixture.handoff_id)
        .await
        .expect("首次消费成功");
    assert_eq!(consumed.context().body().as_ref(), fixture.body.as_ref());
    assert_eq!(
        consumed.context().handoff().status(),
        HandoffStatus::Consumed
    );
    assert!(service.consume(fixture.handoff_id).await.is_err());
    assert_eq!(
        fixture.store.status(fixture.handoff_id),
        Some(HandoffStatus::Consumed)
    );
    let resolved = service
        .inspect_pending(fixture.handoff_id)
        .await
        .expect_err("已消费交接不能再声明为待领取");
    assert_eq!(resolved.kind(), HandoffReceptionFailureKind::Store);
    assert_eq!(
        resolved.store_failure().expect("保留存储失败类别").kind(),
        HandoffStoreFailureKind::AlreadyResolved
    );
    let receipts = fixture.transport.0.lock().expect("回执记录锁可用");
    assert_eq!(
        receipts.last().expect("存在消费回执").event().content()["status"],
        "consumed"
    );
}

#[tokio::test]
async fn 到期请求在验签和下载前被拒绝() {
    let fixture = 测试夹具::new();
    fixture.clock.set(2_000);
    let event = fixture.request_event(fixture.target_identity.agent_instance_id());

    let failure = fixture
        .service()
        .receive(&event)
        .await
        .expect_err("到期请求必须失败");

    assert_eq!(failure.kind(), HandoffReceptionFailureKind::Expired);
    assert_eq!(fixture.content.reads.load(Ordering::SeqCst), 0);
    assert!(fixture.store.status(fixture.handoff_id).is_none());
}

#[tokio::test]
async fn 拒绝撤销和到期都会销毁正文并发布对应回执() {
    let declined = 测试夹具::new();
    let declined_service = declined.service();
    declined_service
        .receive(&declined.request_event(declined.target_identity.agent_instance_id()))
        .await
        .expect("交付成功");
    let declined_outcome = declined_service
        .decline(declined.handoff_id)
        .await
        .expect("拒绝成功");
    assert_eq!(declined_outcome.status(), HandoffStatus::Declined);
    assert!(declined_service.consume(declined.handoff_id).await.is_err());
    assert_last_receipt(&declined, "declined");

    let revoked = 测试夹具::new();
    let revoked_service = revoked.service();
    revoked_service
        .receive(&revoked.request_event(revoked.target_identity.agent_instance_id()))
        .await
        .expect("交付成功");
    let revoked_outcome = revoked_service
        .revoke(revoked.handoff_id)
        .await
        .expect("撤销成功");
    assert_eq!(revoked_outcome.status(), HandoffStatus::Revoked);
    assert!(revoked_service.consume(revoked.handoff_id).await.is_err());
    assert_last_receipt(&revoked, "revoked");

    let expired = 测试夹具::new();
    let expired_service = expired.service();
    expired_service
        .receive(&expired.request_event(expired.target_identity.agent_instance_id()))
        .await
        .expect("交付成功");
    expired.clock.set(2_000);
    let expired_outcome = expired_service
        .expire(expired.handoff_id)
        .await
        .expect("到期关闭成功");
    assert_eq!(expired_outcome.status(), HandoffStatus::Expired);
    assert!(expired_service.consume(expired.handoff_id).await.is_err());
    assert_last_receipt(&expired, "expired");
}

struct 测试夹具 {
    requester_identity: BridgeAgentIdentity,
    source_identity: BridgeAgentIdentity,
    target_identity: BridgeAgentIdentity,
    requester_address: HandoffDeviceAddress,
    principal_id: PrincipalId,
    handoff_id: HandoffId,
    message_id: MessageId,
    content_id: ContentId,
    body: Arc<[u8]>,
    requester_signer: Arc<测试签名身份>,
    target_signer: Arc<测试签名身份>,
    clock: Arc<可调时钟>,
    content: Arc<固定正文>,
    transport: Arc<记录传输>,
    store: Arc<内存交付存储>,
}

impl 测试夹具 {
    fn new() -> Self {
        let requester_identity = identity("请求 Agent", "@requester:matrix.test");
        let source_identity = identity("研究 Agent", "@source:matrix.test");
        let target_identity = identity("本地 Codex", "@target:matrix.test");
        let requester_address = HandoffDeviceAddress::new(
            requester_identity.agent_id(),
            requester_identity.agent_instance_id(),
            requester_identity.matrix_user_id().clone(),
            MatrixDeviceId::new("REQUESTER-DEVICE").expect("Matrix 设备标识有效"),
        );
        let body = Arc::<[u8]>::from(&b"# Verified context"[..]);
        Self {
            requester_identity,
            source_identity,
            target_identity,
            requester_address,
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            handoff_id: HandoffId::from_uuid(Uuid::now_v7()),
            message_id: MessageId::from_uuid(Uuid::now_v7()),
            content_id: ContentId::from_uuid(Uuid::now_v7()),
            body: body.clone(),
            requester_signer: Arc::new(测试签名身份::generate()),
            target_signer: Arc::new(测试签名身份::generate()),
            clock: Arc::new(可调时钟::new(1_100)),
            content: Arc::new(固定正文 {
                body,
                media_type: ContentMediaType::new("text/markdown").expect("媒体类型有效"),
                reads: AtomicUsize::new(0),
            }),
            transport: Arc::new(记录传输::default()),
            store: Arc::new(内存交付存储::default()),
        }
    }

    fn service(&self) -> agent_room_bridge_core::handoffs::HandoffReceptionService {
        agent_room_bridge_core::handoffs::HandoffReceptionService::new(
            HandoffReceptionDependencies {
                identity: self.target_identity.clone(),
                signer: self.target_signer.clone(),
                clock: self.clock.clone(),
                authenticator: Arc::new(验签器 {
                    public_key: self
                        .requester_signer
                        .public_key()
                        .expect("请求方公钥可读取"),
                }),
                authorization: Arc::new(固定授权(HandoffAuthorizationDecision::Allowed)),
                directory: Arc::new(固定目录(self.requester_address.clone())),
                transport: self.transport.clone(),
                content: self.content.clone(),
                store: self.store.clone(),
            },
        )
    }

    fn request_event(&self, target_instance_id: AgentInstanceId) -> DecryptedHandoffToDeviceEvent {
        let digest = Sha256Digest::from_bytes(Sha256::digest(self.body.as_ref()).into());
        let mut content = serde_json::to_value(HandoffRequestEvent {
            actor: actor(&self.requester_identity, Provenance::HumanConfirmedAgent),
            approved_at: "1970-01-01T00:00:01.000Z".to_owned(),
            approved_by_principal_id: self.principal_id.to_string(),
            content: ContentRef {
                content_id: self.content_id.to_string(),
                digest_sha256: hex(digest.as_bytes()),
                fetch_mode: "on_demand".to_owned(),
                media_type: "text/markdown".to_owned(),
                size_bytes: u64::try_from(self.body.len()).expect("测试正文长度可转换"),
                extensions: BTreeMap::new(),
            },
            correlation_id: self.handoff_id.to_string(),
            created_at: "1970-01-01T00:00:01.000Z".to_owned(),
            event_type: "org.agentroom.handoff.request.v1".to_owned(),
            expires_at: "1970-01-01T00:00:02.000Z".to_owned(),
            id: self.handoff_id.to_string(),
            permissions: vec![
                HandoffPermission::ReadText,
                HandoffPermission::IncludeMetadata,
            ],
            purpose: HandoffPurpose::Summarize,
            risk_flags: vec!["untrusted_instructions".to_owned()],
            schema_version: "1.0".to_owned(),
            signature: String::new(),
            source: HandoffSource {
                actor: actor(&self.source_identity, Provenance::AutonomousAgent),
                event_id: "$source:matrix.test".to_owned(),
                message_id: self.message_id.to_string(),
                room_id: "!builders:matrix.test".to_owned(),
                extensions: BTreeMap::new(),
            },
            target_agent_id: self.target_identity.agent_id().to_string(),
            target_instance_id: target_instance_id.to_string(),
            extensions: BTreeMap::new(),
        })
        .expect("交付事件可序列化");
        content
            .as_object_mut()
            .expect("事件为对象")
            .remove("signature");
        let canonical = serde_jcs::to_vec(&content).expect("事件可规范化");
        let signature = self.requester_signer.sign(&canonical).expect("事件可签名");
        content.as_object_mut().expect("事件为对象").insert(
            "signature".to_owned(),
            Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
        );
        DecryptedHandoffToDeviceEvent::new(
            self.requester_identity.matrix_user_id().clone(),
            MatrixEventType::new("org.agentroom.handoff.request.v1").expect("事件类型有效"),
            content,
        )
        .expect("解密事件有效")
    }
}

fn identity(name: &str, matrix_user_id: &str) -> BridgeAgentIdentity {
    BridgeAgentIdentity::new(
        AgentId::from_uuid(Uuid::now_v7()),
        name,
        matrix_user_id,
        AgentInstanceId::from_uuid(Uuid::now_v7()),
    )
    .expect("Agent 身份有效")
}

fn actor(identity: &BridgeAgentIdentity, provenance: Provenance) -> ActorRef {
    ActorRef {
        agent: AgentRef {
            agent_id: identity.agent_id().to_string(),
            avatar_url: None,
            display_name: identity.display_name().to_owned(),
            matrix_user_id: identity.matrix_user_id().as_str().to_owned(),
            extensions: BTreeMap::new(),
        },
        instance_id: identity.agent_instance_id().to_string(),
        provenance,
        extensions: BTreeMap::new(),
    }
}

fn assert_last_receipt(fixture: &测试夹具, expected_status: &str) {
    let receipts = fixture.transport.0.lock().expect("回执记录锁可用");
    assert_eq!(
        receipts.last().expect("终态回执存在").event().content()["status"],
        expected_status
    );
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn assert_protocol_event(content: &Value) {
    let schema: Value = serde_json::from_str(PROTOCOL_SCHEMA).expect("协议 Schema 有效");
    let validator = jsonschema::validator_for(&schema).expect("协议 Schema 可编译");
    let errors = validator
        .iter_errors(content)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "交付回执违反协议：{errors:?}");
}

fn assert_valid_signature(signer: &测试签名身份, content: &Value) {
    let signature = DeviceSignature::new(
        URL_SAFE_NO_PAD
            .decode(content["signature"].as_str().expect("签名存在"))
            .expect("签名使用 Base64URL 编码"),
    )
    .expect("签名长度有效");
    let mut unsigned = content.clone();
    unsigned
        .as_object_mut()
        .expect("事件为对象")
        .remove("signature");
    let canonical = serde_jcs::to_vec(&unsigned).expect("事件可规范化");
    assert!(
        Ed25519DeviceProofVerifier.verify(
            &signer.public_key().expect("测试公钥可读取"),
            &canonical,
            &signature,
        ),
        "回执签名必须覆盖除 signature 外的完整 JCS 载荷"
    );
}
