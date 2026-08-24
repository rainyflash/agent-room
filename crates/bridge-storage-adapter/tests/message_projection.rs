use agent_room_application::ports::{
    MatrixBackfillToken, MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixTransactionId,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        MessagePreviewQuery, MessageProjectionBatch, MessageProjectionMutation,
        MessageProjectionStoreFailureKind, MessageSyncIssue, MessageSyncIssueReason,
        MessageTimelineGap, MessageTimelineProjectionStore, MessageTimelineQueryFailureKind,
        MessageTimelineQueryRepository, ProjectedActorInstanceVerification, ProjectedMessageActor,
        ProjectedMessagePreview, ProjectedMessageRevision,
    },
};
use agent_room_bridge_storage_adapter::SqliteMessageTimelineRepository;
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    ids::{AgentId, AgentInstanceId, ContentId, MessageId, MessageRevisionId},
    messages::{
        MessageContentReference, MessageLanguage, MessagePreview, MessageProvenance,
        MessageRevisionKind, MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
    },
    time::UtcMillis,
};
use sqlx::{
    Row as _, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::TempDir;
use uuid::Uuid;

const ROOM_ID: &str = "!lobby:matrix.test";
const OWNER_AGENT_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3";
const OWNER_INSTANCE_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4";
const OTHER_AGENT_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9b3";
const OTHER_INSTANCE_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9b4";

#[tokio::test]
async fn 重复同步不制造序号空洞且客户端时间不能改写到达顺序() {
    let (_temporary, store, inspector) = open_store().await;
    let first_message_id = MessageId::from_uuid(Uuid::now_v7());
    let second_message_id = MessageId::from_uuid(Uuid::now_v7());
    let batch = MessageProjectionBatch::new(
        sync_token("sync-1"),
        vec![
            preview_mutation(
                "$first:matrix.test",
                first_message_id,
                owner_actor().with_instance_verification(
                    ProjectedActorInstanceVerification::RevokedAfterEvent,
                ),
                4_070_908_800_000,
                "先到但客户端时间更晚",
                1,
                Some(9_000_000_000_000),
            ),
            preview_mutation(
                "$second:matrix.test",
                second_message_id,
                owner_actor(),
                946_684_800_000,
                "后到但客户端时间更早",
                2,
                Some(1),
            ),
        ],
        vec![MessageSyncIssue {
            room_id: room_id(),
            event_id: Some(event_id("$isolated:matrix.test")),
            reason: MessageSyncIssueReason::InvalidSignature,
        }],
        vec![MessageTimelineGap {
            room_id: room_id(),
            previous_batch: Some(MatrixBackfillToken::new("backfill-1").expect("回填游标有效")),
        }],
    );

    store.apply(&batch).await.expect("首次投影成功");
    store.apply(&batch).await.expect("重复投影成功");

    let rows = sqlx::query(
        "SELECT message_id, sequence, created_at_unix_ms, actor_json
         FROM message_projection_event
         ORDER BY sequence ASC",
    )
    .fetch_all(&inspector)
    .await
    .expect("事件日志可查询");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].get::<String, _>("message_id"),
        first_message_id.to_string()
    );
    assert_eq!(rows[0].get::<i64, _>("sequence"), 1);
    let actor: serde_json::Value =
        serde_json::from_str(&rows[0].get::<String, _>("actor_json")).expect("Actor 投影是 JSON");
    assert_eq!(actor["instanceVerification"], "revoked_after_event");
    assert_eq!(
        rows[0].get::<i64, _>("created_at_unix_ms"),
        4_070_908_800_000
    );
    assert_eq!(
        rows[1].get::<String, _>("message_id"),
        second_message_id.to_string()
    );
    assert_eq!(rows[1].get::<i64, _>("sequence"), 2);
    assert_eq!(
        scalar_count(&inspector, "SELECT COUNT(*) FROM message_sync_issue").await,
        1
    );
    assert_eq!(
        scalar_count(&inspector, "SELECT COUNT(*) FROM message_timeline_gap").await,
        1
    );
    assert_eq!(current_cursor(&inspector).await.as_deref(), Some("sync-1"));
}

