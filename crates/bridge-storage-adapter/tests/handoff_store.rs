use std::{path::Path, sync::Arc};

use agent_room_bridge_core::handoffs::{
    HandoffReceiptRecord, HandoffRecordOutcome, HandoffStore, HandoffStoreCommand,
    HandoffStoreCommandOutcome, HandoffStoreFailureKind, OneShotHandoffPackage,
    RemoteHandoffReceiptStatus,
};
use agent_room_bridge_storage_adapter::{HandoffStorageKey, SqliteHandoffStore};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
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
use sha2::{Digest as _, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tempfile::TempDir;
use uuid::Uuid;

const STORAGE_KEY: [u8; 32] = [7; 32];

struct 测试交付 {
    handoff: ContextHandoff,
    package: OneShotHandoffPackage,
}

#[tokio::test]
async fn 正文只以密文落盘且可跨重启消费() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let body = Arc::<[u8]>::from(b"unique plaintext handoff body 2026".as_slice());
    let fixture = delivered_handoff(body.clone());
    let store = open_store(&path, STORAGE_KEY).await;

    let outcome = store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");
    assert!(matches!(outcome, HandoffRecordOutcome::Created(_)));
    store.close().await;

    assert_database_files_do_not_contain(&path, body.as_ref());

    let reopened = open_store(&path, STORAGE_KEY).await;
    assert_eq!(
        reopened
            .find(fixture.handoff.fields().id)
            .await
            .expect("记录可读取")
            .expect("记录存在"),
        fixture.handoff
    );
    let consumed = consume(&reopened, &fixture, time(1_300))
        .await
        .expect("重启后仍可解密并消费");
    assert_eq!(consumed.body().as_ref(), body.as_ref());
    reopened.close().await;

    assert_eq!(package_count(&path).await, 0);
}

#[tokio::test]
async fn 消费与密文删除原子执行且只能成功一次() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"consume-once".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");

    let consumed = consume(&store, &fixture, time(1_300))
        .await
        .expect("首次消费成功");
    assert_eq!(consumed.handoff().status(), HandoffStatus::Consumed);
    assert_eq!(package_count(&path).await, 0);

    let failure = consume(&store, &fixture, time(1_400))
        .await
        .expect_err("重复消费必须失败");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::AlreadyResolved);
    assert_eq!(package_count(&path).await, 0);
}

#[tokio::test]
async fn 错误目标实例不能改变状态或销毁密文() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"target-bound".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");

    let failure = store
        .apply(
            fixture.handoff.fields().id,
            HandoffStoreCommand::Consume {
                target_instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
                occurred_at: time(1_300),
            },
        )
        .await
        .expect_err("错误实例不能消费");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::Conflict);
    assert_eq!(package_count(&path).await, 1);
    assert_eq!(
        store
            .find(fixture.handoff.fields().id)
            .await
            .expect("状态可读取")
            .expect("记录存在")
            .status(),
        HandoffStatus::Delivered
    );

    assert!(consume(&store, &fixture, time(1_300)).await.is_ok());
}

#[tokio::test]
async fn 拒绝撤销和到期都会销毁尚未消费的密文() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let store = open_store(&path, STORAGE_KEY).await;
    let declined = delivered_handoff(Arc::from(b"declined".as_slice()));
    let revoked = delivered_handoff(Arc::from(b"revoked".as_slice()));
    let expired = delivered_handoff(Arc::from(b"expired".as_slice()));
    for fixture in [&declined, &revoked, &expired] {
        store
            .accept_incoming(&fixture.handoff, &fixture.package)
            .await
            .expect("合法交付可以落库");
    }
    assert_eq!(package_count(&path).await, 3);

    apply_target_command(
        &store,
        &declined,
        HandoffStoreCommand::Decline {
            target_instance_id: declined.handoff.fields().target_instance_id,
            occurred_at: time(1_300),
        },
    )
    .await;
    apply_target_command(
        &store,
        &revoked,
        HandoffStoreCommand::Revoke {
            target_instance_id: revoked.handoff.fields().target_instance_id,
            occurred_at: time(1_300),
        },
    )
    .await;
    apply_target_command(
        &store,
        &expired,
        HandoffStoreCommand::Expire {
            target_instance_id: expired.handoff.fields().target_instance_id,
            occurred_at: time(5_000),
        },
    )
    .await;

    assert_eq!(package_count(&path).await, 0);
}

#[tokio::test]
async fn 密文篡改会失败且事务不会提前终结交付() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"tamper-evident".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");
    replace_ciphertext(&path, fixture.handoff.fields().id).await;

    let failure = consume(&store, &fixture, time(1_300))
        .await
        .expect_err("篡改密文不能进入 Agent 上下文");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::Corrupt);
    assert_eq!(package_count(&path).await, 1);
    assert_eq!(
        store
            .find(fixture.handoff.fields().id)
            .await
            .expect("状态可读取")
            .expect("记录存在")
            .status(),
        HandoffStatus::Delivered
    );
}

