use std::{path::Path, time::Duration};

use agent_room_application::ports::{MatrixEventId, MatrixTransactionId, PortFuture};
use agent_room_bridge_core::messages::{
    MessageStoreFailure, MessageStoreFailureKind, MessageSubmissionClaim,
    MessageSubmissionClaimOutcome, MessageSubmissionFingerprint, MessageSubmissionKind,
    MessageSubmissionRecord, MessageSubmissionRepository, MessageSubmissionState,
};
use agent_room_domain::ids::MessageSubmissionId;
use sqlx::{
    Row as _, Sqlite, SqlitePool, Transaction,
    migrate::MigrateError,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use thiserror::Error;
use uuid::{Uuid, Version};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: u32 = 4;

#[derive(Debug, Error)]
pub enum SqliteMessageSubmissionOpenFailure {
    #[error("无法创建 Bridge 消息状态目录")]
    CreateDirectory(#[source] std::io::Error),
    #[error("无法打开 Bridge 消息状态数据库")]
    Connect(#[source] sqlx::Error),
    #[error("无法迁移 Bridge 消息状态数据库")]
    Migrate(#[source] MigrateError),
}

#[derive(Clone)]
pub struct SqliteMessageSubmissionRepository {
    pool: SqlitePool,
}

impl SqliteMessageSubmissionRepository {
    /// 打开并迁移 Bridge 独占的消息提交数据库。
    ///
    /// # Errors
    ///
    /// 目录不可创建、数据库不可连接或迁移失败时返回错误。
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SqliteMessageSubmissionOpenFailure> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(SqliteMessageSubmissionOpenFailure::CreateDirectory)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await
            .map_err(SqliteMessageSubmissionOpenFailure::Connect)?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(SqliteMessageSubmissionOpenFailure::Migrate)?;
        Ok(Self { pool })
    }

    async fn claim_record(
        &self,
        claim: &MessageSubmissionClaim,
    ) -> Result<MessageSubmissionClaimOutcome, MessageStoreFailure> {
        let mut transaction = self.begin().await?;
        let insert = sqlx::query(
            "INSERT INTO message_submissions
             (submission_id, kind, fingerprint, transaction_id, state, event_id)
             VALUES (?, ?, ?, ?, 'claimed', NULL)
             ON CONFLICT(submission_id) DO NOTHING",
        )
        .bind(claim.submission_id.to_string())
        .bind(claim.kind.as_str())
        .bind(claim.fingerprint.as_bytes().to_vec())
        .bind(claim.transaction_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        let record = load_by_submission(&mut transaction, claim.submission_id).await?;
        let outcome = if insert.rows_affected() == 1 {
            MessageSubmissionClaimOutcome::Created(record)
        } else {
            ensure_same_intent(&record, claim)?;
            MessageSubmissionClaimOutcome::Existing(record)
        };
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        Ok(outcome)
    }

    async fn mark_unknown_record(
        &self,
        submission_id: MessageSubmissionId,
    ) -> Result<MessageSubmissionRecord, MessageStoreFailure> {
        let mut transaction = self.begin().await?;
        let mut record = load_by_submission(&mut transaction, submission_id).await?;
        if record.state == MessageSubmissionState::Claimed {
            write_state(
                &mut transaction,
                submission_id,
                MessageSubmissionState::SubmitUnknown,
                None,
            )
            .await?;
            record.state = MessageSubmissionState::SubmitUnknown;
        }
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        Ok(record)
    }

    async fn mark_accepted_record(
        &self,
        submission_id: MessageSubmissionId,
        event_id: &MatrixEventId,
    ) -> Result<MessageSubmissionRecord, MessageStoreFailure> {
        let mut transaction = self.begin().await?;
        let mut record = load_by_submission(&mut transaction, submission_id).await?;
        ensure_compatible_event_id(&record, event_id)?;
        if record.state != MessageSubmissionState::Bound {
            write_state(
                &mut transaction,
                submission_id,
                MessageSubmissionState::Accepted,
                Some(event_id),
            )
            .await?;
            record.state = MessageSubmissionState::Accepted;
            record.event_id = Some(event_id.clone());
        }
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        Ok(record)
    }

    async fn mark_bound_record(
        &self,
        submission_id: MessageSubmissionId,
    ) -> Result<MessageSubmissionRecord, MessageStoreFailure> {
        let mut transaction = self.begin().await?;
        let mut record = load_by_submission(&mut transaction, submission_id).await?;
        let event_id = record.event_id.as_ref().ok_or_else(corrupt_store_failure)?;
        match record.state {
            MessageSubmissionState::Accepted => {
                write_state(
                    &mut transaction,
                    submission_id,
                    MessageSubmissionState::Bound,
                    Some(event_id),
                )
                .await?;
                record.state = MessageSubmissionState::Bound;
            }
            MessageSubmissionState::Bound => {}
            MessageSubmissionState::Claimed | MessageSubmissionState::SubmitUnknown => {
                return Err(corrupt_store_failure());
            }
        }
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        Ok(record)
    }

    async fn observe_record(
        &self,
        transaction_id: &MatrixTransactionId,
        event_id: &MatrixEventId,
    ) -> Result<Option<MessageSubmissionRecord>, MessageStoreFailure> {
        let mut transaction = self.begin().await?;
        let Some(mut record) = load_by_transaction(&mut transaction, transaction_id).await? else {
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(&error))?;
            return Ok(None);
        };
        ensure_compatible_event_id(&record, event_id)?;
        if record.state != MessageSubmissionState::Bound {
            write_state(
                &mut transaction,
                record.submission_id,
                MessageSubmissionState::Accepted,
                Some(event_id),
            )
            .await?;
            record.state = MessageSubmissionState::Accepted;
            record.event_id = Some(event_id.clone());
        }
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        Ok(Some(record))
    }

    async fn begin(&self) -> Result<Transaction<'_, Sqlite>, MessageStoreFailure> {
        self.pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(&error))
    }
}

impl MessageSubmissionRepository for SqliteMessageSubmissionRepository {
    fn claim<'a>(
        &'a self,
        claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>> {
        Box::pin(async move { self.claim_record(claim).await })
    }

    fn mark_submit_unknown(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move { self.mark_unknown_record(submission_id).await })
    }

    fn mark_accepted<'a>(
        &'a self,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move { self.mark_accepted_record(submission_id, event_id).await })
    }

    fn mark_bound(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Box::pin(async move { self.mark_bound_record(submission_id).await })
    }

    fn observe_transaction<'a>(
        &'a self,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        Box::pin(async move { self.observe_record(transaction_id, event_id).await })
    }
}

