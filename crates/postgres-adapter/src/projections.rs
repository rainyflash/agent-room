use std::collections::{BTreeMap, BTreeSet};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixMembership, MatrixProjectionBatch, MatrixProjectionEvent, MatrixProjectionEventKind,
        MatrixProjectionRebuild, MatrixProjectionStore, MembershipProjectionLookup, PortFuture,
        ProjectionApplyOutcome, ProjectionCursor, ProjectionHealth, ProjectionHealthReport,
    },
};
use agent_room_domain::{
    ids::{AgentId, RoomInstanceId},
    time::UtcMillis,
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, error::map_sqlx_error};

impl MatrixProjectionStore for PostgresRepositories {
    fn apply<'a>(
        &'a self,
        batch: &'a MatrixProjectionBatch,
    ) -> PortFuture<'a, RepositoryResult<ProjectionApplyOutcome>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error("projection.apply.begin", &error))?;
            let result = apply_incremental(&mut transaction, batch).await;
            finish_transaction(transaction, result, "projection.apply").await
        })
    }

    fn rebuild<'a>(
        &'a self,
        rebuild: &'a MatrixProjectionRebuild,
    ) -> PortFuture<'a, RepositoryResult<ProjectionApplyOutcome>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error("projection.rebuild.begin", &error))?;
            let result = rebuild_snapshot(&mut transaction, rebuild).await;
            finish_transaction(transaction, result, "projection.rebuild").await
        })
    }

    fn cursor<'a>(
        &'a self,
        consumer_name: &'a str,
    ) -> PortFuture<'a, RepositoryResult<Option<ProjectionCursor>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT sync_token, last_event_id, health_state,
                    floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
                    version
                  FROM agent_room.matrix_projection_cursor
                  WHERE consumer_name = $1",
            )
            .bind(consumer_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("projection.cursor", &error))?;

            row.map(|row| decode_cursor(&row, "projection.cursor.decode"))
                .transpose()
        })
    }

    fn membership<'a>(
        &'a self,
        consumer_name: &'a str,
        room_instance_id: RoomInstanceId,
        agent_id: AgentId,
    ) -> PortFuture<'a, RepositoryResult<Option<MembershipProjectionLookup>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT cursor.health_state,
                    floor(extract(epoch FROM cursor.updated_at) * 1000)::bigint
                        AS cursor_updated_at_ms,
                    membership.matrix_membership,
                    membership.power_level,
                    floor(extract(epoch FROM membership.projected_at) * 1000)::bigint
                        AS membership_projected_at_ms
                  FROM agent_room.matrix_projection_cursor AS cursor
                  LEFT JOIN agent_room.room_membership_projection AS membership
                    ON membership.room_instance_id = $2 AND membership.agent_id = $3
                  WHERE cursor.consumer_name = $1",
            )
            .bind(consumer_name)
            .bind(room_instance_id.as_uuid())
            .bind(agent_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("projection.membership", &error))?;

            row.map(|row| decode_membership_lookup(&row)).transpose()
        })
    }

    fn report_health<'a>(
        &'a self,
        report: &'a ProjectionHealthReport,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let result = sqlx::query(
                r"UPDATE agent_room.matrix_projection_cursor
                   SET health_state = $2,
                       last_error_code = $3,
                       updated_at = to_timestamp($4::double precision / 1000.0),
                       version = version + 1
                   WHERE consumer_name = $1
                     AND updated_at <= to_timestamp($4::double precision / 1000.0)",
            )
            .bind(report.consumer_name())
            .bind(report.health().as_str())
            .bind(report.error_code())
            .bind(report.observed_at().value())
            .execute(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("projection.report_health", &error))?;

            require_single_update(result.rows_affected(), "projection.report_health")
        })
    }
}