#[tokio::test]
async fn 先到修订会在原消息补齐后生效但冒名修订无权覆盖() {
    let (_temporary, store, inspector) = open_store().await;
    let message_id = MessageId::from_uuid(Uuid::now_v7());
    let revisions = MessageProjectionBatch::new(
        sync_token("sync-revisions"),
        vec![
            replacement_mutation(
                "$owner-revision:matrix.test",
                message_id,
                owner_actor(),
                "合法修订",
                3,
            ),
            replacement_mutation(
                "$forged-revision:matrix.test",
                message_id,
                other_actor(),
                "冒名修订",
                4,
            ),
        ],
        Vec::new(),
        Vec::new(),
    );
    store.apply(&revisions).await.expect("乱序修订可暂存");

    let base = MessageProjectionBatch::new(
        sync_token("sync-base"),
        vec![preview_mutation(
            "$base:matrix.test",
            message_id,
            owner_actor(),
            1_700_000_000_000,
            "初始摘要",
            1,
            Some(50),
        )],
        Vec::new(),
        Vec::new(),
    );
    store.apply(&base).await.expect("原消息可补齐");

    let row = current_message(&inspector, message_id).await;
    assert_eq!(row.get::<String, _>("summary"), "合法修订");
    assert_eq!(
        row.get::<String, _>("last_revision_event_id"),
        "$owner-revision:matrix.test"
    );
    assert_eq!(row.get::<i64, _>("first_sequence"), 3);
    assert_eq!(row.get::<i64, _>("last_sequence"), 3);
    assert_eq!(
        scalar_count(&inspector, "SELECT COUNT(*) FROM message_projection_event").await,
        3
    );
}

#[tokio::test]
async fn 撤回会清空正文引用且后续替换不能复活消息() {
    let (_temporary, store, inspector) = open_store().await;
    let message_id = MessageId::from_uuid(Uuid::now_v7());
    let batch = MessageProjectionBatch::new(
        sync_token("sync-redaction"),
        vec![
            preview_mutation(
                "$base:matrix.test",
                message_id,
                owner_actor(),
                1_700_000_000_000,
                "撤回前摘要",
                5,
                Some(10),
            ),
            redaction_mutation("$redact:matrix.test", message_id, owner_actor()),
            replacement_mutation(
                "$late-replace:matrix.test",
                message_id,
                owner_actor(),
                "不应复活",
                6,
            ),
        ],
        Vec::new(),
        Vec::new(),
    );

    store.apply(&batch).await.expect("撤回批次投影成功");

    let row = current_message(&inspector, message_id).await;
    assert_eq!(row.get::<String, _>("visibility"), "redacted");
    assert_eq!(row.get::<String, _>("summary"), "撤回前摘要");
    assert!(row.get::<Option<String>, _>("content_json").is_none());
    assert_eq!(
        row.get::<String, _>("last_revision_event_id"),
        "$redact:matrix.test"
    );
    assert_eq!(row.get::<i64, _>("last_sequence"), 2);
}

#[tokio::test]
async fn 批次中途编码失败必须回滚事件与同步游标() {
    let (_temporary, store, inspector) = open_store().await;
    let batch = MessageProjectionBatch::new(
        sync_token("must-not-commit"),
        vec![
            preview_mutation(
                "$valid:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_700_000_000_000,
                "本应回滚",
                7,
                Some(10),
            ),
            preview_mutation(
                "$overflow:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_700_000_000_001,
                "非法服务端时间",
                8,
                Some(u64::MAX),
            ),
        ],
        Vec::new(),
        Vec::new(),
    );

    let failure = store.apply(&batch).await.expect_err("溢出必须拒绝整个批次");
    assert_eq!(failure.kind(), MessageProjectionStoreFailureKind::Corrupt);
    assert_eq!(
        scalar_count(&inspector, "SELECT COUNT(*) FROM message_projection_event").await,
        0
    );
    assert_eq!(
        scalar_count(
            &inspector,
            "SELECT COUNT(*) FROM message_current_projection"
        )
        .await,
        0
    );
    assert_eq!(current_cursor(&inspector).await, None);
}

