use std::{path::Path, sync::Arc, time::Duration};

use agent_room_application::ports::{MatrixEventId, MatrixTransactionId};
use agent_room_bridge_core::messages::{
    MessageStoreFailureKind, MessageSubmissionClaim, MessageSubmissionClaimOutcome,
    MessageSubmissionFingerprint, MessageSubmissionKind, MessageSubmissionRepository,
    MessageSubmissionState,
};
use agent_room_bridge_storage_adapter::SqliteMessageSubmissionRepository;
use agent_room_domain::ids::MessageSubmissionId;
use sqlx::{Connection as _, SqliteConnection, sqlite::SqliteConnectOptions};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// 远小于连接的忙等待上限，足以让被测操作撞上另一连接持有的写锁。
const CONTENTION_WINDOW: Duration = Duration::from_millis(200);

#[tokio::test]
async fn 未知提交跨进程重启后仍可通过事务观察完成对账() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("message-state.sqlite3");
    let claim = submission_claim(Uuid::now_v7(), 7, "stable-transaction");

    let first = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可打开");
    assert!(matches!(
        first.claim(&claim).await.expect("首次占位成功"),
        MessageSubmissionClaimOutcome::Created(_)
    ));
    let unknown = first
        .mark_submit_unknown(claim.submission_id)
        .await
        .expect("未知状态可持久化");
    assert_eq!(unknown.state, MessageSubmissionState::SubmitUnknown);
    drop(first);

    let reopened = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可重新打开");
    let restored = reopened.claim(&claim).await.expect("原提交可恢复");
    assert_eq!(
        restored.record().state,
        MessageSubmissionState::SubmitUnknown
    );
    let event_id = MatrixEventId::new("$observed:matrix.test").expect("事件标识有效");
    let observed = reopened
        .observe_transaction(&claim.transaction_id, &event_id)
        .await
        .expect("事务观察成功")
        .expect("找到原事务");
    assert_eq!(observed.state, MessageSubmissionState::Accepted);
    let bound = reopened
        .mark_bound(claim.submission_id)
        .await
        .expect("绑定状态可持久化");
    assert_eq!(bound.state, MessageSubmissionState::Bound);
    drop(reopened);

    let final_store = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可第三次打开");
    assert_eq!(
        final_store
            .claim(&claim)
            .await
            .expect("完成记录可恢复")
            .record()
            .state,
        MessageSubmissionState::Bound
    );
}

#[tokio::test]
async fn 并发占位只有一个创建者且冲突意图被拒绝() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let store = Arc::new(
        SqliteMessageSubmissionRepository::open(temporary.path().join("concurrent.sqlite3"))
            .await
            .expect("数据库可打开"),
    );
    let claim = submission_claim(Uuid::now_v7(), 11, "concurrent-transaction");
    let mut workers = Vec::new();
    for _ in 0..12 {
        let worker_store = Arc::clone(&store);
        let worker_claim = claim.clone();
        workers.push(tokio::spawn(async move {
            worker_store.claim(&worker_claim).await
        }));
    }
    let mut created = 0;
    for worker in workers {
        let outcome = worker.await.expect("并发任务完成").expect("占位成功");
        if matches!(outcome, MessageSubmissionClaimOutcome::Created(_)) {
            created += 1;
        }
    }
    assert_eq!(created, 1);

    let conflicting = MessageSubmissionClaim {
        fingerprint: MessageSubmissionFingerprint::from_bytes([99; 32]),
        ..claim.clone()
    };
    let failure = store
        .claim(&conflicting)
        .await
        .expect_err("同一幂等键不能复用为其他意图");
    assert_eq!(failure.kind(), MessageStoreFailureKind::Conflict);

    let duplicate_transaction = submission_claim(Uuid::now_v7(), 12, claim.transaction_id.as_str());
    let failure = store
        .claim(&duplicate_transaction)
        .await
        .expect_err("同一事务号不能绑定两个提交");
    assert_eq!(failure.kind(), MessageStoreFailureKind::Conflict);
}

// 运行时的投影和提交仓储是同一 messages.sqlite 上的两个连接池。发送成功后，
// 同步循环几乎同时收到自己的回显并写库；发送路径推进状态时必须等写锁，
// 不能把锁竞争当成存储不可用返回给 MCP。
#[tokio::test]
async fn 同步循环持有写锁时发送路径推进为已接受会等待而不是报存储不可用() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let store = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可打开");
    let claim = submission_claim(Uuid::now_v7(), 31, "echo-before-accept");
    store.claim(&claim).await.expect("首次占位成功");
    let event_id = MatrixEventId::new("$echo:matrix.test").expect("事件标识有效");

    // 同步循环已拿到写锁，正在把回显对应的提交标成已接受。
    let mut sync_loop = hold_write_lock(&path).await;
    sqlx::query(
        "UPDATE message_submissions SET state = 'accepted', event_id = ?
         WHERE transaction_id = ?",
    )
    .bind(event_id.as_str())
    .bind(claim.transaction_id.as_str())
    .execute(&mut sync_loop)
    .await
    .expect("同步循环写入回显");

    let sender = store.clone();
    let submission_id = claim.submission_id;
    let accepted_event = event_id.clone();
    let pending =
        tokio::spawn(async move { sender.mark_accepted(submission_id, &accepted_event).await });
    let record = release_after_contention(sync_loop, pending)
        .await
        .expect("写锁释放后发送路径完成状态推进");

    assert_eq!(record.state, MessageSubmissionState::Accepted);
    assert_eq!(record.event_id.as_ref(), Some(&event_id));
}

