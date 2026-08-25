use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use agent_room_application::ports::{
    DeviceProofVerifier, DeviceSignature, MatrixAcceptedEvent, MatrixEvent, MatrixEventId,
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
    MatrixTransactionId, PortFuture,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        AutomationAuthorizationDenial, AutomationAuthorizationFailure,
        AutomationAuthorizationGateway, AutomationAuthorizationRequest,
        AutomationAuthorizationResult, EditMessageRequest, MessageBody, MessageContentBindRequest,
        MessageContentFailure, MessageContentFailureKind, MessageContentGateway,
        MessageContentRecord, MessageContentRedactRequest, MessageContentUploadRequest,
        MessageEventPublisher, MessagePublicationDependencies, MessagePublicationFailureKind,
        MessagePublicationOutcome, MessagePublicationService, MessageStoreFailure,
        MessageStoreFailureKind, MessageSubmissionClaim, MessageSubmissionClaimOutcome,
        MessageSubmissionRecord, MessageSubmissionRepository, MessageSubmissionState,
        RedactMessageRequest, SendMessageRequest,
    },
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        DeviceSigningIdentity,
    },
};
use agent_room_domain::{
    content::{ContentEncryptionMode, ContentMediaType},
    devices::DevicePublicSigningKey,
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, ContentId, MessageId, MessageSubmissionId,
        RoomCatalogId,
    },
    messages::{
        MessageLanguage, MessagePreview, MessageProvenance, MessageRelation, MessageRiskFlag,
        MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
    },
};
use agent_room_identity_adapter::{Ed25519DeviceProofVerifier, Ed25519DeviceSigningKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use uuid::Uuid;

const MESSAGE_SCHEMA: &str =
    include_str!("../../../packages/protocol/schema/v1/agent-room.schema.json");

struct 测试签名身份(Ed25519DeviceSigningKey);

struct 允许自动授权;

impl AutomationAuthorizationGateway for 允许自动授权 {
    fn authorize<'a>(
        &'a self,
        _request: &'a AutomationAuthorizationRequest,
    ) -> PortFuture<'a, AutomationAuthorizationResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

struct 拒绝自动授权;

impl AutomationAuthorizationGateway for 拒绝自动授权 {
    fn authorize<'a>(
        &'a self,
        _request: &'a AutomationAuthorizationRequest,
    ) -> PortFuture<'a, AutomationAuthorizationResult<()>> {
        Box::pin(async {
            Err(AutomationAuthorizationFailure::denied(
                AutomationAuthorizationDenial::RateLimitExceeded,
            ))
        })
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum 发布行为 {
    接受,
    接受但响应未知,
}

struct 记录消息发布器 {
    behaviors: Mutex<VecDeque<发布行为>>,
    visible_events: Mutex<Vec<(MatrixRoomId, MatrixEvent, MatrixEventId)>>,
    accepted: Mutex<BTreeMap<MatrixTransactionId, MatrixEventId>>,
    audit: Arc<Mutex<Vec<&'static str>>>,
}

impl 记录消息发布器 {
    fn new(audit: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            behaviors: Mutex::new(VecDeque::new()),
            visible_events: Mutex::new(Vec::new()),
            accepted: Mutex::new(BTreeMap::new()),
            audit,
        }
    }

    fn enqueue(&self, behavior: 发布行为) {
        self.behaviors
            .lock()
            .expect("发布行为锁可用")
            .push_back(behavior);
    }

    fn events(&self) -> Vec<(MatrixRoomId, MatrixEvent, MatrixEventId)> {
        self.visible_events.lock().expect("可见事件锁可用").clone()
    }

    fn accepted_event_id(&self, transaction_id: &MatrixTransactionId) -> MatrixEventId {
        self.accepted
            .lock()
            .expect("接受记录锁可用")
            .get(transaction_id)
            .expect("事务已经被服务端接受")
            .clone()
    }
}

impl MessageEventPublisher for 记录消息发布器 {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        self.audit.lock().expect("审计锁可用").push("发布事件");
        if let Some(event_id) = self
            .accepted
            .lock()
            .expect("接受记录锁可用")
            .get(event.transaction_id())
            .cloned()
        {
            let accepted = MatrixAcceptedEvent::new(event.transaction_id().clone(), event_id);
            return Box::pin(async move { Ok(accepted) });
        }