async fn apply_incremental(
    transaction: &mut Transaction<'_, Postgres>,
    batch: &MatrixProjectionBatch,
) -> RepositoryResult<ProjectionApplyOutcome> {
    acquire_projection_lock(transaction, batch.consumer_name()).await?;
    let cursor = load_cursor_for_update(transaction, batch.consumer_name()).await?;

    if cursor
        .as_ref()
        .is_some_and(|current| current.sync_token() == batch.next_sync_token())
    {
        verify_replayed_batch(transaction, batch.consumer_name(), batch.events()).await?;
        let duplicates = u32::try_from(batch.events().len())
            .map_err(|_| corrupt_data("projection.apply.replay_count"))?;
        return Ok(ProjectionApplyOutcome::Replayed { duplicates });
    }

    verify_expected_cursor(cursor.as_ref(), batch)?;
    if cursor
        .as_ref()
        .is_some_and(|current| batch.projected_at() < current.updated_at())
    {
        return Err(conflict("projection.apply.projected_at"));
    }

    let counts = apply_events(
        transaction,
        batch.consumer_name(),
        batch.events(),
        batch.projected_at(),
    )
    .await?;
    store_cursor(
        transaction,
        batch.consumer_name(),
        batch.next_sync_token(),
        batch.events().last().map(MatrixProjectionEvent::event_id),
        batch.projected_at(),
    )
    .await?;

    Ok(ProjectionApplyOutcome::Applied {
        new_events: counts.new_events,
        duplicates: counts.duplicates,
    })
}

