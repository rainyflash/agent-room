use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ClaimedOutboxEvent, OutboxBacklog, OutboxClaim, OutboxFailure, OutboxFailureOutcome,
        OutboxMessage, OutboxRepository, PortFuture,
    },
};
use agent_room_domain::{ids::OutboxEventId, time::UtcMillis};
use serde_json::{Map, Value};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, error::map_sqlx_error};

impl OutboxRepository for PostgresRepositories {
    fn claim<'a>(
        &'a self,
        claim: &'a OutboxClaim,
    ) -> PortFuture<'a, RepositoryResult<Vec<ClaimedOutboxEvent>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                r"WITH claimable AS (
                    SELECT id
                    FROM agent_room.outbox_event
                    WHERE published_at IS NULL
                      AND dead_lettered_at IS NULL
                      AND next_attempt_at <= to_timestamp($1::double precision / 1000.0)
                      AND (
                          claim_expires_at IS NULL
                          OR claim_expires_at <= to_timestamp($1::double precision / 1000.0)
                      )
                    ORDER BY next_attempt_at, occurred_at, id
                    FOR UPDATE SKIP LOCKED
                    LIMIT $2
                )
                UPDATE agent_room.outbox_event AS event
                SET claimed_by = $3,
                    claim_expires_at = to_timestamp($4::double precision / 1000.0)
                FROM claimable
                WHERE event.id = claimable.id
                RETURNING event.id, event.aggregate_type, event.aggregate_id,
                    event.event_type, event.payload::text AS payload_json,
                    floor(extract(epoch FROM event.occurred_at) * 1000)::bigint AS occurred_at_ms,
                    event.attempt_count, event.claimed_by,
                    floor(extract(epoch FROM event.claim_expires_at) * 1000)::bigint
                        AS claim_expires_at_ms",
            )
            .bind(claim.claimed_at().value())
            .bind(i64::from(claim.batch_size().get()))
            .bind(claim.worker_name())
            .bind(claim.lease_expires_at().value())
            .fetch_all(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("outbox.claim", &error))?;

            rows.iter().map(decode_claimed_event).collect()
        })
    }

    fn mark_published<'a>(
        &'a self,
        event_id: OutboxEventId,
        worker_name: &'a str,
        published_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let result = sqlx::query(
                r"UPDATE agent_room.outbox_event
                   SET published_at = to_timestamp($3::double precision / 1000.0),
                       claimed_by = NULL,
                       claim_expires_at = NULL,
                       last_error_code = NULL
                   WHERE id = $1
                     AND claimed_by = $2
                     AND published_at IS NULL
                     AND dead_lettered_at IS NULL
                     AND claim_expires_at >= to_timestamp($3::double precision / 1000.0)",
            )
            .bind(event_id.as_uuid())
            .bind(worker_name)
            .bind(published_at.value())
            .execute(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("outbox.mark_published", &error))?;

            require_single_update(result.rows_affected(), "outbox.mark_published")
        })
    }

    fn record_failure<'a>(
        &'a self,
        event_id: OutboxEventId,
        worker_name: &'a str,
        failure: &'a OutboxFailure,
    ) -> PortFuture<'a, RepositoryResult<OutboxFailureOutcome>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"UPDATE agent_room.outbox_event
                   SET attempt_count = least(attempt_count + 1, 100),
                       last_error_code = $3,
                       next_attempt_at = to_timestamp($4::double precision / 1000.0),
                       dead_lettered_at = CASE
                           WHEN attempt_count + 1 >= $5
                           THEN to_timestamp($6::double precision / 1000.0)
                           ELSE NULL
                       END,
                       claimed_by = NULL,
                       claim_expires_at = NULL
                   WHERE id = $1
                     AND claimed_by = $2
                     AND published_at IS NULL
                     AND dead_lettered_at IS NULL
                     AND claim_expires_at >= to_timestamp($6::double precision / 1000.0)
                   RETURNING attempt_count, dead_lettered_at IS NOT NULL AS is_dead_lettered",
            )
            .bind(event_id.as_uuid())
            .bind(worker_name)
            .bind(failure.error_code())
            .bind(failure.next_attempt_at().value())
            .bind(i32::from(failure.max_attempts().get()))
            .bind(failure.failed_at().value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("outbox.record_failure", &error))?
            .ok_or_else(|| conflict("outbox.record_failure"))?;

            let attempt_count: i32 = row
                .try_get("attempt_count")
                .map_err(|error| map_sqlx_error("outbox.record_failure.decode", &error))?;
            let attempt_count = u16::try_from(attempt_count)
                .map_err(|_| corrupt_data("outbox.record_failure.decode"))?;
            let is_dead_lettered: bool = row
                .try_get("is_dead_lettered")
                .map_err(|error| map_sqlx_error("outbox.record_failure.decode", &error))?;

            Ok(if is_dead_lettered {
                OutboxFailureOutcome::DeadLettered { attempt_count }
            } else {
                OutboxFailureOutcome::RetryScheduled { attempt_count }
            })
        })
    }

    fn backlog(&self, now: UtcMillis) -> PortFuture<'_, RepositoryResult<OutboxBacklog>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT
                    count(*) FILTER (
                        WHERE published_at IS NULL AND dead_lettered_at IS NULL
                          AND next_attempt_at <= to_timestamp($1::double precision / 1000.0)
                          AND (claim_expires_at IS NULL
                               OR claim_expires_at <= to_timestamp($1::double precision / 1000.0))
                    ) AS ready,
                    count(*) FILTER (
                        WHERE published_at IS NULL AND dead_lettered_at IS NULL
                          AND next_attempt_at > to_timestamp($1::double precision / 1000.0)
                          AND (claim_expires_at IS NULL
                               OR claim_expires_at <= to_timestamp($1::double precision / 1000.0))
                    ) AS scheduled,
                    count(*) FILTER (
                        WHERE published_at IS NULL AND dead_lettered_at IS NULL
                          AND claim_expires_at > to_timestamp($1::double precision / 1000.0)
                    ) AS leased,
                    count(*) FILTER (WHERE dead_lettered_at IS NOT NULL) AS dead_lettered,
                    floor(extract(epoch FROM min(occurred_at) FILTER (
                        WHERE published_at IS NULL AND dead_lettered_at IS NULL
                    )) * 1000)::bigint AS oldest_pending_at_ms
                  FROM agent_room.outbox_event",
            )
            .bind(now.value())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("outbox.backlog", &error))?;

            decode_backlog(&row)
        })
    }
}

