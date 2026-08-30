use agent_room_bridge_core::handoffs::{
    TargetedHandoffInbox, TargetedHandoffInboxFailureKind, TargetedHandoffInboxRecordOutcome,
    TargetedHandoffTarget,
};
use agent_room_bridge_storage_adapter::SqliteTargetedHandoffInbox;
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        HandoffContentReference, HandoffPermission, HandoffPermissions, HandoffPurpose,
        HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::sqlite::SqliteConnectOptions;
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn 收件箱幂等落盘且按精确实例隔离并删除() {
    let fixture = Fixture::new();
    let (temporary, path, inbox) = open_inbox().await;
    let handoff = fixture.handoff([7; 32]);

    let created = inbox.accept(&handoff).await.expect("首次落盘成功");
    assert!(matches!(
        created,
        TargetedHandoffInboxRecordOutcome::Created(_)
    ));
    let existing = inbox.accept(&handoff).await.expect("重复落盘幂等");
    assert!(matches!(
        existing,
        TargetedHandoffInboxRecordOutcome::Existing(_)
    ));

    assert_eq!(
        inbox.list(fixture.target(), 10).await.expect("列表可读"),
        vec![handoff.clone()]
    );
    let foreign_target = TargetedHandoffTarget {
        agent_id: fixture.target_agent,
        instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
    };
    assert!(
        inbox
            .find(foreign_target, fixture.handoff_id)
            .await
            .expect("外部实例查询正常")
            .is_none()
    );
    assert!(
        !inbox
            .remove(foreign_target, fixture.handoff_id)
            .await
            .expect("外部实例删除不命中")
    );
    assert!(
        inbox
            .remove(fixture.target(), fixture.handoff_id)
            .await
            .expect("目标实例可删除")
    );
    assert!(
        inbox
            .list(fixture.target(), 10)
            .await
            .expect("删除后列表可读")
            .is_empty()
    );

    inbox.close().await;
    drop(path);
    drop(temporary);
}

#[tokio::test]
async fn 相同标识的不同意图冲突且损坏记录不会被信任() {
    let fixture = Fixture::new();
    let (_temporary, path, inbox) = open_inbox().await;
    inbox
        .accept(&fixture.handoff([7; 32]))
        .await
        .expect("初始交接落盘");

    let conflict = inbox
        .accept(&fixture.handoff([8; 32]))
        .await
        .expect_err("同标识不同摘要必须冲突");
    assert_eq!(conflict.kind(), TargetedHandoffInboxFailureKind::Conflict);

    let options = SqliteConnectOptions::new().filename(&path);
    let external = sqlx::SqlitePool::connect_with(options)
        .await
        .expect("测试数据库可连接");
    sqlx::query(
        "UPDATE targeted_handoff_inbox
            SET permissions_json = '[\"unknown_permission\"]'
          WHERE handoff_id = ?",
    )
    .bind(fixture.handoff_id.to_string())
    .execute(&external)
    .await
    .expect("测试可注入领域损坏记录");
    external.close().await;

    let corrupt = inbox
        .find(fixture.target(), fixture.handoff_id)
        .await
        .expect_err("损坏记录必须拒绝");
    assert_eq!(corrupt.kind(), TargetedHandoffInboxFailureKind::Corrupt);
}

struct Fixture {
    handoff_id: HandoffId,
    principal_id: PrincipalId,
    message_id: MessageId,
    content_id: ContentId,
    target_agent: AgentId,
    target_instance: AgentInstanceId,
}

impl Fixture {
    fn new() -> Self {
        Self {
            handoff_id: HandoffId::from_uuid(Uuid::now_v7()),
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            message_id: MessageId::from_uuid(Uuid::now_v7()),
            content_id: ContentId::from_uuid(Uuid::now_v7()),
            target_agent: AgentId::from_uuid(Uuid::now_v7()),
            target_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
        }
    }

    const fn target(&self) -> TargetedHandoffTarget {
        TargetedHandoffTarget {
            agent_id: self.target_agent,
            instance_id: self.target_instance,
        }
    }

    fn handoff(&self, digest: [u8; 32]) -> TargetedHandoff {
        let mut handoff = TargetedHandoff::queue(TargetedHandoffFields {
            id: self.handoff_id,
            principal_id: self.principal_id,
            source_room_id: MatrixRoomReference::new("!lobby:matrix.test").expect("房间引用有效"),
            source_event_id: HandoffSourceEventId::new("$event-123").expect("事件引用有效"),
            source_message_id: self.message_id,
            target_agent_id: self.target_agent,
            target_instance_id: self.target_instance,
            content: HandoffContentReference::new(
                self.content_id,
                Sha256Digest::from_bytes(digest),
                ContentByteLength::new(16).expect("正文长度有效"),
                ContentMediaType::new("text/plain").expect("媒体类型有效"),
            ),
            permissions: HandoffPermissions::new([
                HandoffPermission::ReadText,
                HandoffPermission::IncludeMetadata,
            ])
            .expect("权限有效"),
            purpose: HandoffPurpose::Inspect,
            created_at: time(1_000),
            expires_at: time(5_000),
        })
        .expect("排队交接有效");
        handoff.mark_delivered(time(1_100)).expect("交付有效");
        handoff
    }
}

async fn open_inbox() -> (TempDir, std::path::PathBuf, SqliteTargetedHandoffInbox) {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("handoffs.sqlite3");
    let inbox = SqliteTargetedHandoffInbox::open(&path)
        .await
        .expect("收件箱可打开");
    (temporary, path, inbox)
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