async fn rebuild_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    rebuild: &MatrixProjectionRebuild,
) -> RepositoryResult<ProjectionApplyOutcome> {
    acquire_projection_lock(transaction, rebuild.consumer_name()).await?;
    sqlx::query("DELETE FROM agent_room.matrix_projection_event_receipt WHERE consumer_name = $1")
        .bind(rebuild.consumer_name())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("projection.rebuild.receipts", &error))?;
    sqlx::query("DELETE FROM agent_room.room_membership_projection")
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("projection.rebuild.memberships", &error))?;
    sqlx::query(
        r"UPDATE agent_room.room_instance
           SET member_count_projection = 0,
               activity_score = 0,
               updated_at = greatest(
                   updated_at,
                   to_timestamp($1::double precision / 1000.0)
               ),
               version = version + 1",
    )
    .bind(rebuild.projected_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.rebuild.rooms", &error))?;
    sqlx::query("DELETE FROM agent_room.matrix_projection_cursor WHERE consumer_name = $1")
        .bind(rebuild.consumer_name())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("projection.rebuild.cursor", &error))?;

    let counts = apply_events(
        transaction,
        rebuild.consumer_name(),
        rebuild.events(),
        rebuild.projected_at(),
    )
    .await?;
    store_cursor(
        transaction,
        rebuild.consumer_name(),
        rebuild.next_sync_token(),
        rebuild.events().last().map(MatrixProjectionEvent::event_id),
        rebuild.projected_at(),
    )
    .await?;

    Ok(ProjectionApplyOutcome::Applied {
        new_events: counts.new_events,
        duplicates: counts.duplicates,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppliedEventCounts {
    new_events: u32,
    duplicates: u32,
}

async fn apply_events(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
    events: &[MatrixProjectionEvent],
    projected_at: UtcMillis,
) -> RepositoryResult<AppliedEventCounts> {
    let mut touched_rooms = BTreeSet::new();
    let mut activity_by_room = BTreeMap::<RoomInstanceId, u64>::new();
    let mut new_events = 0_u32;
    let mut duplicates = 0_u32;

    for event in events {
        if insert_receipt(transaction, consumer_name, event, projected_at).await? {
            new_events = new_events
                .checked_add(1)
                .ok_or_else(|| corrupt_data("projection.apply.event_count"))?;
            touched_rooms.insert(event.kind().room_instance_id());
            match event.kind() {
                MatrixProjectionEventKind::MembershipChanged {
                    room_instance_id,
                    agent_id,
                    membership,
                    power_level,
                } => {
                    upsert_membership(
                        transaction,
                        room_instance_id,
                        agent_id,
                        membership,
                        power_level,
                        event.event_id(),
                        projected_at,
                    )
                    .await?;
                }
                MatrixProjectionEventKind::ActivityObserved {
                    room_instance_id,
                    score,
                } => {
                    let total = activity_by_room.entry(room_instance_id).or_default();
                    *total = total
                        .checked_add(u64::from(score.value()))
                        .ok_or_else(|| corrupt_data("projection.apply.activity_overflow"))?;
                }
            }
        } else {
            duplicates = duplicates
                .checked_add(1)
                .ok_or_else(|| corrupt_data("projection.apply.duplicate_count"))?;
        }
    }

    for room_id in touched_rooms {
        refresh_room_projection(
            transaction,
            room_id,
            activity_by_room.get(&room_id).copied().unwrap_or_default(),
            projected_at,
        )
        .await?;
    }

    Ok(AppliedEventCounts {
        new_events,
        duplicates,
    })
}

async fn insert_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
    event: &MatrixProjectionEvent,
    processed_at: UtcMillis,
) -> RepositoryResult<bool> {
    let inserted: Option<i32> = sqlx::query_scalar(
        r"INSERT INTO agent_room.matrix_projection_event_receipt (
            consumer_name, event_id, event_digest, event_kind, processed_at
        ) VALUES (
            $1, $2, $3, $4, to_timestamp($5::double precision / 1000.0)
        )
        ON CONFLICT (consumer_name, event_id) DO NOTHING
        RETURNING 1",
    )
    .bind(consumer_name)
    .bind(event.event_id())
    .bind(&event.event_digest()[..])
    .bind(event.kind().event_kind())
    .bind(processed_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.receipt.insert", &error))?;

    if inserted.is_some() {
        return Ok(true);
    }
    verify_receipt(transaction, consumer_name, event).await?;
    Ok(false)
}

async fn verify_replayed_batch(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
    events: &[MatrixProjectionEvent],
) -> RepositoryResult<()> {
    for event in events {
        verify_receipt(transaction, consumer_name, event).await?;
    }
    Ok(())
}

async fn verify_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
    event: &MatrixProjectionEvent,
) -> RepositoryResult<()> {
    let row = sqlx::query(
        r"SELECT event_digest, event_kind
          FROM agent_room.matrix_projection_event_receipt
          WHERE consumer_name = $1 AND event_id = $2",
    )
    .bind(consumer_name)
    .bind(event.event_id())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.receipt.verify", &error))?
    .ok_or_else(|| corrupt_data("projection.receipt.missing"))?;
    let digest: Vec<u8> = row
        .try_get("event_digest")
        .map_err(|error| map_sqlx_error("projection.receipt.decode", &error))?;
    let event_kind: String = row
        .try_get("event_kind")
        .map_err(|error| map_sqlx_error("projection.receipt.decode", &error))?;
    if digest != event.event_digest()[..] || event_kind != event.kind().event_kind() {
        return Err(corrupt_data("projection.receipt.digest_mismatch"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upsert_membership(
    transaction: &mut Transaction<'_, Postgres>,
    room_instance_id: RoomInstanceId,
    agent_id: AgentId,
    membership: MatrixMembership,
    power_level: i16,
    event_id: &str,
    projected_at: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.room_membership_projection (
            room_instance_id, agent_id, matrix_membership, power_level,
            last_event_id, projected_at
        ) VALUES (
            $1, $2, $3, $4, $5, to_timestamp($6::double precision / 1000.0)
        )
        ON CONFLICT (room_instance_id, agent_id) DO UPDATE
        SET matrix_membership = excluded.matrix_membership,
            power_level = excluded.power_level,
            last_event_id = excluded.last_event_id,
            projected_at = excluded.projected_at",
    )
    .bind(room_instance_id.as_uuid())
    .bind(agent_id.as_uuid())
    .bind(membership.as_str())
    .bind(i32::from(power_level))
    .bind(event_id)
    .bind(projected_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.membership.upsert", &error))?;
    Ok(())
}

async fn refresh_room_projection(
    transaction: &mut Transaction<'_, Postgres>,
    room_instance_id: RoomInstanceId,
    activity_score_millis: u64,
    projected_at: UtcMillis,
) -> RepositoryResult<()> {
    let activity_score_millis = i64::try_from(activity_score_millis)
        .map_err(|_| corrupt_data("projection.room.activity"))?;
    let result = sqlx::query(
        r"UPDATE agent_room.room_instance AS room
           SET member_count_projection = (
                   SELECT count(*)::integer
                   FROM agent_room.room_membership_projection AS membership
                   WHERE membership.room_instance_id = room.id
                     AND membership.matrix_membership = 'join'
               ),
               activity_score = activity_score + ($2::numeric / 1000),
               updated_at = greatest(
                   updated_at,
                   to_timestamp($3::double precision / 1000.0)
               ),
               version = version + 1
           WHERE room.id = $1",
    )
    .bind(room_instance_id.as_uuid())
    .bind(activity_score_millis)
    .bind(projected_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.room.refresh", &error))?;
    require_single_update(result.rows_affected(), "projection.room.refresh")
}

async fn acquire_projection_lock(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
) -> RepositoryResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(consumer_name)
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("projection.lock", &error))?;
    Ok(())
}

async fn load_cursor_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
) -> RepositoryResult<Option<ProjectionCursor>> {
    let row = sqlx::query(
        r"SELECT sync_token, last_event_id, health_state,
            floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
            version
          FROM agent_room.matrix_projection_cursor
          WHERE consumer_name = $1
          FOR UPDATE",
    )
    .bind(consumer_name)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.cursor.lock", &error))?;
    row.map(|row| decode_cursor(&row, "projection.cursor.lock.decode"))
        .transpose()
}

fn verify_expected_cursor(
    current: Option<&ProjectionCursor>,
    batch: &MatrixProjectionBatch,
) -> RepositoryResult<()> {
    let matches = match (current, batch.expected_sync_token()) {
        (None, None) => true,
        (Some(current), Some(expected)) => current.sync_token() == expected,
        (None, Some(_)) | (Some(_), None) => false,
    };
    if matches {
        Ok(())
    } else {
        Err(conflict("projection.apply.cursor_conflict"))
    }
}

async fn store_cursor(
    transaction: &mut Transaction<'_, Postgres>,
    consumer_name: &str,
    next_sync_token: &str,
    last_event_id: Option<&str>,
    projected_at: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.matrix_projection_cursor (
            consumer_name, sync_token, last_event_id, health_state,
            last_error_code, updated_at, version
        ) VALUES (
            $1, $2, $3, 'healthy', NULL,
            to_timestamp($4::double precision / 1000.0), 0
        )
        ON CONFLICT (consumer_name) DO UPDATE
        SET sync_token = excluded.sync_token,
            last_event_id = coalesce(excluded.last_event_id,
                                     matrix_projection_cursor.last_event_id),
            health_state = 'healthy',
            last_error_code = NULL,
            updated_at = excluded.updated_at,
            version = matrix_projection_cursor.version + 1",
    )
    .bind(consumer_name)
    .bind(next_sync_token)
    .bind(last_event_id)
    .bind(projected_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("projection.cursor.store", &error))?;
    Ok(())
}

fn decode_cursor(row: &PgRow, operation: &'static str) -> RepositoryResult<ProjectionCursor> {
    let sync_token: String = decode_column(row, "sync_token", operation)?;
    let last_event_id: Option<String> = decode_column(row, "last_event_id", operation)?;
    let health_state: String = decode_column(row, "health_state", operation)?;
    let health =
        ProjectionHealth::try_from(health_state.as_str()).map_err(|_| corrupt_data(operation))?;
    let updated_at = decode_time(row, "updated_at_ms", operation)?;
    let version: i64 = decode_column(row, "version", operation)?;
    let version = u64::try_from(version).map_err(|_| corrupt_data(operation))?;
    Ok(ProjectionCursor::restore(
        sync_token,
        last_event_id,
        health,
        updated_at,
        version,
    ))
}

fn decode_membership_lookup(row: &PgRow) -> RepositoryResult<MembershipProjectionLookup> {
    let health_state: String = decode_column(row, "health_state", "projection.membership.decode")?;
    let health = ProjectionHealth::try_from(health_state.as_str())
        .map_err(|_| corrupt_data("projection.membership.decode"))?;
    let cursor_updated_at =
        decode_time(row, "cursor_updated_at_ms", "projection.membership.decode")?;
    let membership: Option<String> =
        decode_column(row, "matrix_membership", "projection.membership.decode")?;
    let membership = membership
        .as_deref()
        .map(MatrixMembership::try_from)
        .transpose()
        .map_err(|_| corrupt_data("projection.membership.decode"))?;
    let power_level: Option<i32> =
        decode_column(row, "power_level", "projection.membership.decode")?;
    let power_level = power_level
        .map(i16::try_from)
        .transpose()
        .map_err(|_| corrupt_data("projection.membership.decode"))?;
    let membership_projected_at_ms: Option<i64> = decode_column(
        row,
        "membership_projected_at_ms",
        "projection.membership.decode",
    )?;
    let membership_projected_at = membership_projected_at_ms
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| corrupt_data("projection.membership.decode"))?;
    Ok(MembershipProjectionLookup::restore(
        membership,
        power_level,
        membership_projected_at,
        cursor_updated_at,
        health,
    ))
}

async fn finish_transaction<T>(
    transaction: Transaction<'_, Postgres>,
    result: RepositoryResult<T>,
    operation: &'static str,
) -> RepositoryResult<T> {
    match result {
        Ok(value) => {
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(value)
        }
        Err(original) => {
            transaction
                .rollback()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Err(original)
        }
    }
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