async fn load_by_submission(
    transaction: &mut Transaction<'_, Sqlite>,
    submission_id: MessageSubmissionId,
) -> Result<MessageSubmissionRecord, MessageStoreFailure> {
    let row = sqlx::query(
        "SELECT submission_id, kind, fingerprint, transaction_id, state, event_id
         FROM message_submissions
         WHERE submission_id = ?",
    )
    .bind(submission_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?
    .ok_or_else(not_found_store_failure)?;
    decode_record(&row)
}

async fn load_by_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    transaction_id: &MatrixTransactionId,
) -> Result<Option<MessageSubmissionRecord>, MessageStoreFailure> {
    sqlx::query(
        "SELECT submission_id, kind, fingerprint, transaction_id, state, event_id
         FROM message_submissions
         WHERE transaction_id = ?",
    )
    .bind(transaction_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?
    .map(|row| decode_record(&row))
    .transpose()
}

async fn write_state(
    transaction: &mut Transaction<'_, Sqlite>,
    submission_id: MessageSubmissionId,
    state: MessageSubmissionState,
    event_id: Option<&MatrixEventId>,
) -> Result<(), MessageStoreFailure> {
    let result = sqlx::query(
        "UPDATE message_submissions
         SET state = ?, event_id = ?
         WHERE submission_id = ?",
    )
    .bind(state.as_str())
    .bind(event_id.map(MatrixEventId::as_str))
    .bind(submission_id.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(not_found_store_failure())
    }
}

fn decode_record(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<MessageSubmissionRecord, MessageStoreFailure> {
    let submission_value = column::<String>(row, "submission_id")?;
    let submission_id = decode_submission_id(&submission_value)?;
    let kind = decode_kind(&column::<String>(row, "kind")?)?;
    let fingerprint = decode_fingerprint(column(row, "fingerprint")?)?;
    let transaction_id = MatrixTransactionId::new(column::<String>(row, "transaction_id")?)
        .map_err(|_| corrupt_store_failure())?;
    let state = decode_state(&column::<String>(row, "state")?)?;
    let event_id = column::<Option<String>>(row, "event_id")?
        .map(MatrixEventId::new)
        .transpose()
        .map_err(|_| corrupt_store_failure())?;
    let record = MessageSubmissionRecord {
        submission_id,
        kind,
        fingerprint,
        transaction_id,
        state,
        event_id,
    };
    ensure_record_invariant(&record)?;
    Ok(record)
}

fn column<T>(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<T, MessageStoreFailure>
where
    T: for<'decode> sqlx::Decode<'decode, Sqlite> + sqlx::Type<Sqlite>,
{
    row.try_get(name).map_err(|_| corrupt_store_failure())
}

fn decode_submission_id(value: &str) -> Result<MessageSubmissionId, MessageStoreFailure> {
    let value = Uuid::parse_str(value).map_err(|_| corrupt_store_failure())?;
    if value.get_version() != Some(Version::SortRand) {
        return Err(corrupt_store_failure());
    }
    Ok(MessageSubmissionId::from_uuid(value))
}

fn decode_kind(value: &str) -> Result<MessageSubmissionKind, MessageStoreFailure> {
    match value {
        "preview" => Ok(MessageSubmissionKind::Preview),
        "replace" => Ok(MessageSubmissionKind::Replace),
        "redact" => Ok(MessageSubmissionKind::Redact),
        _ => Err(corrupt_store_failure()),
    }
}

fn decode_state(value: &str) -> Result<MessageSubmissionState, MessageStoreFailure> {
    match value {
        "claimed" => Ok(MessageSubmissionState::Claimed),
        "submit_unknown" => Ok(MessageSubmissionState::SubmitUnknown),
        "accepted" => Ok(MessageSubmissionState::Accepted),
        "bound" => Ok(MessageSubmissionState::Bound),
        _ => Err(corrupt_store_failure()),
    }
}

fn decode_fingerprint(value: Vec<u8>) -> Result<MessageSubmissionFingerprint, MessageStoreFailure> {
    let bytes = <[u8; 32]>::try_from(value).map_err(|_| corrupt_store_failure())?;
    Ok(MessageSubmissionFingerprint::from_bytes(bytes))
}

fn ensure_same_intent(
    record: &MessageSubmissionRecord,
    claim: &MessageSubmissionClaim,
) -> Result<(), MessageStoreFailure> {
    if record.kind == claim.kind
        && record.fingerprint == claim.fingerprint
        && record.transaction_id == claim.transaction_id
    {
        Ok(())
    } else {
        Err(conflict_store_failure())
    }
}

fn ensure_compatible_event_id(
    record: &MessageSubmissionRecord,
    event_id: &MatrixEventId,
) -> Result<(), MessageStoreFailure> {
    if record
        .event_id
        .as_ref()
        .is_some_and(|existing| existing != event_id)
    {
        Err(conflict_store_failure())
    } else {
        Ok(())
    }
}

fn ensure_record_invariant(record: &MessageSubmissionRecord) -> Result<(), MessageStoreFailure> {
    let expects_event = matches!(
        record.state,
        MessageSubmissionState::Accepted | MessageSubmissionState::Bound
    );
    if expects_event == record.event_id.is_some() {
        Ok(())
    } else {
        Err(corrupt_store_failure())
    }
}

fn map_sqlx_error(error: &sqlx::Error) -> MessageStoreFailure {
    let kind = match error {
        sqlx::Error::RowNotFound => MessageStoreFailureKind::NotFound,
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::TypeNotFound { .. } => MessageStoreFailureKind::Corrupt,
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            MessageStoreFailureKind::Conflict
        }
        _ => MessageStoreFailureKind::Unavailable,
    };
    MessageStoreFailure::new(kind)
}

const fn not_found_store_failure() -> MessageStoreFailure {
    MessageStoreFailure::new(MessageStoreFailureKind::NotFound)
}

const fn conflict_store_failure() -> MessageStoreFailure {
    MessageStoreFailure::new(MessageStoreFailureKind::Conflict)
}

const fn corrupt_store_failure() -> MessageStoreFailure {
    MessageStoreFailure::new(MessageStoreFailureKind::Corrupt)
}
