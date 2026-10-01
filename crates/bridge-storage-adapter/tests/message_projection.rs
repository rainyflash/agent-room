use std::time::Duration;

use agent_room_application::ports::{
    MatrixBackfillToken, MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixTransactionId,
    MatrixUserId,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        MessageBackfillBatch, MessageContentSourceQuery, MessagePreviewQuery,
        MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailureKind,
        MessageRecoveryBatch, MessageSyncIssue, MessageSyncIssueReason, MessageTimelineGap,
        MessageTimelineProjectionStore, MessageTimelineQueryFailureKind,
        MessageTimelineQueryRepository, PendingTimelineGap, ProjectedActorInstanceVerification,
        ProjectedMessageActor, ProjectedMessagePreview, ProjectedMessageRevision,
        ReservedIsolatedEvent, UndecryptableSession,
    },
};
use agent_room_bridge_storage_adapter::{
    MessageProjectionStorageKey, SqliteMessageTimelineRepository,
};
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    ids::{
        AgentId, AgentInstanceId, ContentEncryptionContextId, ContentId, MessageId,
        MessageRevisionId,
    },
    messages::{
        ClientContentEncryption, ClientContentEncryptionAlgorithm, MessageContentReference,
        MessageLanguage, MessagePreview, MessageProvenance, MessageRevisionKind, MessageRiskFlags,
        MessageSensitivity, MessageSummary, MessageTitle,
    },
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sqlx::{
    Connection as _, Row as _, SqliteConnection, SqlitePool,
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
    // 从未同步过时没有游标，Bridge 只能全量同步。
    assert_eq!(store.sync_cursor().await.expect("游标可读"), None);
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
            mutations_before: 2,
            session: None,
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
    // 重启后从这个游标接着同步，而不是重新全量同步只拿最近几十条。
    assert_eq!(
        store
            .sync_cursor()
            .await
            .expect("游标可读")
            .as_ref()
            .map(MatrixSyncToken::as_str),
        Some("sync-1")
    );
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

// 投影和提交仓储共用 messages.sqlite。发送路径正持有写锁时，同步循环写入回显批次
// 必须等锁；否则锁竞争会被当成存储不可用，触发整条 Agent 会话重连。
#[tokio::test]
async fn 提交仓储持有写锁时投影批次会等待而不是报存储不可用() {
    let (temporary, store, inspector) = open_store().await;
    let mut submissions = SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(temporary.path().join("messages.sqlite3")),
    )
    .await
    .expect("提交仓储连接可打开");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut submissions)
        .await
        .expect("提交仓储拿到写锁");
    sqlx::query(
        "INSERT INTO message_submissions
         (submission_id, kind, fingerprint, transaction_id, state, event_id)
         VALUES (?, 'preview', zeroblob(32), 'transaction-7', 'accepted', '$echo:matrix.test')",
    )
    .bind(Uuid::now_v7().to_string())
    .execute(&mut submissions)
    .await
    .expect("发送路径写入已接受");

    let batch = MessageProjectionBatch::new(
        sync_token("sync-echo"),
        vec![preview_mutation(
            "$echo:matrix.test",
            MessageId::from_uuid(Uuid::now_v7()),
            owner_actor(),
            946_684_800_000,
            "自己的回显",
            7,
            Some(1),
        )],
        Vec::new(),
        Vec::new(),
    );
    let sync_loop = store.clone();
    let pending = tokio::spawn(async move { sync_loop.apply(&batch).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !pending.is_finished(),
        "写锁被另一连接占用时投影批次应等待锁释放，而不是立即失败"
    );
    sqlx::query("COMMIT")
        .execute(&mut submissions)
        .await
        .expect("发送路径提交");

    pending
        .await
        .expect("同步任务完成")
        .expect("写锁释放后投影成功");
    assert_eq!(
        scalar_count(
            &inspector,
            "SELECT COUNT(*) FROM message_current_projection"
        )
        .await,
        1
    );
    assert_eq!(
        current_cursor(&inspector).await.as_deref(),
        Some("sync-echo")
    );
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
async fn 按消息_id_取同一个房间里还在的消息_撤回的和找不到的跳过() {
    let (_temporary, store, _inspector) = open_store().await;
    let kept = MessageId::from_uuid(Uuid::now_v7());
    let redacted = MessageId::from_uuid(Uuid::now_v7());
    let unrelated = MessageId::from_uuid(Uuid::now_v7());
    let batch = MessageProjectionBatch::new(
        sync_token("sync-find"),
        vec![
            preview_mutation(
                "$find-kept:matrix.test",
                kept,
                owner_actor(),
                1_700_000_000_001,
                "还在的",
                1,
                Some(10),
            ),
            preview_mutation(
                "$find-redacted:matrix.test",
                redacted,
                owner_actor(),
                1_700_000_000_002,
                "撤回的",
                2,
                Some(20),
            ),
            redaction_mutation("$find-redact:matrix.test", redacted, owner_actor()),
            preview_mutation(
                "$find-unrelated:matrix.test",
                unrelated,
                owner_actor(),
                1_700_000_000_003,
                "没要的",
                3,
                Some(30),
            ),
        ],
        Vec::new(),
        Vec::new(),
    );
    store.apply(&batch).await.expect("批次可投影");

    let missing = MessageId::from_uuid(Uuid::now_v7());
    let found = store
        .find_messages(&room_id(), &[kept, redacted, missing])
        .await
        .expect("可以按 ID 读取");
    assert_eq!(
        found
            .iter()
            .map(|preview| preview.preview.summary().as_str())
            .collect::<Vec<_>>(),
        ["还在的"]
    );
    assert_eq!(found[0].message_id, kept);

    let other_room = MatrixRoomId::new("!other:matrix.test").expect("房间标识有效");
    assert!(
        store
            .find_messages(&other_room, &[kept])
            .await
            .expect("可以按 ID 读取")
            .is_empty(),
        "别的房间里查不到这个房间的消息"
    );
    assert!(
        store
            .find_messages(&room_id(), &[])
            .await
            .expect("空列表直接返回")
            .is_empty()
    );
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

#[tokio::test]
async fn 正文来源索引随替换迁移且撤回后立即消失() {
    let (_temporary, store, _inspector) = open_store().await;
    let message_id = MessageId::from_uuid(Uuid::now_v7());
    let base = preview_mutation(
        "$content-base:matrix.test",
        message_id,
        owner_actor(),
        1_700_000_000_000,
        "初始正文",
        1,
        Some(10),
    );
    let MessageProjectionMutation::Preview(base_preview) = &base else {
        panic!("测试夹具必须是预览");
    };
    let original_content_id = base_preview.content.content_id();
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("content-base"),
            vec![base],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("初始正文可投影");
    assert!(has_content_source(&store, original_content_id).await);

    let replacement = replacement_mutation(
        "$content-replacement:matrix.test",
        message_id,
        owner_actor(),
        "替换正文",
        2,
    );
    let MessageProjectionMutation::Revision(revision) = &replacement else {
        panic!("测试夹具必须是修订");
    };
    let replacement_content_id = revision
        .content
        .as_ref()
        .expect("替换正文存在")
        .content_id();
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("content-replacement"),
            vec![replacement],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("替换正文可投影");
    assert!(!has_content_source(&store, original_content_id).await);
    assert!(has_content_source(&store, replacement_content_id).await);

    store
        .apply(&MessageProjectionBatch::new(
            sync_token("content-redaction"),
            vec![redaction_mutation(
                "$content-redaction:matrix.test",
                message_id,
                owner_actor(),
            )],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("撤回可投影");
    assert!(!has_content_source(&store, replacement_content_id).await);
}

#[tokio::test]
async fn 客户端正文密钥落盘前必须由设备存储密钥封装() {
    let (_temporary, store, inspector) = open_store().await;
    let message_id = MessageId::from_uuid(Uuid::now_v7());
    let event = "$encrypted-content:matrix.test";
    let key = [0xAB; 32];
    let encryption = ClientContentEncryption::new(
        ClientContentEncryptionAlgorithm::Aes256GcmV1,
        ContentEncryptionContextId::from_uuid(
            Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e48").expect("加密上下文标识有效"),
        ),
        key,
        [0xCD; 12],
        128,
    )
    .expect("客户端加密元数据有效");
    let mut mutation = preview_mutation(
        event,
        message_id,
        owner_actor(),
        1_700_000_000_000,
        "加密正文",
        9,
        Some(10),
    );
    let MessageProjectionMutation::Preview(preview) = &mut mutation else {
        panic!("测试夹具必须是预览");
    };
    preview.content = MessageContentReference::new(
        ContentId::from_uuid(Uuid::now_v7()),
        Sha256Digest::from_bytes([9; 32]),
        144,
    )
    .expect("密文引用有效")
    .with_client_encryption(encryption);
    let content_id = preview.content.content_id();

    store
        .apply(&MessageProjectionBatch::new(
            sync_token("encrypted-content"),
            vec![mutation],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("加密正文可投影");

    let raw_content: String = sqlx::query_scalar(
        "SELECT content_json FROM message_current_projection WHERE message_id = ?",
    )
    .bind(message_id.to_string())
    .fetch_one(&inspector)
    .await
    .expect("原始正文投影可读取");
    assert!(!raw_content.contains(&URL_SAFE_NO_PAD.encode(key)));
    assert!(raw_content.contains("wrappedKeyBase64Url"));
    assert!(!raw_content.contains("keyBase64Url"));

    let projected = store
        .find_content_source(&MessageContentSourceQuery::new(room_id(), content_id))
        .await
        .expect("正文来源可查询")
        .expect("正文来源存在");
    let restored = projected
        .content
        .client_encryption()
        .expect("客户端加密元数据已恢复");
    assert_eq!(restored.key(), &key);
    assert_eq!(restored.nonce(), &[0xCD; 12]);
    assert_eq!(restored.plaintext_size_bytes(), 128);
}

async fn open_store() -> (TempDir, SqliteMessageTimelineRepository, SqlitePool) {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let store = SqliteMessageTimelineRepository::open(
        &path,
        &MessageProjectionStorageKey::from_bytes([29; 32]),
    )
    .await
    .expect("投影数据库可打开");
    let inspector = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .expect("只读检查连接可打开");
    (temporary, store, inspector)
}

async fn has_content_source(
    store: &SqliteMessageTimelineRepository,
    content_id: ContentId,
) -> bool {
    store
        .find_content_source(&MessageContentSourceQuery::new(room_id(), content_id))
        .await
        .expect("正文来源可查询")
        .is_some()
}

#[allow(clippy::too_many_arguments)]
#[tokio::test]
async fn 缺口补回后结清_补回的消息排在已收到的之后_已记下的事件能认出() {
    let (_temporary, store, inspector) = open_store().await;
    // 房间还没有任何消息时，不算“漏了消息”。
    assert!(!store.room_has_messages(&room_id()).await.expect("可查询"));
    let gap_token = MatrixBackfillToken::new("backfill-gap").expect("回填游标有效");
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("sync-1"),
            vec![preview_mutation(
                "$latest:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                2_000,
                "最新一条",
                1,
                Some(2_000),
            )],
            vec![MessageSyncIssue {
                room_id: room_id(),
                event_id: Some(event_id("$broken:matrix.test")),
                reason: MessageSyncIssueReason::Undecryptable,
                mutations_before: 1,
                session: None,
            }],
            vec![
                MessageTimelineGap {
                    room_id: room_id(),
                    previous_batch: Some(gap_token.clone()),
                },
                // 没有往回翻令牌的缺口补不了，不会交给补缺口流程。
                MessageTimelineGap {
                    room_id: room_id(),
                    previous_batch: None,
                },
            ],
        ))
        .await
        .expect("同步批次可写入");
    assert!(store.room_has_messages(&room_id()).await.expect("可查询"));
    assert_eq!(
        store
            .known_events(
                &room_id(),
                &[
                    event_id("$latest:matrix.test"),
                    event_id("$broken:matrix.test"),
                    event_id("$unknown:matrix.test"),
                ],
            )
            .await
            .expect("可查询"),
        [
            event_id("$latest:matrix.test"),
            event_id("$broken:matrix.test")
        ]
    );
    let gaps = store.pending_gaps(16).await.expect("可查询");
    assert_eq!(
        gaps,
        [PendingTimelineGap {
            sync_token: sync_token("sync-1"),
            room_id: room_id(),
            previous_batch: gap_token,
        }]
    );

    store
        .apply_backfill(&MessageBackfillBatch::new(
            gaps[0].clone(),
            vec![preview_mutation(
                "$missed:matrix.test",
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1_000,
                "离线时漏掉的一条",
                2,
                Some(1_000),
            )],
            vec![MessageSyncIssue {
                room_id: room_id(),
                event_id: Some(event_id("$forged:matrix.test")),
                reason: MessageSyncIssueReason::InvalidSignature,
                mutations_before: 1,
                session: None,
            }],
        ))
        .await
        .expect("补回的事件可写入");
    assert!(store.pending_gaps(16).await.expect("可查询").is_empty());
    // 补回的消息按到达顺序排在后面，收件箱把它当作新到的消息。
    let order = sqlx::query_scalar::<_, String>(
        "SELECT event_id FROM message_projection_event ORDER BY sequence ASC",
    )
    .fetch_all(&inspector)
    .await
    .expect("可查询");
    assert_eq!(order, ["$latest:matrix.test", "$missed:matrix.test"]);
    assert_eq!(
        scalar_count(&inspector, "SELECT COUNT(*) FROM message_sync_issue").await,
        2
    );
    // 补缺口不动同步游标。
    assert_eq!(current_cursor(&inspector).await.as_deref(), Some("sync-1"));
}

fn undecryptable(event: &str, session_id: &str, mutations_before: usize) -> MessageSyncIssue {
    MessageSyncIssue {
        room_id: room_id(),
        event_id: Some(event_id(event)),
        reason: MessageSyncIssueReason::Undecryptable,
        mutations_before,
        session: Some(UndecryptableSession {
            sender: MatrixUserId::new("@_agent_old:matrix.test").expect("用户有效"),
            sender_device: Some("OLDDEVICE".to_owned()),
            session_id: session_id.to_owned(),
        }),
    }
}

async fn event_order(inspector: &SqlitePool) -> Vec<(String, i64)> {
    sqlx::query("SELECT event_id, sequence FROM message_projection_event ORDER BY sequence ASC")
        .fetch_all(inspector)
        .await
        .expect("事件日志可查询")
        .iter()
        .map(|row| (row.get("event_id"), row.get("sequence")))
        .collect()
}

async fn page_event_ids(
    store: &SqliteMessageTimelineRepository,
    query: &MessagePreviewQuery,
) -> Vec<String> {
    store
        .list_previews(query)
        .await
        .expect("预览可查询")
        .previews()
        .iter()
        .map(|preview| preview.event_id.as_str().to_owned())
        .collect()
}

async fn sync_message(
    store: &SqliteMessageTimelineRepository,
    token: &str,
    event: &str,
    digest: u8,
    at: i64,
) {
    store
        .apply(&MessageProjectionBatch::new(
            sync_token(token),
            vec![preview_mutation(
                event,
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                at,
                "一条消息",
                digest,
                u64::try_from(at).ok(),
            )],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("同步批次可写入");
}

/// 时间线：加入前的一条（解不开）、加入后的一条、再一条解不开的（另一个会话），之后又来一条。
async fn sync_around_join(store: &SqliteMessageTimelineRepository) {
    let first = MessageProjectionBatch::new(
        sync_token("sync-1"),
        vec![preview_mutation(
            "$after-join:matrix.test",
            MessageId::from_uuid(Uuid::now_v7()),
            owner_actor(),
            2_000,
            "加入后的消息",
            1,
            Some(2_000),
        )],
        vec![
            undecryptable("$pre-join:matrix.test", "session-a", 0),
            undecryptable("$other:matrix.test", "session-b", 1),
        ],
        Vec::new(),
    );
    store.apply(&first).await.expect("同步批次可写入");
    // 重复同步到同一批不再占新位置。
    store.apply(&first).await.expect("重复同步可写入");
    sync_message(store, "sync-2", "$latest:matrix.test", 2, 3_000).await;
}

#[tokio::test]
async fn 解不开的消息按观察顺序占位_按会话查得到() {
    let (_temporary, store, inspector) = open_store().await;
    sync_around_join(&store).await;
    assert_eq!(
        event_order(&inspector).await,
        [
            ("$after-join:matrix.test".to_owned(), 2),
            ("$latest:matrix.test".to_owned(), 4)
        ]
    );

    let sessions = store.undecryptable_sessions(10).await.expect("可查询");
    assert_eq!(
        sessions
            .iter()
            .map(|isolated| isolated.session.session_id.as_str())
            .collect::<Vec<_>>(),
        ["session-b", "session-a"],
        "最近的会话在前"
    );
    assert_eq!(
        sessions[0].session.sender_device.as_deref(),
        Some("OLDDEVICE")
    );
    assert_eq!(
        store
            .undecryptable_events(&room_id(), &["session-a".to_owned()], 0, 10)
            .await
            .expect("可查询"),
        [ReservedIsolatedEvent {
            event_id: event_id("$pre-join:matrix.test"),
            position: 1,
        }]
    );
    assert!(
        store
            .undecryptable_events(&room_id(), &["session-a".to_owned()], 1, 10)
            .await
            .expect("可查询")
            .is_empty(),
        "按位置往后翻"
    );
}

#[tokio::test]
async fn 找回的消息写回原位_不当新消息_之后的新消息排在所有占位之后() {
    let (_temporary, store, inspector) = open_store().await;
    sync_around_join(&store).await;
    let pre_join = MessageId::from_uuid(Uuid::now_v7());

    store
        .apply_recovery(&MessageRecoveryBatch::new(
            room_id(),
            vec![preview_mutation(
                "$pre-join:matrix.test",
                pre_join,
                owner_actor(),
                1_000,
                "加入前的消息",
                3,
                Some(1_000),
            )],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("找回的消息可写入");

    assert_eq!(
        event_order(&inspector).await,
        [
            ("$pre-join:matrix.test".to_owned(), 1),
            ("$after-join:matrix.test".to_owned(), 2),
            ("$latest:matrix.test".to_owned(), 4)
        ]
    );
    assert_eq!(
        current_message(&inspector, pre_join)
            .await
            .get::<i64, _>("first_sequence"),
        1
    );
    // Agent 读到“加入后的消息”之后，收件箱只有更新的；找回的旧消息只出现在历史里。
    assert_eq!(
        page_event_ids(
            &store,
            &MessagePreviewQuery::after(room_id(), event_id("$after-join:matrix.test"), 10)
                .expect("游标有效"),
        )
        .await,
        ["$latest:matrix.test"]
    );
    assert_eq!(
        page_event_ids(
            &store,
            &MessagePreviewQuery::from_start(room_id(), 10).expect("分页有效")
        )
        .await,
        [
            "$pre-join:matrix.test",
            "$after-join:matrix.test",
            "$latest:matrix.test"
        ]
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT event_id FROM message_sync_issue")
            .fetch_all(&inspector)
            .await
            .expect("可查询"),
        ["$other:matrix.test"],
        "找回的那条不再隔离"
    );
    sync_message(&store, "sync-3", "$newest:matrix.test", 4, 4_000).await;
    assert_eq!(
        event_order(&inspector).await.last(),
        Some(&("$newest:matrix.test".to_owned(), 5))
    );
}

#[tokio::test]
async fn 找回的旧修订不覆盖已生效的新修订_找回的原消息补上先到的修订() {
    let (_temporary, store, inspector) = open_store().await;
    let edited = MessageId::from_uuid(Uuid::now_v7());
    let recovered_base = MessageId::from_uuid(Uuid::now_v7());
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("sync-1"),
            vec![
                preview_mutation(
                    "$edited:matrix.test",
                    edited,
                    owner_actor(),
                    1_000,
                    "原文",
                    1,
                    Some(1_000),
                ),
                replacement_mutation(
                    "$newer-edit:matrix.test",
                    edited,
                    owner_actor(),
                    "第二次修改",
                    2,
                ),
                replacement_mutation(
                    "$edit-before-base:matrix.test",
                    recovered_base,
                    owner_actor(),
                    "改过的加入前消息",
                    3,
                ),
            ],
            vec![
                undecryptable("$base-before-join:matrix.test", "session-a", 0),
                undecryptable("$older-edit:matrix.test", "session-a", 1),
            ],
            Vec::new(),
        ))
        .await
        .expect("同步批次可写入");

    store
        .apply_recovery(&MessageRecoveryBatch::new(
            room_id(),
            vec![
                preview_mutation(
                    "$base-before-join:matrix.test",
                    recovered_base,
                    owner_actor(),
                    500,
                    "加入前的原文",
                    4,
                    Some(500),
                ),
                replacement_mutation(
                    "$older-edit:matrix.test",
                    edited,
                    owner_actor(),
                    "第一次修改",
                    5,
                ),
            ],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("找回的消息可写入");

    let edited_row = current_message(&inspector, edited).await;
    assert_eq!(edited_row.get::<String, _>("summary"), "第二次修改");
    assert_eq!(
        edited_row.get::<String, _>("last_revision_event_id"),
        "$newer-edit:matrix.test"
    );
    let base_row = current_message(&inspector, recovered_base).await;
    assert_eq!(base_row.get::<String, _>("summary"), "改过的加入前消息");
    assert_eq!(base_row.get::<i64, _>("first_sequence"), 1);
}

#[tokio::test]
async fn 解开后不收的改记原因_不是消息的删掉记录() {
    let (_temporary, store, inspector) = open_store().await;
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("sync-1"),
            Vec::new(),
            vec![
                undecryptable("$forged:matrix.test", "session-a", 0),
                undecryptable("$reaction:matrix.test", "session-a", 0),
                // 功能上线前记下的隔离事件没有会话，不占位置，也不会被重读。
                MessageSyncIssue {
                    room_id: room_id(),
                    event_id: Some(event_id("$legacy:matrix.test")),
                    reason: MessageSyncIssueReason::Undecryptable,
                    mutations_before: 0,
                    session: None,
                },
            ],
            Vec::new(),
        ))
        .await
        .expect("同步批次可写入");
    assert_eq!(
        store
            .undecryptable_events(&room_id(), &["session-a".to_owned()], 0, 10)
            .await
            .expect("可查询")
            .len(),
        2
    );

    store
        .apply_recovery(&MessageRecoveryBatch::new(
            room_id(),
            Vec::new(),
            vec![MessageSyncIssue {
                room_id: room_id(),
                event_id: Some(event_id("$forged:matrix.test")),
                reason: MessageSyncIssueReason::InvalidSignature,
                mutations_before: 0,
                session: None,
            }],
            vec![event_id("$reaction:matrix.test")],
        ))
        .await
        .expect("重读结果可写入");

    let rows = sqlx::query(
        "SELECT event_id, reason, reserved_sequence FROM message_sync_issue ORDER BY event_id",
    )
    .fetch_all(&inspector)
    .await
    .expect("可查询");
    assert_eq!(
        rows.iter()
            .map(|row| (
                row.get::<String, _>("event_id"),
                row.get::<String, _>("reason"),
                row.get::<Option<i64>, _>("reserved_sequence"),
            ))
            .collect::<Vec<_>>(),
        [
            (
                "$forged:matrix.test".to_owned(),
                "invalid_signature".to_owned(),
                None
            ),
            (
                "$legacy:matrix.test".to_owned(),
                "undecryptable".to_owned(),
                None
            ),
        ]
    );
    assert!(
        store
            .undecryptable_sessions(10)
            .await
            .expect("可查询")
            .is_empty()
    );
}

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

#[tokio::test]
async fn 人类聊天持久化且自报同一账号不能篡改他人消息() {
    let (temporary, store, _) = open_store().await;
    let principal_id = agent_room_domain::ids::PrincipalId::from_uuid(Uuid::now_v7());
    let human = ProjectedMessageActor::Human {
        principal_id,
        display_name: "小雨".to_owned(),
        matrix_user_id: MatrixUserId::new("@rainy:matrix.test").expect("用户有效"),
        avatar_url: None,
    };
    let impostor = ProjectedMessageActor::Human {
        principal_id,
        display_name: "小雨".to_owned(),
        matrix_user_id: MatrixUserId::new("@impostor:matrix.test").expect("用户有效"),
        avatar_url: None,
    };
    let id = MessageId::from_uuid(Uuid::now_v7());
    let mut mutation = preview_mutation(
        "$human-chat",
        id,
        human.clone(),
        1_000,
        "原始问题",
        1,
        Some(1_000),
    );
    if let MessageProjectionMutation::Preview(message) = &mut mutation {
        message.preview = message.preview.clone().with_conversation(
            agent_room_domain::messages::ConversationMessage::new(
                "原始问题".to_owned(),
                vec!["@agent:matrix.test".to_owned()],
            )
            .expect("聊天有效")
            .with_attachment_name(Some("design.png".to_owned()))
            .expect("附件名有效"),
        );
    }
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("human-chat"),
            vec![
                mutation,
                replacement_mutation("$forged-edit", id, impostor, "伪造替换", 2),
            ],
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("投影成功");
    let query = MessagePreviewQuery::new(room_id(), None, 20).expect("查询有效");
    let page = store.list_previews(&query).await.expect("读取成功");
    assert_eq!(page.previews()[0].actor, human);
    assert_eq!(
        page.previews()[0]
            .preview
            .conversation()
            .expect("聊天仍在")
            .text(),
        "原始问题"
    );
    assert_eq!(page.previews()[0].preview.summary().as_str(), "原始问题");
    drop(store);
    let reopened = SqliteMessageTimelineRepository::open(
        &temporary.path().join("messages.sqlite3"),
        &MessageProjectionStorageKey::from_bytes([29; 32]),
    )
    .await
    .expect("数据库可重开");
    let recovered = reopened
        .list_previews(&query)
        .await
        .expect("重开后可读取人类聊天");
    assert_eq!(recovered.previews()[0].actor, human);
    assert_eq!(
        recovered.previews()[0]
            .preview
            .conversation()
            .expect("聊天已持久化")
            .attachment_name(),
        Some("design.png")
    );
    assert_eq!(
        recovered.previews()[0]
            .preview
            .conversation()
            .expect("聊天已持久化")
            .text(),
        "原始问题"
    );
}

#[tokio::test]
async fn 增量分页不遗漏突发消息且拒绝跨房间游标() {
    let (_temporary, store, _inspector) = open_store().await;
    let start = MessagePreviewQuery::from_start(room_id(), 2).expect("初次取信");
    assert!(
        store
            .list_previews(&start)
            .await
            .expect("空房间")
            .previews()
            .is_empty()
    );
    let mutations = (0..6)
        .map(|index| {
            preview_mutation(
                &format!("$chat-{index}"),
                MessageId::from_uuid(Uuid::now_v7()),
                owner_actor(),
                1000,
                "连续消息",
                index + 1,
                Some(1000),
            )
        })
        .collect();
    store
        .apply(&MessageProjectionBatch::new(
            sync_token("incremental"),
            mutations,
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("写入成功");
    let beginning = store
        .list_previews(&start)
        .await
        .expect("空房间首批突发消息");
    assert_eq!(
        beginning
            .previews()
            .iter()
            .map(|message| message.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$chat-0", "$chat-1"]
    );
    let first = store
        .list_previews(
            &MessagePreviewQuery::after(room_id(), event_id("$chat-0"), 2).expect("游标有效"),
        )
        .await
        .expect("分页成功");
    assert_eq!(
        first
            .previews()
            .iter()
            .map(|message| message.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$chat-1", "$chat-2"]
    );
    let next = store
        .list_previews(
            &MessagePreviewQuery::after(room_id(), event_id("$chat-2"), 2).expect("游标有效"),
        )
        .await
        .expect("分页成功");
    assert_eq!(
        next.previews()
            .iter()
            .map(|message| message.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$chat-3", "$chat-4"]
    );
    let foreign = MatrixRoomId::new("!elsewhere:matrix.test").expect("房间有效");
    assert!(
        store
            .list_previews(
                &MessagePreviewQuery::after(foreign, event_id("$chat-0"), 2).expect("格式有效")
            )
            .await
            .is_err()
    );
}
