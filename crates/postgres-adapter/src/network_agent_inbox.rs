//! 网络 Agent 的房间与收件箱（ADR 0010，2-收发）：同步结果按到达顺序编号写入，
//! 只有 Agent 显式确认才往前走；确认过的直接删掉。可以只读、只确认一个房间的
//! （`specs/agent-reading/design.md` 第 5 步）。
//!
//! 同一次写入也记进消息记录（`network_agent_message`）：每个房间留最近几百条，确认过的、
//! Agent 自己发的也在，按需查看时从那里读（见 `network_agent_history.rs`）。
//!
//! 凭口令进的私人房间里加入之前解不开的一段，先记在房间上（`before_join_gap_status`），再挂到这个
//! 房间之后第一条进收件箱的消息上；每个房间只说一次。

use std::collections::HashSet;

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixEventId, MatrixRoomId, MatrixSyncToken, NetworkAgentAckOutcome,
        NetworkAgentGapReason, NetworkAgentInboxAppend, NetworkAgentInboxAppendOutcome,
        NetworkAgentInboxChange, NetworkAgentInboxEntry, NetworkAgentInboxMessage,
        NetworkAgentInboxPage, NetworkAgentInboxStore, NetworkAgentRoomRecord,
        NetworkAgentTimelineGap, PortFuture,
    },
};
use agent_room_domain::{
    ids::{NetworkAgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::{PgExecutor, Postgres, Transaction};

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
        let rows = sqlx::query_as::<_, (uuid::Uuid, String, i64, Option<String>)>(
            r"SELECT room.catalog_entry_id, room.matrix_room_id,
                     floor(extract(epoch FROM room.joined_at) * 1000)::bigint,
                     catalog.name
                FROM agent_room.network_agent_room room
                LEFT JOIN agent_room.room_catalog_entry catalog
                  ON catalog.id = room.catalog_entry_id
               WHERE room.network_agent_id = $1
               ORDER BY room.joined_at, room.matrix_room_id",
        )
        .bind(id.as_uuid())
        .fetch_all(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        rows.into_iter()
            .map(|(catalog_id, matrix_room_id, joined_at, name)| {
                Ok(NetworkAgentRoomRecord {
                    catalog_id: RoomCatalogId::from_uuid(catalog_id),
                    matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                        .map_err(|error| map_domain_error(operation, &error))?,
                    joined_at: UtcMillis::new(joined_at)
                        .map_err(|error| map_domain_error(operation, &error))?,
                    name,
                })
            })
            .collect()
    }
}

impl NetworkAgentInboxStore for PostgresRepositories {
    fn pending<'a>(
        &'a self,
        id: NetworkAgentId,
        room: Option<&'a MatrixRoomId>,
        limit: u16,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxPage>> {
        Box::pin(async move {
            let operation = "network_agent.inbox_pending";
            let room = room.map(MatrixRoomId::as_str);
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
            let rows = sqlx::query_as::<_, InboxRow>(
                r"SELECT sequence, matrix_event_id, matrix_room_id, preview::text,
                         (extract(epoch FROM received_at) * 1000)::bigint,
                         before_join_gap, gap_reason, gap_after_event_id
                    FROM agent_room.network_agent_inbox
                   WHERE network_agent_id = $1
                     AND ($3::text IS NULL OR matrix_room_id = $3)
                   ORDER BY sequence
                   LIMIT $2",
            )
            .bind(id.as_uuid())
            .bind(i64::from(limit))
            .bind(room)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let pending = count_pending(self.pool(), id, room, operation).await?;
            Ok(NetworkAgentInboxPage {
                sync_token: sync_token
                    .map(MatrixSyncToken::new)
                    .transpose()
                    .map_err(|_| corrupt_data(operation))?,
                entries: rows
                    .into_iter()
                    .map(|row| inbox_entry(row).ok_or_else(|| corrupt_data(operation)))
                    .collect::<RepositoryResult<_>>()?,
                pending,
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
            // 锁住这个网络 Agent：同一时间只有一次同步能写，位置对不上就作废。停用以后不再写：
            // 停用前就开始的长轮询可能这时才同步完，它的消息在记下离开房间时和别的一起删掉。
            let (sync_token, mut sequence, disabled) =
                sqlx::query_as::<_, (Option<String>, i64, bool)>(
                    r"SELECT sync_token, inbox_sequence, status = 'disabled'
                        FROM agent_room.network_agent
                       WHERE id = $1
                       FOR UPDATE",
                )
                .bind(append.id.as_uuid())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
                .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))?;
            if disabled
                || sync_token.as_deref()
                    != append
                        .expected_sync_token
                        .as_ref()
                        .map(MatrixSyncToken::as_str)
            {
                return Ok(NetworkAgentInboxAppendOutcome::Stale);
            }
            let mut before_join = BeforeJoinGaps::load(&mut transaction, append, operation).await?;
            let mut appended = 0_u32;
            for change in &append.changes {
                if apply_change(
                    &mut transaction,
                    append,
                    change,
                    &mut sequence,
                    &mut before_join,
                    operation,
                )
                .await?
                {
                    appended = appended.saturating_add(1);
                }
            }
            before_join
                .record_reported(&mut transaction, append, operation)
                .await?;
            let dropped = trim_to_capacity(&mut transaction, append, operation).await?;
            trim_history(&mut transaction, append, operation).await?;
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
        room: Option<&'a MatrixRoomId>,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>> {
        Box::pin(async move {
            let operation = "network_agent.inbox_acknowledge";
            let room = room.map(MatrixRoomId::as_str);
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
                   WHERE network_agent_id = $1 AND matrix_event_id = $2
                     AND ($3::text IS NULL OR matrix_room_id = $3)",
            )
            .bind(id.as_uuid())
            .bind(event_id.as_str())
            .bind(room)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if let Some(sequence) = sequence {
                sqlx::query(
                    r"DELETE FROM agent_room.network_agent_inbox
                       WHERE network_agent_id = $1 AND sequence <= $2
                         AND ($3::text IS NULL OR matrix_room_id = $3)",
                )
                .bind(id.as_uuid())
                .bind(sequence)
                .bind(room)
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
                // 只确认一个房间时，别的房间还有更早的没确认：整体的位置不动。
                sqlx::query(
                    r"UPDATE agent_room.network_agent
                         SET acked_sequence = CASE WHEN $3::text IS NULL
                                 THEN greatest(acked_sequence, $2) ELSE acked_sequence END,
                             dropped_messages = 0
                       WHERE id = $1",
                )
                .bind(id.as_uuid())
                .bind(sequence)
                .bind(room)
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            let pending = count_pending(&mut *transaction, id, room, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(if sequence.is_some() {
                NetworkAgentAckOutcome::Acknowledged { pending }
            } else {
                NetworkAgentAckOutcome::NotPending { pending }
            })
        })
    }
}

/// 收件箱的一行：编号、事件、房间、预览、收到的时间、前面有没有加入之前解不开的一段、
/// 前面补不回来的一段（原因、之前最后一条）。
type InboxRow = (
    i64,
    String,
    String,
    String,
    i64,
    bool,
    Option<String>,
    Option<String>,
);

fn inbox_entry(
    (sequence, event_id, room_id, preview, received_at, before_join, gap_reason, gap_after): InboxRow,
) -> Option<NetworkAgentInboxEntry> {
    let mut gaps = Vec::new();
    if before_join {
        gaps.push(NetworkAgentTimelineGap {
            after_event_id: None,
            reason: NetworkAgentGapReason::UndecryptableBeforeJoin,
        });
    }
    if let Some(reason) = gap_reason {
        gaps.push(NetworkAgentTimelineGap {
            after_event_id: gap_after.map(MatrixEventId::new).transpose().ok()?,
            reason: NetworkAgentGapReason::parse(&reason)?,
        });
    }
    Some(NetworkAgentInboxEntry {
        sequence: u64::try_from(sequence).ok()?,
        event_id: MatrixEventId::new(event_id).ok()?,
        room_id: MatrixRoomId::new(room_id).ok()?,
        preview: serde_json::from_str(&preview).ok()?,
        received_at: UtcMillis::new(received_at).ok()?,
        gaps,
    })
}

/// 加入之前解不开、还没告诉 Agent 的房间（`before_join_gap_status = 'pending'`）。这一批新见到的
/// 先记上；挂到这个房间第一条进收件箱的消息上以后改记 `reported`，以后不再说。
struct BeforeJoinGaps {
    pending: HashSet<String>,
    reported: Vec<String>,
}

impl BeforeJoinGaps {
    async fn load(
        transaction: &mut Transaction<'_, Postgres>,
        append: &NetworkAgentInboxAppend,
        operation: &'static str,
    ) -> RepositoryResult<Self> {
        let mut gaps = Self {
            pending: HashSet::new(),
            reported: Vec::new(),
        };
        if !append.undecryptable_before_join.is_empty() {
            let rooms: Vec<&str> = append
                .undecryptable_before_join
                .iter()
                .map(MatrixRoomId::as_str)
                .collect();
            sqlx::query(
                r"UPDATE agent_room.network_agent_room
                     SET before_join_gap_status = 'pending'
                   WHERE network_agent_id = $1 AND matrix_room_id = ANY($2)
                     AND before_join_gap_status IS NULL",
            )
            .bind(append.id.as_uuid())
            .bind(&rooms)
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        }
        // 这一批没有要进收件箱的消息，就没处可挂，不用读。
        let for_inbox = append.changes.iter().any(|change| {
            matches!(change, NetworkAgentInboxChange::Message(message) if !message.from_me)
        });
        if for_inbox {
            let pending: Vec<String> = sqlx::query_scalar(
                r"SELECT matrix_room_id FROM agent_room.network_agent_room
                   WHERE network_agent_id = $1 AND before_join_gap_status = 'pending'",
            )
            .bind(append.id.as_uuid())
            .fetch_all(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            gaps.pending.extend(pending);
        }
        Ok(gaps)
    }

    /// 这个房间还欠着加入之前那一段。
    fn owes(&self, room_id: &MatrixRoomId) -> bool {
        self.pending.contains(room_id.as_str())
    }

    /// 已经挂到这个房间的一条消息上了。
    fn told(&mut self, room_id: &MatrixRoomId) {
        if self.pending.remove(room_id.as_str()) {
            self.reported.push(room_id.as_str().to_owned());
        }
    }

    async fn record_reported(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        append: &NetworkAgentInboxAppend,
        operation: &'static str,
    ) -> RepositoryResult<()> {
        if self.reported.is_empty() {
            return Ok(());
        }
        sqlx::query(
            r"UPDATE agent_room.network_agent_room
                 SET before_join_gap_status = 'reported'
               WHERE network_agent_id = $1 AND matrix_room_id = ANY($2)",
        )
        .bind(append.id.as_uuid())
        .bind(&self.reported)
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        Ok(())
    }
}

/// 按时间线顺序写入一条变化；新消息真的进了收件箱（不是重复的、不是自己发的）才返回 `true`。
async fn apply_change(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    change: &NetworkAgentInboxChange,
    sequence: &mut i64,
    before_join: &mut BeforeJoinGaps,
    operation: &'static str,
) -> RepositoryResult<bool> {
    match change {
        NetworkAgentInboxChange::Message(message) => {
            let next = sequence
                .checked_add(1)
                .ok_or_else(|| corrupt_data(operation))?;
            // 先记进消息记录：记过的是重复同步到的，收件箱也不再写，确认过的不会再交一遍。
            if !record_message(transaction, append, message, next, operation).await? {
                return Ok(false);
            }
            *sequence = next;
            if message.from_me {
                return Ok(false);
            }
            let owed = before_join.owes(&message.room_id);
            let inserted =
                insert_inbox(transaction, append, message, next, owed, operation).await?;
            if inserted && owed {
                before_join.told(&message.room_id);
            }
            return Ok(inserted);
        }
        NetworkAgentInboxChange::Replace {
            room_id,
            message_id,
            actor_key,
            patch,
        } => {
            // 收件箱和消息记录里的同一条一起改。
            for statement in [
                r"UPDATE agent_room.network_agent_inbox
                     SET preview = preview || $5::jsonb
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
                r"UPDATE agent_room.network_agent_message
                     SET preview = preview || $5::jsonb
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
            ] {
                sqlx::query(statement)
                    .bind(append.id.as_uuid())
                    .bind(room_id.as_str())
                    .bind(message_id.as_uuid())
                    .bind(actor_key)
                    .bind(patch.to_string())
                    .execute(&mut **transaction)
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            }
        }
        NetworkAgentInboxChange::Redact {
            room_id,
            message_id,
            actor_key,
        } => {
            for statement in [
                r"DELETE FROM agent_room.network_agent_inbox
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
                r"DELETE FROM agent_room.network_agent_message
                   WHERE network_agent_id = $1 AND matrix_room_id = $2
                     AND message_id = $3 AND actor_key = $4",
            ] {
                sqlx::query(statement)
                    .bind(append.id.as_uuid())
                    .bind(room_id.as_str())
                    .bind(message_id.as_uuid())
                    .bind(actor_key)
                    .execute(&mut **transaction)
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            }
        }
    }
    Ok(false)
}

/// 写进收件箱；前面少了一段补不回来的、或者加入之前解不开的（`before_join`），记在这一条上。
async fn insert_inbox(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    message: &NetworkAgentInboxMessage,
    sequence: i64,
    before_join: bool,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let gap = message.gap.as_ref();
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.network_agent_inbox (
              network_agent_id, sequence, matrix_event_id, matrix_room_id,
              message_id, actor_key, preview, received_at, gap_reason, gap_after_event_id,
              before_join_gap
          ) VALUES (
              $1, $2, $3, $4, $5, $6, $7::jsonb,
              to_timestamp($8::double precision / 1000.0), $9, $10, $11
          )
          ON CONFLICT (network_agent_id, matrix_event_id) DO NOTHING",
    )
    .bind(append.id.as_uuid())
    .bind(sequence)
    .bind(message.event_id.as_str())
    .bind(message.room_id.as_str())
    .bind(message.message_id.as_uuid())
    .bind(&message.actor_key)
    .bind(message.preview.to_string())
    .bind(append.received_at.value())
    .bind(gap.map(|gap| gap.reason.as_str()))
    .bind(gap.and_then(|gap| gap.after_event_id.as_ref().map(MatrixEventId::as_str)))
    .bind(before_join)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(inserted.rows_affected() == 1)
}

/// 记进消息记录；这个事件已经记过时返回 `false`。
async fn record_message(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    message: &NetworkAgentInboxMessage,
    sequence: i64,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let recorded = sqlx::query(
        r"INSERT INTO agent_room.network_agent_message (
              network_agent_id, sequence, matrix_event_id, matrix_room_id, message_id,
              actor_key, actor_matrix_user_id, actor_name_folded, mentions_me, preview,
              received_at
          ) VALUES (
              $1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb,
              to_timestamp($11::double precision / 1000.0)
          )
          ON CONFLICT (network_agent_id, matrix_event_id) DO NOTHING",
    )
    .bind(append.id.as_uuid())
    .bind(sequence)
    .bind(message.event_id.as_str())
    .bind(message.room_id.as_str())
    .bind(message.message_id.as_uuid())
    .bind(&message.actor_key)
    .bind(&message.actor.matrix_user_id)
    .bind(&message.actor.name_folded)
    .bind(message.mentions_me)
    .bind(message.preview.to_string())
    .bind(append.received_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(recorded.rows_affected() == 1)
}

/// 一个房间里没确认的超过上限时丢掉这个房间最早的，返回一共丢了几条。
async fn trim_to_capacity(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    operation: &'static str,
) -> RepositoryResult<i64> {
    let dropped = sqlx::query(
        r"DELETE FROM agent_room.network_agent_inbox inbox
           USING (
               SELECT sequence
                 FROM (SELECT sequence,
                              row_number() OVER (
                                  PARTITION BY matrix_room_id ORDER BY sequence DESC
                              ) AS newer
                         FROM agent_room.network_agent_inbox
                        WHERE network_agent_id = $1) ranked
                WHERE ranked.newer > $2
           ) excess
           WHERE inbox.network_agent_id = $1 AND inbox.sequence = excess.sequence",
    )
    .bind(append.id.as_uuid())
    .bind(i64::from(append.capacity))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .rows_affected();
    i64::try_from(dropped).map_err(|_| corrupt_data(operation))
}

/// 这次写到的房间，消息记录只留最近的 `history_capacity` 条，更早的删掉。
async fn trim_history(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    operation: &'static str,
) -> RepositoryResult<()> {
    let mut rooms: Vec<&str> = append
        .changes
        .iter()
        .filter_map(|change| match change {
            NetworkAgentInboxChange::Message(message) => Some(message.room_id.as_str()),
            NetworkAgentInboxChange::Replace { .. } | NetworkAgentInboxChange::Redact { .. } => {
                None
            }
        })
        .collect();
    rooms.sort_unstable();
    rooms.dedup();
    if rooms.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r"DELETE FROM agent_room.network_agent_message message
           USING (
               SELECT sequence
                 FROM (SELECT sequence,
                              row_number() OVER (
                                  PARTITION BY matrix_room_id ORDER BY sequence DESC
                              ) AS newer
                         FROM agent_room.network_agent_message
                        WHERE network_agent_id = $1 AND matrix_room_id = ANY($3)) ranked
                WHERE ranked.newer > $2
           ) excess
           WHERE message.network_agent_id = $1 AND message.sequence = excess.sequence",
    )
    .bind(append.id.as_uuid())
    .bind(i64::from(append.history_capacity))
    .bind(&rooms)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 还没确认的条数；给了房间就只算这个房间的。
async fn count_pending<'e>(
    executor: impl PgExecutor<'e>,
    id: NetworkAgentId,
    room: Option<&str>,
    operation: &'static str,
) -> RepositoryResult<u64> {
    let pending: i64 = sqlx::query_scalar(
        r"SELECT count(*) FROM agent_room.network_agent_inbox
           WHERE network_agent_id = $1
             AND ($2::text IS NULL OR matrix_room_id = $2)",
    )
    .bind(id.as_uuid())
    .bind(room)
    .fetch_one(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    u64::try_from(pending).map_err(|_| corrupt_data(operation))
}