        let behavior = self
            .behaviors
            .lock()
            .expect("发布行为锁可用")
            .pop_front()
            .unwrap_or(发布行为::接受);
        let ordinal = self.visible_events.lock().expect("可见事件锁可用").len() + 1;
        let event_id = MatrixEventId::new(format!("$message-{ordinal}:matrix.test"))
            .expect("测试事件标识有效");
        self.accepted
            .lock()
            .expect("接受记录锁可用")
            .insert(event.transaction_id().clone(), event_id.clone());
        self.visible_events.lock().expect("可见事件锁可用").push((
            room_id.clone(),
            event.clone(),
            event_id.clone(),
        ));

        Box::pin(async move {
            match behavior {
                发布行为::接受 => Ok(MatrixAcceptedEvent::new(
                    event.transaction_id().clone(),
                    event_id,
                )),
                发布行为::接受但响应未知 => Err(MatrixFailure::new(
                    MatrixOperation::SendEvent,
                    MatrixFailureKind::UnknownCommit,
                )),
            }
        })
    }
}

struct 内存内容网关 {
    uploads: Mutex<BTreeMap<Uuid, MessageContentRecord>>,
    upload_calls: AtomicUsize,
    bind_calls: AtomicUsize,
    fail_next_bind: AtomicBool,
    redacted: Mutex<Vec<ContentId>>,
    audit: Arc<Mutex<Vec<&'static str>>>,
}

impl 内存内容网关 {
    fn new(audit: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            uploads: Mutex::new(BTreeMap::new()),
            upload_calls: AtomicUsize::new(0),
            bind_calls: AtomicUsize::new(0),
            fail_next_bind: AtomicBool::new(false),
            redacted: Mutex::new(Vec::new()),
            audit,
        }
    }
}

impl MessageContentGateway for 内存内容网关 {
    fn upload<'a>(
        &'a self,
        request: &'a MessageContentUploadRequest,
    ) -> PortFuture<'a, Result<MessageContentRecord, MessageContentFailure>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        self.audit.lock().expect("审计锁可用").push("上传正文");
        let key = request.request_id.as_uuid();
        let mut uploads = self.uploads.lock().expect("上传记录锁可用");
        let record = uploads
            .entry(key)
            .or_insert_with(|| MessageContentRecord {
                content_id: ContentId::from_uuid(key),
                digest: request.digest,
                byte_length: request.byte_length,
                media_type: request.media_type.clone(),
            })
            .clone();
        Box::pin(async move { Ok(record) })
    }

    fn bind<'a>(
        &'a self,
        _request: &'a MessageContentBindRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
        self.bind_calls.fetch_add(1, Ordering::SeqCst);
        self.audit.lock().expect("审计锁可用").push("绑定正文");
        let should_fail = self.fail_next_bind.swap(false, Ordering::SeqCst);
        Box::pin(async move {
            if should_fail {
                Err(MessageContentFailure::new(
                    MessageContentFailureKind::Unavailable,
                ))
            } else {
                Ok(())
            }
        })
    }

    fn redact<'a>(
        &'a self,
        request: &'a MessageContentRedactRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
        self.audit.lock().expect("审计锁可用").push("撤销正文");
        self.redacted
            .lock()
            .expect("撤销记录锁可用")
            .push(request.content_id);
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct 内存提交仓库 {
    fail_next_claim: AtomicBool,
    records: Mutex<BTreeMap<MessageSubmissionId, MessageSubmissionRecord>>,
}

impl MessageSubmissionRepository for 内存提交仓库 {
    fn claim<'a>(
        &'a self,
        claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>> {
        if self.fail_next_claim.swap(false, Ordering::SeqCst) {
            return Box::pin(async {
                Err(MessageStoreFailure::new(
                    MessageStoreFailureKind::Unavailable,
                ))
            });
        }
        let mut records = self.records.lock().expect("提交仓库锁可用");
        let result = if let Some(existing) = records.get(&claim.submission_id) {
            if existing.kind != claim.kind
                || existing.fingerprint != claim.fingerprint
                || existing.transaction_id != claim.transaction_id
            {
                Err(MessageStoreFailure::new(MessageStoreFailureKind::Conflict))
            } else {
                Ok(MessageSubmissionClaimOutcome::Existing(existing.clone()))
            }
        } else {
            let record = MessageSubmissionRecord {
                submission_id: claim.submission_id,
                kind: claim.kind,
                fingerprint: claim.fingerprint,
                transaction_id: claim.transaction_id.clone(),
                state: MessageSubmissionState::Claimed,
                event_id: None,
            };
            records.insert(claim.submission_id, record.clone());
            Ok(MessageSubmissionClaimOutcome::Created(record))
        };
        Box::pin(async move { result })
    }