#[tokio::test]
async fn 发送路径持有写锁时同步循环观察回显会等待而不是触发重连() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let store = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可打开");
    let claim = submission_claim(Uuid::now_v7(), 32, "accept-before-echo");
    store.claim(&claim).await.expect("首次占位成功");
    let event_id = MatrixEventId::new("$accepted:matrix.test").expect("事件标识有效");

    let mut sender = hold_write_lock(&path).await;
    sqlx::query(
        "UPDATE message_submissions SET state = 'accepted', event_id = ?
         WHERE submission_id = ?",
    )
    .bind(event_id.as_str())
    .bind(claim.submission_id.to_string())
    .execute(&mut sender)
    .await
    .expect("发送路径写入已接受");

    let sync_loop = store.clone();
    let transaction_id = claim.transaction_id.clone();
    let echoed_event = event_id.clone();
    let pending = tokio::spawn(async move {
        sync_loop
            .observe_transaction(&transaction_id, &echoed_event)
            .await
    });
    let observed = release_after_contention(sender, pending)
        .await
        .expect("写锁释放后同步循环完成对账")
        .expect("找到原事务");

    assert_eq!(observed.state, MessageSubmissionState::Accepted);
    assert_eq!(observed.event_id.as_ref(), Some(&event_id));
}

#[tokio::test]
async fn 另一连接持有写锁时绑定与未知状态推进同样会等待() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let store = SqliteMessageSubmissionRepository::open(&path)
        .await
        .expect("数据库可打开");
    let accepted = submission_claim(Uuid::now_v7(), 33, "bind-under-contention");
    let unknown = submission_claim(Uuid::now_v7(), 34, "unknown-under-contention");
    store.claim(&accepted).await.expect("首次占位成功");
    store.claim(&unknown).await.expect("首次占位成功");
    let event_id = MatrixEventId::new("$bind:matrix.test").expect("事件标识有效");
    store
        .mark_accepted(accepted.submission_id, &event_id)
        .await
        .expect("已接受可持久化");

    let holder = hold_write_lock(&path).await;
    let bind_store = store.clone();
    let bind_id = accepted.submission_id;
    let pending_bind = tokio::spawn(async move { bind_store.mark_bound(bind_id).await });
    let bound = release_after_contention(holder, pending_bind)
        .await
        .expect("写锁释放后绑定成功");
    assert_eq!(bound.state, MessageSubmissionState::Bound);

    let holder = hold_write_lock(&path).await;
    let unknown_store = store.clone();
    let unknown_id = unknown.submission_id;
    let pending_unknown =
        tokio::spawn(async move { unknown_store.mark_submit_unknown(unknown_id).await });
    let marked = release_after_contention(holder, pending_unknown)
        .await
        .expect("写锁释放后未知状态可持久化");
    assert_eq!(marked.state, MessageSubmissionState::SubmitUnknown);
}

// 与运行时相同：两个独立连接池打开同一文件，发送路径和同步循环对同一批提交并发推进状态。
#[tokio::test]
async fn 同一文件上的两个连接池并发推进同一批提交都不会报存储不可用() {
    let temporary = TempDir::new().expect("临时目录可创建");
    let path = temporary.path().join("messages.sqlite3");
    let sender = Arc::new(
        SqliteMessageSubmissionRepository::open(&path)
            .await
            .expect("发送路径连接池可打开"),
    );
    let sync_loop = Arc::new(
        SqliteMessageSubmissionRepository::open(&path)
            .await
            .expect("同步循环连接池可打开"),
    );
    let mut workers = Vec::new();
    for index in 0..48_u8 {
        let claim = submission_claim(Uuid::now_v7(), index, &format!("race-{index}"));
        sender.claim(&claim).await.expect("首次占位成功");
        let event_id =
            MatrixEventId::new(format!("$race-{index}:matrix.test")).expect("事件标识有效");

        let accept_store = Arc::clone(&sender);
        let accept_event = event_id.clone();
        let submission_id = claim.submission_id;
        workers.push(tokio::spawn(async move {
            accept_store
                .mark_accepted(submission_id, &accept_event)
                .await
                .map(|_| ())
        }));
        let observe_store = Arc::clone(&sync_loop);
        let transaction_id = claim.transaction_id.clone();
        workers.push(tokio::spawn(async move {
            observe_store
                .observe_transaction(&transaction_id, &event_id)
                .await
                .map(|_| ())
        }));
    }
    for worker in workers {
        worker
            .await
            .expect("并发任务完成")
            .expect("锁竞争不能表现为存储不可用");
    }
}

async fn hold_write_lock(path: &Path) -> SqliteConnection {
    let mut connection =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path))
            .await
            .expect("第二条连接可打开");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut connection)
        .await
        .expect("第二条连接拿到写锁");
    connection
}

async fn release_after_contention<T>(mut holder: SqliteConnection, pending: JoinHandle<T>) -> T {
    tokio::time::sleep(CONTENTION_WINDOW).await;
    assert!(
        !pending.is_finished(),
        "写锁被另一连接占用时应等待锁释放，而不是立即失败"
    );
    sqlx::query("COMMIT")
        .execute(&mut holder)
        .await
        .expect("持锁连接提交");
    pending.await.expect("被测任务完成")
}

fn submission_claim(
    submission_id: Uuid,
    fingerprint: u8,
    transaction_id: &str,
) -> MessageSubmissionClaim {
    MessageSubmissionClaim {
        submission_id: MessageSubmissionId::from_uuid(submission_id),
        kind: MessageSubmissionKind::Preview,
        fingerprint: MessageSubmissionFingerprint::from_bytes([fingerprint; 32]),
        transaction_id: MatrixTransactionId::new(transaction_id).expect("事务标识有效"),
    }
}