#[tokio::test]
async fn 错误密钥不能消费且不会破坏可恢复数据() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"wrong-key".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");
    store.close().await;

    let wrong_key_store = open_store(&path, [8; 32]).await;
    let failure = consume(&wrong_key_store, &fixture, time(1_300))
        .await
        .expect_err("错误密钥不能解密");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::Corrupt);
    wrong_key_store.close().await;
    assert_eq!(package_count(&path).await, 1);

    let recovered = open_store(&path, STORAGE_KEY).await;
    assert!(consume(&recovered, &fixture, time(1_300)).await.is_ok());
}

#[tokio::test]
async fn 相同意图重放幂等而同一标识的不同意图冲突() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"replay".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;

    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("首次交付成功");
    let replay = store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("相同意图重放幂等");
    assert!(replay.reused());
    assert_eq!(package_count(&path).await, 1);

    let conflicting = changed_purpose(&fixture.handoff);
    let failure = store
        .accept_incoming(&conflicting, &fixture.package)
        .await
        .expect_err("同一标识不能改写意图");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::Conflict);
}

#[tokio::test]
async fn 到期消费会原子标记过期并清除密文() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"expire-on-consume".as_slice()));
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .accept_incoming(&fixture.handoff, &fixture.package)
        .await
        .expect("合法交付可以落库");

    let failure = consume(&store, &fixture, time(5_000))
        .await
        .expect_err("到期瞬间不能再消费");
    assert_eq!(failure.kind(), HandoffStoreFailureKind::Expired);
    assert_eq!(package_count(&path).await, 0);
    assert_eq!(
        store
            .find(fixture.handoff.fields().id)
            .await
            .expect("状态可读取")
            .expect("记录存在")
            .status(),
        HandoffStatus::Expired
    );
}

#[tokio::test]
async fn 发件状态可以用认证回执推进且晚到重放保持幂等() {
    let directory = tempfile::tempdir().expect("可创建临时目录");
    let path = database_path(&directory);
    let fixture = delivered_handoff(Arc::from(b"outgoing-receipts".as_slice()));
    let mut outgoing =
        ContextHandoff::propose(fixture.handoff.fields().clone()).expect("发件提案有效");
    outgoing
        .approve(
            fixture
                .handoff
                .approved_by_principal_id()
                .expect("测试交付已批准"),
            fixture.handoff.approved_at().expect("测试交付有批准时间"),
        )
        .expect("发件批准有效");
    let store = open_store(&path, STORAGE_KEY).await;
    store
        .record_outgoing(&outgoing)
        .await
        .expect("发件状态可以落库");

    let delivered_receipt = receipt(
        &outgoing,
        RemoteHandoffReceiptStatus::Delivered,
        time(1_200),
    );
    assert_eq!(
        store
            .apply_receipt(&delivered_receipt)
            .await
            .expect("送达回执有效")
            .status(),
        HandoffStatus::Delivered
    );
    let consumed_receipt = receipt(&outgoing, RemoteHandoffReceiptStatus::Consumed, time(1_300));
    assert_eq!(
        store
            .apply_receipt(&consumed_receipt)
            .await
            .expect("消费回执有效")
            .status(),
        HandoffStatus::Consumed
    );
    assert_eq!(
        store
            .apply_receipt(&consumed_receipt)
            .await
            .expect("重复消费回执幂等")
            .status(),
        HandoffStatus::Consumed
    );
    let replay = store
        .record_outgoing(&outgoing)
        .await
        .expect("原始发件意图晚到重放幂等");
    assert!(replay.reused());
    assert_eq!(replay.handoff().status(), HandoffStatus::Consumed);
}

