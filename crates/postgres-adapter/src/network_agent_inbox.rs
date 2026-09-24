//! 网络 Agent 的房间与收件箱（ADR 0010，2-收发）：同步结果按到达顺序编号写入，
//! 只有 Agent 显式确认才往前走；确认过的直接删掉。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixEventId, MatrixSyncToken, NetworkAgentAckOutcome, NetworkAgentInboxAppend,
        NetworkAgentInboxAppendOutcome, NetworkAgentInboxChange, NetworkAgentInboxEntry,
        NetworkAgentInboxPage, NetworkAgentInboxStore, NetworkAgentRoomRecord, PortFuture,
    },
};
use agent_room_domain::{
    ids::{NetworkAgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction};

use crate::{
    PostgresRepositories,
    agents::corrupt_data,
    error::{map_domain_error, map_sqlx_error},
};

impl PostgresRepositories {
    pub(crate) async fn record_network_agent_room(
        &self,
        id: NetworkAgentId,
        room: &NetworkAgentRoomRecord,
    ) -> RepositoryResult<()> {
        let operation = "network_agent.record_room";
        sqlx::query(
            r"INSERT INTO agent_room.network_agent_room
                  (network_agent_id, matrix_room_id, catalog_entry_id, joined_at)
              VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0))
              ON CONFLICT (network_agent_id, matrix_room_id) DO NOTHING",
        )
        .bind(id.as_uuid())
        .bind(room.matrix_room_id.as_str())
        .bind(room.catalog_id.as_uuid())
        .bind(room.joined_at.value())
        .execute(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        Ok(())
    }

    pub(crate) async fn network_agent_rooms(
        &self,
        id: NetworkAgentId,
    ) -> RepositoryResult<Vec<NetworkAgentRoomRecord>> {
        let operation = "network_agent.rooms";
        let rows = sqlx::query_as::<_, (uuid::Uuid, String, i64)>(
            r"SELECT catalog_entry_id, matrix_room_id,
                     floor(extract(epoch FROM joined_at) * 1000)::bigint
                FROM agent_room.network_agent_room
               WHERE network_agent_id = $1
               ORDER BY joined_at, matrix_room_id",
        )
        .bind(id.as_uuid())
        .fetch_all(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        rows.into_iter()
            .map(|(catalog_id, matrix_room_id, joined_at)| {
                Ok(NetworkAgentRoomRecord {
                    catalog_id: RoomCatalogId::from_uuid(catalog_id),
                    matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                        .map_err(|error| map_domain_error(operation, &error))?,
                    joined_at: UtcMillis::new(joined_at)
                        .map_err(|error| map_domain_error(operation, &error))?,
                })
            })
            .collect()
    }
}

impl NetworkAgentInboxStore for PostgresRepositories {
    fn pending(
        &self,
        id: NetworkAgentId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentInboxPage>> {
        Box::pin(async move {
            let operation = "network_agent.inbox_pending";
            let (sync_token, dropped) = sqlx::query_as::<_, (Option<String>, i64)>(
                r"SELECT sync_token, dropped_messages
                    FROM agent_room.network_agent
                   WHERE id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))?;
            let rows = sqlx::query_as::<_, (i64, String, String)>(
                r"SELECT sequence, matrix_event_id, preview::text
                    FROM agent_room.network_agent_inbox
                   WHERE network_agent_id = $1
                   ORDER BY sequence
                   LIMIT $2",
            )
            .bind(id.as_uuid())
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agent_room.network_agent_inbox WHERE network_agent_id = $1",
            )
            .bind(id.as_uuid())
            .fetch_one(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(NetworkAgentInboxPage {
                sync_token: sync_token
                    .map(MatrixSyncToken::new)
                    .transpose()
                    .map_err(|_| corrupt_data(operation))?,
                entries: rows
                    .into_iter()
                    .map(|(sequence, event_id, preview)| {
                        Ok(NetworkAgentInboxEntry {
                            sequence: u64::try_from(sequence)
                                .map_err(|_| corrupt_data(operation))?,
                            event_id: MatrixEventId::new(event_id)
                                .map_err(|_| corrupt_data(operation))?,
                            preview: serde_json::from_str(&preview)
                                .map_err(|_| corrupt_data(operation))?,
                        })
                    })
                    .collect::<RepositoryResult<_>>()?,
                pending: u64::try_from(pending).map_err(|_| corrupt_data(operation))?,
                dropped: u64::try_from(dropped).map_err(|_| corrupt_data(operation))?,
            })
        })
    }

    fn append<'a>(
        &'a self,
        append: &'a NetworkAgentInboxAppend,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxAppendOutcome>> {
        Box::pin(async move {
            let operation = "network_agent.inbox_append";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            // 锁住这个网络 Agent：同一时间只有一次同步能写，位置对不上就作废。
            let (sync_token, mut sequence) = sqlx::query_as::<_, (Option<String>, i64)>(
                r"SELECT sync_token, inbox_sequence
                    FROM agent_room.network_agent
                   WHERE id = $1
                   FOR UPDATE",
            )
            .bind(append.id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))?;
            if sync_token.as_deref()
                != append
                    .expected_sync_token
                    .as_ref()
                    .map(MatrixSyncToken::as_str)
            {
                return Ok(NetworkAgentInboxAppendOutcome::Stale);
            }
            let mut appended = 0_u32;
            for change in &append.changes {
                if apply_change(&mut transaction, append, change, &mut sequence, operation).await? {
                    appended = appended.saturating_add(1);
                }
            }
            let dropped = trim_to_capacity(&mut transaction, append, operation).await?;
            sqlx::query(
                r"UPDATE agent_room.network_agent
                     SET sync_token = $2,
                         inbox_sequence = $3,
                         dropped_messages = dropped_messages + $4
                   WHERE id = $1",
            )
            .bind(append.id.as_uuid())
            .bind(append.next_sync_token.as_str())
            .bind(sequence)
            .bind(dropped)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(NetworkAgentInboxAppendOutcome::Applied { appended })
        })
    }

    fn acknowledge<'a>(
        &'a self,
        id: NetworkAgentId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>> {
        Box::pin(async move {
            let operation = "network_agent.inbox_acknowledge";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            // 与写入同一把锁，先锁网络 Agent 再动收件箱，避免互相等待。
            sqlx::query_scalar::<_, uuid::Uuid>(
                "SELECT id FROM agent_room.network_agent WHERE id = $1 FOR UPDATE",
            )
            .bind(id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))?;
            let sequence: Option<i64> = sqlx::query_scalar(
                r"SELECT sequence FROM agent_room.network_agent_inbox
                   WHERE network_agent_id = $1 AND matrix_event_id = $2",
            )
            .bind(id.as_uuid())
            .bind(event_id.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if let Some(sequence) = sequence {
                sqlx::query(
                    r"DELETE FROM agent_room.network_agent_inbox
                       WHERE network_agent_id = $1 AND sequence <= $2",
                )
                .bind(id.as_uuid())
                .bind(sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
                sqlx::query(
                    r"UPDATE agent_room.network_agent
                         SET acked_sequence = greatest(acked_sequence, $2),
                             dropped_messages = 0
                       WHERE id = $1",
                )
                .bind(id.as_uuid())
                .bind(sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agent_room.network_agent_inbox WHERE network_agent_id = $1",
            )
            .bind(id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let pending = u64::try_from(pending).map_err(|_| corrupt_data(operation))?;
            Ok(if sequence.is_some() {
                NetworkAgentAckOutcome::Acknowledged { pending }
            } else {
                NetworkAgentAckOutcome::NotPending { pending }
            })
        })
    }
}

/// 按时间线顺序写入一条变化；新消息真的写进去了（不是重复的）才返回 `true`。
async fn apply_change(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    change: &NetworkAgentInboxChange,
    sequence: &mut i64,
    operation: &'static str,
) -> RepositoryResult<bool> {
    match change {
        NetworkAgentInboxChange::Message(message) => {
            let next = sequence
                .checked_add(1)
                .ok_or_else(|| corrupt_data(operation))?;
            let inserted = sqlx::query(
                r"INSERT INTO agent_room.network_agent_inbox (
                      network_agent_id, sequence, matrix_event_id, matrix_room_id,
                      message_id, actor_key, preview, received_at
                  ) VALUES (
                      $1, $2, $3, $4, $5, $6, $7::jsonb,
                      to_timestamp($8::double precision / 1000.0)
                  )
                  ON CONFLICT (network_agent_id, matrix_event_id) DO NOTHING",
            )
            .bind(append.id.as_uuid())
            .bind(next)
            .bind(message.event_id.as_str())
            .bind(message.room_id.as_str())
            .bind(message.message_id.as_uuid())
            .bind(&message.actor_key)
            .bind(message.preview.to_string())
            .bind(append.received_at.value())
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if inserted.rows_affected() == 1 {
                *sequence = next;
                return Ok(true);
            }
        }
        NetworkAgentInboxChange::Replace {
            room_id,
            message_id,
            actor_key,
            patch,
        } => {
            sqlx::query(
                r"UPDATE agent_room.network_agent_inbox
                     SET preview = preview || $5::jsonb
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
            )
            .bind(append.id.as_uuid())
            .bind(room_id.as_str())
            .bind(message_id.as_uuid())
            .bind(actor_key)
            .bind(patch.to_string())
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        }
        NetworkAgentInboxChange::Redact {
            room_id,
            message_id,
            actor_key,
        } => {
            sqlx::query(
                r"DELETE FROM agent_room.network_agent_inbox
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
            )
            .bind(append.id.as_uuid())
            .bind(room_id.as_str())
            .bind(message_id.as_uuid())
            .bind(actor_key)
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        }
    }
    Ok(false)
}

/// 没确认的超过上限时丢掉最早的，返回丢了几条。
async fn trim_to_capacity(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    operation: &'static str,
) -> RepositoryResult<i64> {
    let dropped = sqlx::query(
        r"DELETE FROM agent_room.network_agent_inbox
           WHERE network_agent_id = $1
             AND sequence IN (
                 SELECT sequence FROM agent_room.network_agent_inbox
                  WHERE network_agent_id = $1
                  ORDER BY sequence DESC
                 OFFSET $2
             )",
    )
    .bind(append.id.as_uuid())
    .bind(i64::from(append.capacity))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .rows_affected();
    i64::try_from(dropped).map_err(|_| corrupt_data(operation))
}