#[tokio::test]
async fn 预览查询按本地到达顺序分页且不读取正文() {
    let (_temporary, store, _inspector) = open_store().await;
    let batch = MessageProjectionBatch::new(
        sync_token("sync-page"),
        vec![
            preview_mutation(
                "$page-first:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_700_000_000_003,
                "第一条",
                1,
                Some(30),
            ),
            preview_mutation(
                "$page-second:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_700_000_000_002,
                "第二条",
                2,
                Some(20),
            ),
            preview_mutation(
                "$page-third:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_700_000_000_001,
                "第三条",
                3,
                Some(10),
            ),
        ],
        Vec::new(),
        Vec::new(),
    );
    store.apply(&batch).await.expect("预览批次可投影");

    let first_page = store
        .list_previews(&MessagePreviewQuery::new(room_id(), None, 2).expect("查询有效"))
        .await
        .expect("首页可读取");
    assert_eq!(
        first_page
            .previews()
            .iter()
            .map(|preview| preview.preview.summary().as_str())
            .collect::<Vec<_>>(),
        ["第三条", "第二条"]
    );
    assert_eq!(
        first_page.next_cursor().expect("仍有下一页").as_str(),
        "$page-second:matrix.test"
    );

    let second_page = store
        .list_previews(
            &MessagePreviewQuery::new(room_id(), first_page.next_cursor().cloned(), 2)
                .expect("查询有效"),
        )
        .await
        .expect("次页可读取");
    assert_eq!(second_page.previews().len(), 1);
    assert_eq!(
        second_page.previews()[0].preview.summary().as_str(),
        "第一条"
    );
    assert!(second_page.next_cursor().is_none());
}

#[tokio::test]
async fn 预览查询拒绝不存在的游标() {
    let (_temporary, store, _inspector) = open_store().await;
    let failure = store
        .list_previews(
            &MessagePreviewQuery::new(room_id(), Some(event_id("$missing:matrix.test")), 20)
                .expect("查询有效"),
        )
        .await
        .expect_err("不存在的游标不能伪装成空页");

    assert_eq!(
        failure.kind(),
        MessageTimelineQueryFailureKind::CursorNotFound
    );
}

async fn open_store() -> (TempDir, SqliteMessageTimelineRepository, SqlitePool) {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let store = SqliteMessageTimelineRepository::open(&path)
        .await
        .expect("投影数据库可打开");
    let inspector = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .expect("只读检查连接可打开");
    (temporary, store, inspector)
}

#[allow(clippy::too_many_arguments)]
fn preview_mutation(
    event: &str,
    message_id: MessageId,
    actor: ProjectedMessageActor,
    created_at: i64,
    summary: &str,
    digest: u8,
    origin_server_timestamp: Option<u64>,
) -> MessageProjectionMutation {
    MessageProjectionMutation::Preview(ProjectedMessagePreview {
        event_id: event_id(event),
        transaction_id: Some(
            MatrixTransactionId::new(format!("transaction-{digest}")).expect("事务标识有效"),
        ),
        room_id: room_id(),
        message_id,
        created_at: UtcMillis::new(created_at).expect("时间有效"),
        origin_server_timestamp,
        actor,
        preview: preview(summary),
        content: content(digest),
        relation: None,
    })
}

fn replacement_mutation(
    event: &str,
    message_id: MessageId,
    actor: ProjectedMessageActor,
    summary: &str,
    digest: u8,
) -> MessageProjectionMutation {
    MessageProjectionMutation::Revision(ProjectedMessageRevision {
        event_id: event_id(event),
        transaction_id: None,
        room_id: room_id(),
        revision_id: MessageRevisionId::from_uuid(Uuid::now_v7()),
        target_message_id: message_id,
        created_at: UtcMillis::new(1_700_000_000_100).expect("时间有效"),
        origin_server_timestamp: Some(20),
        actor,
        kind: MessageRevisionKind::Replace,
        preview: Some(preview(summary)),
        content: Some(content(digest)),
    })
}

