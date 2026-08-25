use std::sync::{Arc, Mutex};

use agent_room_application::ports::{MatrixEventId, MatrixRoomId, PortFuture};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    handoffs::{HandoffContentFailureKind, HandoffContentGateway, ProjectedHandoffContentGateway},
    messages::{
        DownloadedMessageContent, MessageContentReadFailure, MessageContentReadGateway,
        MessageContentReadRequest, MessageContentSourceQuery, MessagePreviewPage,
        MessagePreviewQuery, MessageTimelineQueryFailure, MessageTimelineQueryRepository,
        ProjectedMessageActor, ProjectedMessagePreview,
    },
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffPermission,
        HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
        HandoffSourceEventId,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    messages::{
        MessageContentReference, MessagePreview, MessageProvenance, MessageRiskFlag,
        MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
    },
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

#[tokio::test]
async fn 交接正文只从完全匹配签名来源的本地投影下载() {
    let body = Arc::<[u8]>::from("已验证的一次性交接正文".as_bytes());
    let source = source(body.as_ref(), "text/markdown");
    let handoff = handoff(&source, [HandoffPermission::ReadText]);
    let content = Arc::new(固定正文网关 {
        opened: downloaded(body.clone(), "text/markdown"),
        requests: Mutex::new(Vec::new()),
    });
    let gateway = ProjectedHandoffContentGateway::new(
        Arc::new(固定来源仓储(Some(source.clone()))),
        content.clone(),
    );

    let opened = gateway.read(&handoff).await.expect("精确来源允许读取");

    assert_eq!(opened.body, body);
    assert_eq!(opened.media_type.as_str(), "text/markdown");
    let requests = content.requests.lock().expect("正文请求锁可用");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].content_id(), source.content.content_id());
    assert_eq!(requests[0].maximum_bytes(), source.content.size_bytes());
}

#[tokio::test]
async fn 来源事件或权限范围不匹配时在下载前拒绝() {
    let body = Arc::<[u8]>::from("不可越权读取".as_bytes());
    let source = source(body.as_ref(), "text/plain");
    let mut mismatched = source.clone();
    mismatched.event_id = MatrixEventId::new("$other:matrix.test").expect("事件标识有效");
    let content = Arc::new(固定正文网关 {
        opened: downloaded(body, "text/plain"),
        requests: Mutex::new(Vec::new()),
    });
    let gateway = ProjectedHandoffContentGateway::new(
        Arc::new(固定来源仓储(Some(mismatched))),
        content.clone(),
    );
    let failure = gateway
        .read(&handoff(&source, [HandoffPermission::ReadText]))
        .await
        .expect_err("来源换绑必须拒绝");
    assert_eq!(failure.kind(), HandoffContentFailureKind::Denied);
    assert!(content.requests.lock().expect("正文请求锁可用").is_empty());

    let gateway = ProjectedHandoffContentGateway::new(
        Arc::new(固定来源仓储(Some(source.clone()))),
        content.clone(),
    );
    let failure = gateway
        .read(&handoff(&source, [HandoffPermission::ReadAttachments]))
        .await
        .expect_err("文本交接未授予文本读取权限");
    assert_eq!(failure.kind(), HandoffContentFailureKind::Denied);
    assert!(content.requests.lock().expect("正文请求锁可用").is_empty());
}

#[tokio::test]
async fn 控制面返回的正文元数据再次逐项校验() {
    let body = Arc::<[u8]>::from("原始正文".as_bytes());
    let source = source(body.as_ref(), "text/plain");
    let tampered = Arc::<[u8]>::from("篡改正文".as_bytes());
    let gateway = ProjectedHandoffContentGateway::new(
        Arc::new(固定来源仓储(Some(source.clone()))),
        Arc::new(固定正文网关 {
            opened: downloaded(tampered, "text/plain"),
            requests: Mutex::new(Vec::new()),
        }),
    );

    let failure = gateway
        .read(&handoff(&source, [HandoffPermission::ReadText]))
        .await
        .expect_err("摘要或长度不一致必须拒绝");
    assert_eq!(failure.kind(), HandoffContentFailureKind::InvalidResponse);
}

struct 固定来源仓储(Option<ProjectedMessagePreview>);

impl MessageTimelineQueryRepository for 固定来源仓储 {
    fn list_previews<'a>(
        &'a self,
        _query: &'a MessagePreviewQuery,
    ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>> {
        Box::pin(async { Ok(MessagePreviewPage::new(Vec::new(), None)) })
    }

