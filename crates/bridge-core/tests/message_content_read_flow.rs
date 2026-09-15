use std::sync::{Arc, Mutex};

use agent_room_application::ports::{MatrixEventId, MatrixRoomId, PortFuture};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        DownloadedMessageContent, MessageContentReadFailure, MessageContentReadGateway,
        MessageContentReadRequest, MessageContentSourceQuery, MessagePreviewPage,
        MessagePreviewQuery, MessageTimelineQueryFailure, MessageTimelineQueryRepository,
        OpenMessageContentDependencies, OpenMessageContentFailureKind, OpenMessageContentRequest,
        OpenMessageContentService, OpenedMessageBody, ProjectedMessageActor,
        ProjectedMessagePreview,
    },
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    ids::{AgentId, AgentInstanceId, ContentId, MessageId},
    messages::{
        ConversationMessage, MessageContentReference, MessagePreview, MessageProvenance,
        MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
    },
    time::UtcMillis,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

#[tokio::test]
async fn 正文只在本地验签来源存在后下载并逐项校验完整性() {
    let body = Arc::<[u8]>::from("经过校验的远端正文".as_bytes());
    let source = source(body.as_ref(), "text/markdown");
    let gateway = Arc::new(固定正文网关 {
        opened: downloaded(body.clone(), "text/markdown"),
        requests: Mutex::new(Vec::new()),
    });
    let service = OpenMessageContentService::new(OpenMessageContentDependencies {
        projections: Arc::new(固定来源仓储(Some(source.clone()))),
        content: gateway.clone(),
        cryptography: None,
    });

    let opened = service
        .open(&OpenMessageContentRequest::new(
            room_id(),
            source.content.content_id(),
        ))
        .await
        .expect("完整性一致的文本正文可打开");

    assert_eq!(opened.source(), &source);
    assert_eq!(
        opened.body(),
        &OpenedMessageBody::Text("经过校验的远端正文".into())
    );
    let requests = gateway.requests.lock().expect("请求记录锁可用");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].content_id(), source.content.content_id());
    assert_eq!(requests[0].maximum_bytes(), source.content.size_bytes());
}