    fn mark_submit_unknown(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        self.update(submission_id, |record| {
            if record.state == MessageSubmissionState::Claimed {
                record.state = MessageSubmissionState::SubmitUnknown;
            }
            Ok(())
        })
    }

    fn mark_accepted<'a>(
        &'a self,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        let event_id = event_id.clone();
        self.update(submission_id, move |record| {
            if record
                .event_id
                .as_ref()
                .is_some_and(|existing| existing != &event_id)
            {
                return Err(MessageStoreFailure::new(MessageStoreFailureKind::Conflict));
            }
            record.event_id = Some(event_id);
            if record.state != MessageSubmissionState::Bound {
                record.state = MessageSubmissionState::Accepted;
            }
            Ok(())
        })
    }

    fn mark_bound(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        self.update(submission_id, |record| {
            if record.event_id.is_none() {
                return Err(MessageStoreFailure::new(MessageStoreFailureKind::Corrupt));
            }
            record.state = MessageSubmissionState::Bound;
            Ok(())
        })
    }

    fn observe_transaction<'a>(
        &'a self,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        let mut records = self.records.lock().expect("提交仓库锁可用");
        let Some(record) = records
            .values_mut()
            .find(|record| &record.transaction_id == transaction_id)
        else {
            return Box::pin(async { Ok(None) });
        };
        if record
            .event_id
            .as_ref()
            .is_some_and(|existing| existing != event_id)
        {
            return Box::pin(async {
                Err(MessageStoreFailure::new(MessageStoreFailureKind::Conflict))
            });
        }
        record.event_id = Some(event_id.clone());
        if record.state != MessageSubmissionState::Bound {
            record.state = MessageSubmissionState::Accepted;
        }
        let observed = record.clone();
        Box::pin(async move { Ok(Some(observed)) })
    }
}

impl 内存提交仓库 {
    fn update<'a>(
        &'a self,
        submission_id: MessageSubmissionId,
        transition: impl FnOnce(&mut MessageSubmissionRecord) -> Result<(), MessageStoreFailure>
        + Send
        + 'a,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        let mut records = self.records.lock().expect("提交仓库锁可用");
        let result = records
            .get_mut(&submission_id)
            .ok_or_else(|| MessageStoreFailure::new(MessageStoreFailureKind::NotFound))
            .and_then(|record| {
                transition(record)?;
                Ok(record.clone())
            });
        Box::pin(async move { result })
    }
}

struct 测试夹具 {
    signer: Arc<测试签名身份>,
    publisher: Arc<记录消息发布器>,
    content: Arc<内存内容网关>,
    submissions: Arc<内存提交仓库>,
    service: MessagePublicationService,
    audit: Arc<Mutex<Vec<&'static str>>>,
}

impl 测试夹具 {
    fn new() -> Self {
        Self::with_automation(Arc::new(允许自动授权))
    }

    fn with_automation(automation: Arc<dyn AutomationAuthorizationGateway>) -> Self {
        let audit = Arc::new(Mutex::new(Vec::new()));
        let signer = Arc::new(测试签名身份::generate());
        let publisher = Arc::new(记录消息发布器::new(Arc::clone(&audit)));
        let content = Arc::new(内存内容网关::new(Arc::clone(&audit)));
        let submissions = Arc::new(内存提交仓库::default());
        let service = MessagePublicationService::new(MessagePublicationDependencies {
            identity: identity(),
            signer: signer.clone(),
            publisher: publisher.clone(),
            content: content.clone(),
            submissions: submissions.clone(),
            automation,
            room_catalog_id: room_catalog_id(),
        });
        Self {
            signer,
            publisher,
            content,
            submissions,
            service,
            audit,
        }
    }
}