fn redaction_mutation(
    event: &str,
    message_id: MessageId,
    actor: ProjectedMessageActor,
) -> MessageProjectionMutation {
    MessageProjectionMutation::Revision(ProjectedMessageRevision {
        event_id: event_id(event),
        transaction_id: None,
        room_id: room_id(),
        revision_id: MessageRevisionId::from_uuid(Uuid::now_v7()),
        target_message_id: message_id,
        created_at: UtcMillis::new(1_700_000_000_200).expect("时间有效"),
        origin_server_timestamp: Some(30),
        actor,
        kind: MessageRevisionKind::Redact,
        preview: None,
        content: None,
    })
}

fn preview(summary: &str) -> MessagePreview {
    MessagePreview::new(
        MessageTitle::new("投影测试").expect("标题有效"),
        MessageSummary::new(summary).expect("摘要有效"),
        ContentMediaType::new("text/markdown").expect("媒体类型有效"),
        Some(MessageLanguage::new("zh-CN").expect("语言有效")),
        MessageSensitivity::Normal,
        MessageRiskFlags::new(Vec::new()).expect("空风险集合有效"),
    )
}

fn content(digest: u8) -> MessageContentReference {
    MessageContentReference::new(
        ContentId::from_uuid(Uuid::now_v7()),
        Sha256Digest::from_bytes([digest; 32]),
        128,
    )
    .expect("正文引用有效")
}

fn owner_actor() -> ProjectedMessageActor {
    actor(OWNER_AGENT_ID, OWNER_INSTANCE_ID, "消息所有者")
}

fn other_actor() -> ProjectedMessageActor {
    actor(OTHER_AGENT_ID, OTHER_INSTANCE_ID, "冒名 Agent")
}

fn actor(agent_id: &str, instance_id: &str, display_name: &str) -> ProjectedMessageActor {
    let identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(Uuid::parse_str(agent_id).expect("Agent 标识有效")),
        display_name,
        format!(
            "@{}:matrix.test",
            display_name.replace(' ', "_").to_lowercase()
        ),
        AgentInstanceId::from_uuid(Uuid::parse_str(instance_id).expect("实例标识有效")),
    )
    .expect("投影身份有效");
    ProjectedMessageActor::new(identity, MessageProvenance::AutonomousAgent)
}

fn room_id() -> MatrixRoomId {
    MatrixRoomId::new(ROOM_ID).expect("房间标识有效")
}

fn event_id(value: &str) -> MatrixEventId {
    MatrixEventId::new(value).expect("事件标识有效")
}

fn sync_token(value: &str) -> MatrixSyncToken {
    MatrixSyncToken::new(value).expect("同步游标有效")
}

async fn scalar_count(inspector: &SqlitePool, statement: &'static str) -> i64 {
    sqlx::query_scalar(statement)
        .fetch_one(inspector)
        .await
        .expect("计数可查询")
}

async fn current_cursor(inspector: &SqlitePool) -> Option<String> {
    sqlx::query_scalar("SELECT next_batch FROM message_sync_state WHERE singleton = 1")
        .fetch_optional(inspector)
        .await
        .expect("同步游标可查询")
}

async fn current_message(inspector: &SqlitePool, message_id: MessageId) -> sqlx::sqlite::SqliteRow {
    sqlx::query(
        "SELECT json_extract(preview_json, '$.summary') AS summary,
                content_json, visibility, first_sequence, last_sequence,
                last_revision_event_id
         FROM message_current_projection
         WHERE message_id = ?",
    )
    .bind(message_id.to_string())
    .fetch_one(inspector)
    .await
    .expect("当前消息可查询")
}
