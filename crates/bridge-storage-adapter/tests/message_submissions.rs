use std::sync::Arc;

use agent_room_application::ports::{MatrixEventId, MatrixTransactionId};
use agent_room_bridge_core::messages::{
    MessageStoreFailureKind, MessageSubmissionClaim, MessageSubmissionClaimOutcome,
    MessageSubmissionFingerprint, MessageSubmissionKind, MessageSubmissionRepository,
    MessageSubmissionState,
};
use agent_room_bridge_storage_adapter::SqliteMessageSubmissionRepository;
use agent_room_domain::ids::MessageSubmissionId;
use tempfile::TempDir;
use uuid::Uuid;

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
