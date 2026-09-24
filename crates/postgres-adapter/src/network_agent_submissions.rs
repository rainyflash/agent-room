//! 网络 Agent 发言的幂等记录（ADR 0010，2-收发）：与本机 Bridge 的提交记录同一套状态，按网络 Agent
//! 分开存。同一个提交 ID 换了内容就是冲突；Matrix 已收下的不回退。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixEventId, MatrixTransactionId, NetworkAgentSubmissionClaim,
        NetworkAgentSubmissionClaimOutcome, NetworkAgentSubmissionKind,
        NetworkAgentSubmissionRecord, NetworkAgentSubmissionState, NetworkAgentSubmissionStore,
        PortFuture,
    },
};
use agent_room_domain::ids::{MessageSubmissionId, NetworkAgentId};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{corrupt_data, decode_column},
    error::map_sqlx_error,
};

const RECORD_COLUMNS: &str = "submission_id, kind, fingerprint, transaction_id, state, event_id";

impl NetworkAgentSubmissionStore for PostgresRepositories {
    fn claim<'a>(
        &'a self,
        id: NetworkAgentId,
        claim: &'a NetworkAgentSubmissionClaim,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionClaimOutcome>> {
        Box::pin(async move {
            let operation = "network_agent.submission_claim";
            let mut transaction = begin(self, operation).await?;
            let inserted = sqlx::query(
                r"INSERT INTO agent_room.network_agent_submission (
                      network_agent_id, submission_id, kind, fingerprint, transaction_id, state,
                      event_id, created_at, updated_at
                  ) VALUES (
                      $1, $2, $3, $4, $5, 'claimed', NULL,
                      to_timestamp($6::double precision / 1000.0),
                      to_timestamp($6::double precision / 1000.0)
                  )
                  ON CONFLICT (network_agent_id, submission_id) DO NOTHING",
            )
            .bind(id.as_uuid())
            .bind(claim.submission_id.as_uuid())
            .bind(claim.kind.as_str())
            .bind(claim.fingerprint.as_slice())
            .bind(claim.transaction_id.as_str())
            .bind(claim.claimed_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let record = load(&mut transaction, id, claim.submission_id, operation).await?;
            let outcome = if inserted.rows_affected() == 1 {
                NetworkAgentSubmissionClaimOutcome::Created(record)
            } else if record.kind == claim.kind
                && record.fingerprint == claim.fingerprint
                && record.transaction_id == claim.transaction_id
            {
                NetworkAgentSubmissionClaimOutcome::Existing(record)
            } else {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Conflict,
                ));
            };
            commit(transaction, operation).await?;
            Ok(outcome)
        })
    }

    fn mark_submit_unknown(
        &self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>> {
        Box::pin(async move {
            let operation = "network_agent.submission_unknown";
            let mut transaction = begin(self, operation).await?;
            let mut record = load(&mut transaction, id, submission_id, operation).await?;
            if record.state == NetworkAgentSubmissionState::Claimed {
                write_state(
                    &mut transaction,
                    id,
                    submission_id,
                    NetworkAgentSubmissionState::SubmitUnknown,
                    None,
                    operation,
                )
                .await?;
                record.state = NetworkAgentSubmissionState::SubmitUnknown;
            }
            commit(transaction, operation).await?;
            Ok(record)
        })
    }

    fn mark_accepted<'a>(
        &'a self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionRecord>> {
        Box::pin(async move {
            let operation = "network_agent.submission_accepted";
            let mut transaction = begin(self, operation).await?;
            let mut record = load(&mut transaction, id, submission_id, operation).await?;
            accept(&mut transaction, id, &mut record, event_id, operation).await?;
            commit(transaction, operation).await?;
            Ok(record)
        })
    }

    fn mark_bound(
        &self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>> {
        Box::pin(async move {
            let operation = "network_agent.submission_bound";
            let mut transaction = begin(self, operation).await?;
            let mut record = load(&mut transaction, id, submission_id, operation).await?;
            match record.state {
                NetworkAgentSubmissionState::Accepted => {
                    let event_id = record
                        .event_id
                        .clone()
                        .ok_or_else(|| corrupt_data(operation))?;
                    write_state(
                        &mut transaction,
                        id,
                        submission_id,
                        NetworkAgentSubmissionState::Bound,
                        Some(&event_id),
                        operation,
                    )
                    .await?;
                    record.state = NetworkAgentSubmissionState::Bound;
                }
                NetworkAgentSubmissionState::Bound => {}
                // 还没被 Matrix 收下就要求绑定，说明调用顺序错了。
                NetworkAgentSubmissionState::Claimed
                | NetworkAgentSubmissionState::SubmitUnknown => {
                    return Err(corrupt_data(operation));
                }
            }
            commit(transaction, operation).await?;
            Ok(record)
        })
    }

    fn observe_transaction<'a>(
        &'a self,
        id: NetworkAgentId,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentSubmissionRecord>>> {
        Box::pin(async move {
            let operation = "network_agent.submission_observe";
            let mut transaction = begin(self, operation).await?;
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {RECORD_COLUMNS} FROM agent_room.network_agent_submission \
                 WHERE network_agent_id = $1 AND transaction_id = $2 FOR UPDATE"
            )))
            .bind(id.as_uuid())
            .bind(transaction_id.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let Some(row) = row else {
                commit(transaction, operation).await?;
                return Ok(None);
            };
            let mut record = decode(&row, operation)?;
            accept(&mut transaction, id, &mut record, event_id, operation).await?;
            commit(transaction, operation).await?;
            Ok(Some(record))
        })
    }
}