pub(crate) async fn insert_outbox_event(
    transaction: &mut Transaction<'_, Postgres>,
    event: &OutboxMessage,
) -> RepositoryResult<()> {
    let payload = serde_json::to_string(event.payload())
        .map_err(|_| corrupt_data("outbox.insert.serialize"))?;
    sqlx::query(
        r"INSERT INTO agent_room.outbox_event (
            id, aggregate_type, aggregate_id, event_type, payload,
            occurred_at, next_attempt_at
        ) VALUES (
            $1, $2, $3, $4, $5::jsonb,
            to_timestamp($6::double precision / 1000.0),
            to_timestamp($6::double precision / 1000.0)
        )",
    )
    .bind(event.id().as_uuid())
    .bind(event.aggregate_type())
    .bind(event.aggregate_id())
    .bind(event.event_type())
    .bind(payload)
    .bind(event.occurred_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("outbox.insert", &error))?;
    Ok(())
}

fn decode_claimed_event(row: &PgRow) -> RepositoryResult<ClaimedOutboxEvent> {
    let event_id: uuid::Uuid = decode_column(row, "id", "outbox.claim.decode")?;
    let aggregate_type: String = decode_column(row, "aggregate_type", "outbox.claim.decode")?;
    let aggregate_id: uuid::Uuid = decode_column(row, "aggregate_id", "outbox.claim.decode")?;
    let event_type: String = decode_column(row, "event_type", "outbox.claim.decode")?;
    let payload_json: String = decode_column(row, "payload_json", "outbox.claim.decode")?;
    let occurred_at = decode_time(row, "occurred_at_ms", "outbox.claim.decode")?;
    let attempt_count: i32 = decode_column(row, "attempt_count", "outbox.claim.decode")?;
    let attempt_count = u16::try_from(attempt_count)
        .map_err(|_| corrupt_data("outbox.claim.decode.attempt_count"))?;
    let worker_name: String = decode_column(row, "claimed_by", "outbox.claim.decode")?;
    let lease_expires_at = decode_time(row, "claim_expires_at_ms", "outbox.claim.decode")?;
    let payload: Value = serde_json::from_str(&payload_json)
        .map_err(|_| corrupt_data("outbox.claim.decode.payload"))?;
    let payload: Map<String, Value> = payload
        .as_object()
        .cloned()
        .ok_or_else(|| corrupt_data("outbox.claim.decode.payload"))?;
    let message = OutboxMessage::new(
        OutboxEventId::from_uuid(event_id),
        aggregate_type,
        aggregate_id,
        event_type,
        payload,
        occurred_at,
    )
    .map_err(|_| corrupt_data("outbox.claim.decode.message"))?;

    Ok(ClaimedOutboxEvent::restore(
        message,
        attempt_count,
        worker_name,
        lease_expires_at,
    ))
}

fn decode_backlog(row: &PgRow) -> RepositoryResult<OutboxBacklog> {
    let ready = decode_count(row, "ready")?;
    let scheduled = decode_count(row, "scheduled")?;
    let leased = decode_count(row, "leased")?;
    let dead_lettered = decode_count(row, "dead_lettered")?;
    let oldest: Option<i64> = row
        .try_get("oldest_pending_at_ms")
        .map_err(|error| map_sqlx_error("outbox.backlog.decode", &error))?;
    let oldest_pending_at = oldest
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| corrupt_data("outbox.backlog.decode"))?;
    Ok(OutboxBacklog::restore(
        ready,
        scheduled,
        leased,
        dead_lettered,
        oldest_pending_at,
    ))
}

fn decode_count(row: &PgRow, column: &'static str) -> RepositoryResult<u64> {
    let value: i64 = row
        .try_get(column)
        .map_err(|error| map_sqlx_error("outbox.backlog.decode", &error))?;
    u64::try_from(value).map_err(|_| corrupt_data("outbox.backlog.decode"))
}

fn decode_time(
    row: &PgRow,
    column: &'static str,
    operation: &'static str,
) -> RepositoryResult<UtcMillis> {
    let value: i64 = decode_column(row, column, operation)?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}

fn decode_column<T>(
    row: &PgRow,
    column: &'static str,
    operation: &'static str,
) -> RepositoryResult<T>
where
    for<'r> T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn require_single_update(rows_affected: u64, operation: &'static str) -> RepositoryResult<()> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(conflict(operation))
    }
}

fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