    fn find_content_source<'a>(
        &'a self,
        _query: &'a MessageContentSourceQuery,
    ) -> PortFuture<'a, Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure>> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

struct 固定正文网关 {
    opened: DownloadedMessageContent,
    requests: Mutex<Vec<MessageContentReadRequest>>,
}

impl MessageContentReadGateway for 固定正文网关 {
    fn open<'a>(
        &'a self,
        request: &'a MessageContentReadRequest,
    ) -> PortFuture<'a, Result<DownloadedMessageContent, MessageContentReadFailure>> {
        self.requests
            .lock()
            .expect("正文请求锁可用")
            .push(request.clone());
        let opened = self.opened.clone();
        Box::pin(async move { Ok(opened) })
    }
}

fn source(body: &[u8], media_type: &str) -> ProjectedMessagePreview {
    let identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(Uuid::now_v7()),
        "来源 Agent",
        "@source:matrix.test",
        AgentInstanceId::from_uuid(Uuid::now_v7()),
    )
    .expect("来源身份有效");
    let digest = Sha256Digest::from_bytes(Sha256::digest(body).into());
    ProjectedMessagePreview {
        event_id: MatrixEventId::new("$source:matrix.test").expect("事件标识有效"),
        transaction_id: None,
        room_id: room_id(),
        message_id: MessageId::from_uuid(Uuid::now_v7()),
        created_at: time(1_000),
        origin_server_timestamp: Some(1_000),
        actor: ProjectedMessageActor::new(identity, MessageProvenance::AutonomousAgent),
        preview: MessagePreview::new(
            MessageTitle::new("远端消息").expect("标题有效"),
            MessageSummary::new("交给本地 Agent").expect("摘要有效"),
            ContentMediaType::new(media_type).expect("媒体类型有效"),
            None,
            MessageSensitivity::Normal,
            MessageRiskFlags::new([
                MessageRiskFlag::new("untrusted_instructions").expect("风险标签有效")
            ])
            .expect("风险集合有效"),
        ),
        content: MessageContentReference::new(
            ContentId::from_uuid(Uuid::now_v7()),
            digest,
            u64::try_from(body.len()).expect("正文长度有效"),
        )
        .expect("正文引用有效"),
        relation: None,
    }
}

fn handoff(
    source: &ProjectedMessagePreview,
    permissions: impl IntoIterator<Item = HandoffPermission>,
) -> ContextHandoff {
    let actor = source.actor.identity();
    let mut handoff = ContextHandoff::propose(ContextHandoffFields {
        id: HandoffId::from_uuid(Uuid::now_v7()),
        requester_agent_id: AgentId::from_uuid(Uuid::now_v7()),
        requester_instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        source: HandoffSource::new(
            MatrixRoomReference::new(source.room_id.as_str()).expect("房间引用有效"),
            HandoffSourceEventId::new(source.event_id.as_str()).expect("来源事件有效"),
            source.message_id,
            HandoffSourceActor::new(
                actor.agent_id(),
                actor.agent_instance_id(),
                source.actor.provenance(),
            ),
        ),
        target_agent_id: AgentId::from_uuid(Uuid::now_v7()),
        target_instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        content: HandoffContentReference::new(
            source.content.content_id(),
            source.content.digest(),
            ContentByteLength::new(source.content.size_bytes()).expect("正文长度有效"),
            source.preview.content_type().clone(),
        ),
        permissions: HandoffPermissions::new(permissions).expect("交接权限有效"),
        purpose: HandoffPurpose::Summarize,
        risk_flags: source.preview.risk_flags().clone(),
        proposed_at: time(1_000),
        expires_at: time(2_000),
    })
    .expect("交接提案有效");
    handoff
        .approve(PrincipalId::from_uuid(Uuid::now_v7()), time(1_100))
        .expect("交接批准有效");
    handoff
}

fn downloaded(body: Arc<[u8]>, media_type: &str) -> DownloadedMessageContent {
    DownloadedMessageContent {
        digest: Sha256Digest::from_bytes(Sha256::digest(&body).into()),
        byte_length: ContentByteLength::new(u64::try_from(body.len()).expect("正文长度有效"))
            .expect("正文长度有效"),
        media_type: ContentMediaType::new(media_type).expect("媒体类型有效"),
        bytes: body,
    }
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