#[tokio::test]
async fn 自动授权拒绝发生在本地认领正文上传与_matrix_发布之前() {
    let fixture = 测试夹具::with_automation(Arc::new(拒绝自动授权));
    let request = send_request(
        MessageSubmissionId::from_uuid(Uuid::now_v7()),
        "不会发送",
        None,
    );

    let failure = fixture
        .service
        .send(&request)
        .await
        .expect_err("频率耗尽必须拒绝发送");

    assert_eq!(
        failure.kind(),
        MessagePublicationFailureKind::AutomationAuthorization
    );
    assert_eq!(
        failure
            .automation_failure()
            .expect("保留自动授权失败")
            .denial(),
        Some(AutomationAuthorizationDenial::RateLimitExceeded)
    );
    assert!(fixture.audit.lock().expect("审计锁可用").is_empty());
    assert!(fixture.publisher.events().is_empty());
    assert!(
        fixture
            .submissions
            .records
            .lock()
            .expect("提交仓库锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 本地磁盘暂不可用时发送明确失败且恢复后只产生一个事件() {
    let fixture = 测试夹具::new();
    fixture
        .submissions
        .fail_next_claim
        .store(true, Ordering::SeqCst);
    let request = send_request(
        MessageSubmissionId::from_uuid(Uuid::now_v7()),
        "磁盘恢复",
        None,
    );

    let failure = fixture
        .service
        .send(&request)
        .await
        .expect_err("存储不可用不能伪装成已发送");
    assert_eq!(failure.kind(), MessagePublicationFailureKind::Store);
    assert_eq!(
        failure.store_failure().expect("包含存储错误").kind(),
        MessageStoreFailureKind::Unavailable
    );
    assert!(fixture.publisher.events().is_empty());
    assert_eq!(fixture.content.upload_calls.load(Ordering::SeqCst), 0);

    let recovered = fixture
        .service
        .send(&request)
        .await
        .expect("磁盘恢复后可发送");
    assert!(matches!(
        recovered,
        MessagePublicationOutcome::Published { reused: false, .. }
    ));
    assert_eq!(fixture.publisher.events().len(), 1);
}

#[tokio::test]
async fn 未知提交通过事务对账收敛且重试不制造可见重复() {
    let fixture = 测试夹具::new();
    fixture.publisher.enqueue(发布行为::接受但响应未知);
    let submission_id = MessageSubmissionId::from_uuid(Uuid::now_v7());
    let request = send_request(submission_id, "第一版摘要", None);

    let first = fixture
        .service
        .send(&request)
        .await
        .expect("未知提交被显式返回");
    let MessagePublicationOutcome::PendingReconciliation { transaction_id, .. } = first else {
        panic!("响应丢失必须进入对账状态");
    };
    assert_eq!(fixture.publisher.events().len(), 1);

    let event_id = fixture.publisher.accepted_event_id(&transaction_id);
    fixture
        .submissions
        .observe_transaction(&transaction_id, &event_id)
        .await
        .expect("同步观察可写入")
        .expect("事务可对账");
    let reconciled = fixture
        .service
        .send(&request)
        .await
        .expect("对账后仅补绑定");
    assert_eq!(
        reconciled,
        MessagePublicationOutcome::Published {
            submission_id,
            event_id: event_id.clone(),
            reused: true,
        }
    );
    let repeated = fixture
        .service
        .send(&request)
        .await
        .expect("完成后重复调用幂等");
    assert_eq!(
        repeated,
        MessagePublicationOutcome::Published {
            submission_id,
            event_id,
            reused: true,
        }
    );
    assert_eq!(fixture.publisher.events().len(), 1);
    assert_eq!(fixture.content.upload_calls.load(Ordering::SeqCst), 2);

    let conflict = send_request(submission_id, "不同意图", None);
    let failure = fixture
        .service
        .send(&conflict)
        .await
        .expect_err("相同幂等键不能承载不同意图");
    assert_eq!(failure.kind(), MessagePublicationFailureKind::Store);
    assert_eq!(
        failure.store_failure().expect("包含存储错误").kind(),
        MessageStoreFailureKind::Conflict
    );
}

#[tokio::test]
async fn 事件已接受但绑定失败时重试只补绑定() {
    let fixture = 测试夹具::new();
    fixture.content.fail_next_bind.store(true, Ordering::SeqCst);
    let submission_id = MessageSubmissionId::from_uuid(Uuid::now_v7());
    let request = send_request(submission_id, "绑定恢复", None);

    let first = fixture.service.send(&request).await.expect("事件发布成功");
    assert!(matches!(
        first,
        MessagePublicationOutcome::AcceptedBindingPending { .. }
    ));
    let second = fixture.service.send(&request).await.expect("重试补齐绑定");
    assert!(matches!(
        second,
        MessagePublicationOutcome::Published { reused: true, .. }
    ));
    assert_eq!(fixture.publisher.events().len(), 1);
    assert_eq!(fixture.content.bind_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn 回复编辑撤回都符合协议且撤回先切断正文访问() {
    let fixture = 测试夹具::new();
    let reply_target = MessageId::from_uuid(Uuid::now_v7());
    let message_id = MessageSubmissionId::from_uuid(Uuid::now_v7());
    let sent = send_request(
        message_id,
        "带回复关系",
        Some(MessageRelation::ReplyTo(reply_target)),
    );
    fixture.service.send(&sent).await.expect("消息可发布");

    let edit_id = MessageSubmissionId::from_uuid(Uuid::now_v7());
    let edit = EditMessageRequest::new(
        edit_id,
        room_id(),
        MessageId::from_uuid(message_id.as_uuid()),
        preview("替换后的摘要"),
        body("替换后的正文"),
        MessageProvenance::HumanConfirmedAgent,
    )
    .expect("编辑请求有效");
    fixture.service.edit(&edit).await.expect("编辑可发布");

    let events_before_redact = fixture.publisher.events();
    let content_id = ContentId::from_uuid(
        Uuid::parse_str(
            events_before_redact[1].1.content()["content"]["contentId"]
                .as_str()
                .expect("内容标识存在"),
        )
        .expect("内容标识有效"),
    );
    let redact_id = MessageSubmissionId::from_uuid(Uuid::now_v7());
    let redact = RedactMessageRequest::new(
        redact_id,
        room_id(),
        MessageId::from_uuid(message_id.as_uuid()),
        content_id,
        MessageProvenance::Human,
    )
    .expect("撤回请求有效");
    fixture.service.redact(&redact).await.expect("撤回可发布");

    let events = fixture.publisher.events();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0].1.content()["relation"]["targetMessageId"],
        reply_target.to_string()
    );
    assert_eq!(events[1].1.content()["kind"], "replace");
    assert_eq!(events[2].1.content()["kind"], "redact");
    assert!(events[2].1.content().get("content").is_none());
    for (_, event, _) in &events {
        assert_protocol_event(event.content());
        assert_valid_signature(fixture.signer.as_ref(), event.content());
    }

    let audit = fixture.audit.lock().expect("审计锁可用");
    let redact_position = audit
        .iter()
        .rposition(|operation| *operation == "撤销正文")
        .expect("记录正文撤销");
    let publish_position = audit
        .iter()
        .rposition(|operation| *operation == "发布事件")
        .expect("记录修订发布");
    assert!(
        redact_position < publish_position,
        "必须先撤权再发布撤回事件"
    );
}

fn send_request(
    submission_id: MessageSubmissionId,
    summary: &str,
    relation: Option<MessageRelation>,
) -> SendMessageRequest {
    SendMessageRequest::new(
        submission_id,
        room_id(),
        preview(summary),
        body("正文只按需读取"),
        MessageProvenance::AutonomousAgent,
        relation,
        Some(automation_grant_id()),
    )
    .expect("发送请求有效")
}

fn automation_grant_id() -> AutomationGrantId {
    AutomationGrantId::from_uuid(
        Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e47").expect("测试授权标识有效"),
    )
}

fn room_catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(
        Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e46").expect("测试房间目录标识有效"),
    )
}

fn preview(summary: &str) -> MessagePreview {
    MessagePreview::new(
        MessageTitle::new("工作状态更新").expect("标题有效"),
        MessageSummary::new(summary).expect("摘要有效"),
        ContentMediaType::new("text/markdown").expect("媒体类型有效"),
        Some(MessageLanguage::new("zh-CN").expect("语言有效")),
        MessageSensitivity::Normal,
        MessageRiskFlags::new([
            MessageRiskFlag::new("untrusted_instructions").expect("风险标签有效")
        ])
        .expect("风险标签集合有效"),
    )
}

fn body(value: &str) -> MessageBody {
    MessageBody::new(
        value.as_bytes().to_vec(),
        ContentMediaType::new("text/markdown").expect("媒体类型有效"),
        ContentEncryptionMode::ServerSide,
        None,
    )
    .expect("消息正文有效")
}

fn identity() -> BridgeAgentIdentity {
    BridgeAgentIdentity::new(
        AgentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"),
        ),
        "构建 Agent",
        "@agent:matrix.test",
        AgentInstanceId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a4").expect("UUID 有效"),
        ),
    )
    .expect("身份有效")
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}

fn assert_protocol_event(content: &Value) {
    let schema: Value = serde_json::from_str(MESSAGE_SCHEMA).expect("协议 Schema 有效");
    let validator = jsonschema::validator_for(&schema).expect("协议 Schema 可编译");
    let errors = validator
        .iter_errors(content)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "消息事件违反协议：{errors:?}");
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