#[tokio::test]
async fn 摘要不一致和缺失来源都不能把远端字节交给宿主() {
    let expected = Arc::<[u8]>::from("可信摘要对应正文".as_bytes());
    let source = source(expected.as_ref(), "text/plain");
    let tampered = Arc::<[u8]>::from("被篡改的远端正文".as_bytes());
    let gateway = Arc::new(固定正文网关 {
        opened: downloaded(tampered, "text/plain"),
        requests: Mutex::new(Vec::new()),
    });
    let service = OpenMessageContentService::new(OpenMessageContentDependencies {
        projections: Arc::new(固定来源仓储(Some(source.clone()))),
        content: gateway.clone(),
        cryptography: None,
    });

    let failure = service
        .open(&OpenMessageContentRequest::new(
            room_id(),
            source.content.content_id(),
        ))
        .await
        .expect_err("篡改正文必须拒绝");
    assert_eq!(
        failure.kind(),
        OpenMessageContentFailureKind::IntegrityMismatch
    );

    let missing_gateway = Arc::new(固定正文网关 {
        opened: downloaded(expected, "text/plain"),
        requests: Mutex::new(Vec::new()),
    });
    let missing_service = OpenMessageContentService::new(OpenMessageContentDependencies {
        projections: Arc::new(固定来源仓储(None)),
        content: missing_gateway.clone(),
        cryptography: None,
    });
    let failure = missing_service
        .open(&OpenMessageContentRequest::new(
            room_id(),
            source.content.content_id(),
        ))
        .await
        .expect_err("没有本地来源时不得下载");
    assert_eq!(failure.kind(), OpenMessageContentFailureKind::NotFound);
    assert!(
        missing_gateway
            .requests
            .lock()
            .expect("请求记录锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 图片附件可读取超过文本上限的原始字节且篡改仍被拒绝() {
    let body = Arc::<[u8]>::from(vec![0xff; 100 * 1024]);
    let mut expected = source(&body, "image/png");
    expected.preview = expected.preview.with_conversation(
        ConversationMessage::new("请查看附件".into(), Vec::new())
            .expect("对话有效")
            .with_attachment_name(Some("设计.png".into()))
            .expect("附件名有效"),
    );
    for tampered in [false, true] {
        let bytes = if tampered {
            Arc::from(vec![0x00; body.len()])
        } else {
            body.clone()
        };
        let gateway = Arc::new(固定正文网关 {
            opened: downloaded(bytes, "image/png"),
            requests: Mutex::new(Vec::new()),
        });
        let service = OpenMessageContentService::new(OpenMessageContentDependencies {
            projections: Arc::new(固定来源仓储(Some(expected.clone()))),
            content: gateway,
            cryptography: None,
        });
        let result = service
            .open(&OpenMessageContentRequest::new(
                room_id(),
                expected.content.content_id(),
            ))
            .await;
        if tampered {
            assert_eq!(
                result.expect_err("篡改附件必须拒绝").kind(),
                OpenMessageContentFailureKind::IntegrityMismatch
            );
        } else {
            assert_eq!(
                result.expect("附件可读取").body(),
                &OpenedMessageBody::Attachment {
                    name: "设计.png".into(),
                    bytes: body.clone()
                }
            );
        }
    }
}

#[tokio::test]
async fn 超出附件上限或未声明附件的二进制消息不触发下载() {
    for attachment in [false, true] {
        let body = Arc::<[u8]>::from([0xff]);
        let mut expected = source(&body, "image/png");
        if attachment {
            expected.preview = expected.preview.with_conversation(
                ConversationMessage::new("大文件".into(), Vec::new())
                    .expect("对话有效")
                    .with_attachment_name(Some("large.png".into()))
                    .expect("附件名有效"),
            );
            expected.content = MessageContentReference::new(
                expected.content.content_id(),
                expected.content.digest(),
                21 * 1024 * 1024,
            )
            .expect("协议大小有效");
        }
        let gateway = Arc::new(固定正文网关 {
            opened: downloaded(body, "image/png"),
            requests: Mutex::new(Vec::new()),
        });
        let service = OpenMessageContentService::new(OpenMessageContentDependencies {
            projections: Arc::new(固定来源仓储(Some(expected.clone()))),
            content: gateway.clone(),
            cryptography: None,
        });
        let failure = service
            .open(&OpenMessageContentRequest::new(
                room_id(),
                expected.content.content_id(),
            ))
            .await
            .expect_err("不能下载");
        assert_eq!(
            failure.kind(),
            if attachment {
                OpenMessageContentFailureKind::TooLarge
            } else {
                OpenMessageContentFailureKind::UnsupportedMediaType
            }
        );
        assert!(gateway.requests.lock().expect("请求锁").is_empty());
    }
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
            .expect("请求记录锁可用")
            .push(request.clone());
        let opened = self.opened.clone();
        Box::pin(async move { Ok(opened) })
    }
}

fn source(body: &[u8], media_type: &str) -> ProjectedMessagePreview {
    let digest = Sha256Digest::from_bytes(Sha256::digest(body).into());
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    ProjectedMessagePreview {
        event_id: MatrixEventId::new("$message:matrix.test").expect("事件标识有效"),
        transaction_id: None,
        room_id: room_id(),
        message_id: MessageId::from_uuid(Uuid::now_v7()),
        created_at: UtcMillis::new(1_000).expect("时间有效"),
        origin_server_timestamp: Some(1_000),
        actor: ProjectedMessageActor::new(identity(), MessageProvenance::AutonomousAgent),
        preview: MessagePreview::new(
            MessageTitle::new("远端消息").expect("标题有效"),
            MessageSummary::new("按需读取正文").expect("摘要有效"),
            ContentMediaType::new(media_type).expect("媒体类型有效"),
            None,
            MessageSensitivity::Normal,
            MessageRiskFlags::new([]).expect("风险标签有效"),
        ),
        content: MessageContentReference::new(
            content_id,
            digest,
            u64::try_from(body.len()).expect("测试正文长度有效"),
        )
        .expect("正文引用有效"),
        relation: None,
    }
}

fn downloaded(body: Arc<[u8]>, media_type: &str) -> DownloadedMessageContent {
    DownloadedMessageContent {
        digest: Sha256Digest::from_bytes(Sha256::digest(&body).into()),
        byte_length: ContentByteLength::new(u64::try_from(body.len()).expect("测试正文长度有效"))
            .expect("正文长度有效"),
        media_type: ContentMediaType::new(media_type).expect("媒体类型有效"),
        bytes: body,
    }
}

fn identity() -> BridgeAgentIdentity {
    BridgeAgentIdentity::new(
        AgentId::from_uuid(Uuid::now_v7()),
        "远端 Agent",
        "@remote-agent:matrix.test",
        AgentInstanceId::from_uuid(Uuid::now_v7()),
    )
    .expect("身份有效")
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}