/// 记为 Matrix 已收下；事件 ID 与已记下的不一致就是冲突，已经绑定的不回退。
async fn accept(
    transaction: &mut Transaction<'_, Postgres>,
    id: NetworkAgentId,
    record: &mut NetworkAgentSubmissionRecord,
    event_id: &MatrixEventId,
    operation: &'static str,
) -> RepositoryResult<()> {
    if record
        .event_id
        .as_ref()
        .is_some_and(|existing| existing != event_id)
    {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Conflict,
        ));
    }
    if record.state != NetworkAgentSubmissionState::Bound {
        write_state(
            transaction,
            id,
            record.submission_id,
            NetworkAgentSubmissionState::Accepted,
            Some(event_id),
            operation,
        )
        .await?;
        record.state = NetworkAgentSubmissionState::Accepted;
        record.event_id = Some(event_id.clone());
    }
    Ok(())
}

async fn begin<'a>(
    repositories: &'a PostgresRepositories,
    operation: &'static str,
) -> RepositoryResult<Transaction<'a, Postgres>> {
    repositories
        .pool()
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))
}

async fn commit(
    transaction: Transaction<'_, Postgres>,
    operation: &'static str,
) -> RepositoryResult<()> {
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))
}

async fn load(
    transaction: &mut Transaction<'_, Postgres>,
    id: NetworkAgentId,
    submission_id: MessageSubmissionId,
    operation: &'static str,
) -> RepositoryResult<NetworkAgentSubmissionRecord> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {RECORD_COLUMNS} FROM agent_room.network_agent_submission \
         WHERE network_agent_id = $1 AND submission_id = $2 FOR UPDATE"
    )))
    .bind(id.as_uuid())
    .bind(submission_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))?;
    decode(&row, operation)
}

async fn write_state(
    transaction: &mut Transaction<'_, Postgres>,
    id: NetworkAgentId,
    submission_id: MessageSubmissionId,
    state: NetworkAgentSubmissionState,
    event_id: Option<&MatrixEventId>,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.network_agent_submission
             SET state = $3, event_id = $4, updated_at = greatest(updated_at, now())
           WHERE network_agent_id = $1 AND submission_id = $2",
    )
    .bind(id.as_uuid())
    .bind(submission_id.as_uuid())
    .bind(state.as_str())
    .bind(event_id.map(MatrixEventId::as_str))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode(row: &PgRow, operation: &'static str) -> RepositoryResult<NetworkAgentSubmissionRecord> {
    let submission_id: uuid::Uuid = decode_column(row, "submission_id", operation)?;
    let kind: String = decode_column(row, "kind", operation)?;
    let fingerprint: Vec<u8> = decode_column(row, "fingerprint", operation)?;
    let transaction_id: String = decode_column(row, "transaction_id", operation)?;
    let state: String = decode_column(row, "state", operation)?;
    let event_id: Option<String> = decode_column(row, "event_id", operation)?;
    Ok(NetworkAgentSubmissionRecord {
        submission_id: MessageSubmissionId::from_uuid(submission_id),
        kind: NetworkAgentSubmissionKind::parse(&kind).ok_or_else(|| corrupt_data(operation))?,
        fingerprint: fingerprint
            .try_into()
            .map_err(|_| corrupt_data(operation))?,
        transaction_id: MatrixTransactionId::new(transaction_id)
            .map_err(|_| corrupt_data(operation))?,
        state: NetworkAgentSubmissionState::parse(&state).ok_or_else(|| corrupt_data(operation))?,
        event_id: event_id
            .map(MatrixEventId::new)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
    })
}