fn delivered_handoff(body: Arc<[u8]>) -> 测试交付 {
    let digest = Sha256Digest::from_bytes(Sha256::digest(body.as_ref()).into());
    let id = HandoffId::from_uuid(Uuid::now_v7());
    let mut handoff = ContextHandoff::propose(ContextHandoffFields {
        id,
        requester_agent_id: AgentId::from_uuid(Uuid::now_v7()),
        requester_instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        source: HandoffSource::new(
            MatrixRoomReference::new("!builders:matrix.test").expect("房间标识有效"),
            HandoffSourceEventId::new("$source:matrix.test").expect("事件标识有效"),
            MessageId::from_uuid(Uuid::now_v7()),
            HandoffSourceActor::new(
                AgentId::from_uuid(Uuid::now_v7()),
                AgentInstanceId::from_uuid(Uuid::now_v7()),
                MessageProvenance::AutonomousAgent,
            ),
        ),
        target_agent_id: AgentId::from_uuid(Uuid::now_v7()),
        target_instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        content: HandoffContentReference::new(
            ContentId::from_uuid(Uuid::now_v7()),
            digest,
            ContentByteLength::new(u64::try_from(body.len()).expect("测试正文长度可转换"))
                .expect("测试正文长度有效"),
            ContentMediaType::new("text/plain").expect("媒体类型有效"),
        ),
        permissions: HandoffPermissions::new([
            HandoffPermission::ReadText,
            HandoffPermission::IncludeMetadata,
        ])
        .expect("权限范围有效"),
        purpose: HandoffPurpose::Summarize,
        risk_flags: MessageRiskFlags::new([
            MessageRiskFlag::new("untrusted_instructions").expect("风险标签有效")
        ])
        .expect("风险标签集合有效"),
        proposed_at: time(1_000),
        expires_at: time(5_000),
    })
    .expect("交付提案有效");
    handoff
        .approve(PrincipalId::from_uuid(Uuid::now_v7()), time(1_100))
        .expect("批准有效");
    handoff.mark_delivered(time(1_200)).expect("交付有效");
    测试交付 {
        handoff,
        package: OneShotHandoffPackage::new(id, body),
    }
}

fn changed_purpose(original: &ContextHandoff) -> ContextHandoff {
    let mut fields = original.fields().clone();
    fields.purpose = HandoffPurpose::ReplyDraft;
    let mut changed = ContextHandoff::propose(fields).expect("修改后的提案仍有效");
    changed
        .approve(
            original.approved_by_principal_id().expect("原交付已批准"),
            original.approved_at().expect("原交付有批准时间"),
        )
        .expect("批准有效");
    changed
        .mark_delivered(original.delivered_at().expect("原交付有送达时间"))
        .expect("送达有效");
    changed
}

fn receipt(
    handoff: &ContextHandoff,
    status: RemoteHandoffReceiptStatus,
    occurred_at: UtcMillis,
) -> HandoffReceiptRecord {
    HandoffReceiptRecord::new(
        handoff.fields().id,
        handoff.fields().target_agent_id,
        handoff.fields().target_instance_id,
        handoff.fields().requester_instance_id,
        status,
        None,
        occurred_at,
    )
}

async fn consume(
    store: &SqliteHandoffStore,
    fixture: &测试交付,
    occurred_at: UtcMillis,
) -> Result<
    agent_room_bridge_core::handoffs::ConsumedHandoffContext,
    agent_room_bridge_core::handoffs::HandoffStoreFailure,
> {
    match store
        .apply(
            fixture.handoff.fields().id,
            HandoffStoreCommand::Consume {
                target_instance_id: fixture.handoff.fields().target_instance_id,
                occurred_at,
            },
        )
        .await?
    {
        HandoffStoreCommandOutcome::Consumed(context) => Ok(context),
        HandoffStoreCommandOutcome::Updated(_) => {
            panic!("消费命令不能返回非消费结果")
        }
    }
}

async fn apply_target_command(
    store: &SqliteHandoffStore,
    fixture: &测试交付,
    command: HandoffStoreCommand,
) {
    let outcome = store
        .apply(fixture.handoff.fields().id, command)
        .await
        .expect("终态命令成功");
    assert!(matches!(outcome, HandoffStoreCommandOutcome::Updated(_)));
}

async fn open_store(path: &Path, key: [u8; 32]) -> SqliteHandoffStore {
    SqliteHandoffStore::open(path, HandoffStorageKey::from_bytes(key))
        .await
        .expect("一次性上下文数据库可打开")
}

async fn package_count(path: &Path) -> i64 {
    let pool = open_raw_pool(path).await;
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context_handoff_package")
        .fetch_one(&pool)
        .await
        .expect("可统计密文包");
    pool.close().await;
    count
}

async fn replace_ciphertext(path: &Path, handoff_id: HandoffId) {
    let pool = open_raw_pool(path).await;
    sqlx::query(
        "UPDATE context_handoff_package
         SET ciphertext = randomblob(length(ciphertext)) WHERE handoff_id = ?",
    )
    .bind(handoff_id.to_string())
    .execute(&pool)
    .await
    .expect("测试可篡改密文");
    pool.close().await;
}

async fn open_raw_pool(path: &Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false),
        )
        .await
        .expect("测试数据库可直接打开")
}

fn assert_database_files_do_not_contain(path: &Path, plaintext: &[u8]) {
    for candidate in [path.to_path_buf(), suffix_path(path, "-wal")] {
        if candidate.exists() {
            let bytes = std::fs::read(&candidate).expect("数据库文件可读取");
            assert!(
                !contains_subslice(&bytes, plaintext),
                "数据库文件不得出现 Handoff 明文：{}",
                candidate.display()
            );
        }
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn suffix_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    value.into()
}

fn database_path(directory: &TempDir) -> std::path::PathBuf {
    directory.path().join("handoffs").join("handoffs.sqlite")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
