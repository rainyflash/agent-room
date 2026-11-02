//! 申请进公开房间（`specs/public-rooms/design.md`）。同一个人对同一间房同时只能有一个在等的申请，
//! 由部分唯一索引卡着；处理完的 30 天后由定时清理删掉。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        JoinRequestCreateOutcome, JoinRequestRecord, JoinRequestSweep, PendingJoinRequest,
        PortFuture, PrivateRoomJoinRequestStore,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
    time::UtcMillis,
};
use sqlx::postgres::PgRow;

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::decode::corrupt_data;

const ONE_PENDING_INDEX: &str = "private_room_join_request_one_pending";

const REQUEST_COLUMNS: &str = r"
    request.catalog_entry_id,
    request.requester_principal_id,
    request.message,
    request.status,
    floor(extract(epoch FROM request.requested_at) * 1000)::bigint AS requested_at_ms,
    floor(extract(epoch FROM request.decided_at) * 1000)::bigint AS decided_at_ms,
    request.decided_by";

impl PrivateRoomJoinRequestStore for PostgresRepositories {
    fn pending_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<JoinRequestRecord>>> {
        Box::pin(async move {
            let operation = "private_room.join_request.find_pending";
            let query = format!(
                "SELECT {REQUEST_COLUMNS} FROM agent_room.private_room_join_request AS request
                  WHERE request.catalog_entry_id = $1
                    AND request.requester_principal_id = $2
                    AND request.status = 'pending'"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .bind(requester.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_request(&row, operation)).transpose()
        })
    }

    fn last_rejected_at(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<UtcMillis>>> {
        Box::pin(async move {
            let operation = "private_room.join_request.last_rejected";
            let rejected_at: Option<i64> = sqlx::query_scalar(
                r"SELECT floor(extract(epoch FROM max(decided_at)) * 1000)::bigint
                    FROM agent_room.private_room_join_request
                   WHERE catalog_entry_id = $1
                     AND requester_principal_id = $2
                     AND status = 'rejected'",
            )
            .bind(catalog_id.as_uuid())
            .bind(requester.as_uuid())
            .fetch_one(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rejected_at
                .map(|value| {
                    UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error))
                })
                .transpose()
        })
    }

    fn create_join_request<'a>(
        &'a self,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<JoinRequestCreateOutcome>> {
        Box::pin(async move {
            let operation = "private_room.join_request.create";
            let result = sqlx::query(
                r"INSERT INTO agent_room.private_room_join_request
                      (catalog_entry_id, requester_principal_id, message, status, requested_at)
                  VALUES ($1, $2, $3, 'pending', to_timestamp($4::double precision / 1000.0))",
            )
            .bind(record.catalog_id.as_uuid())
            .bind(record.requester.as_uuid())
            .bind(record.message.as_str())
            .bind(record.requested_at.value())
            .execute(self.pool())
            .await;
            match result {
                Ok(_) => Ok(JoinRequestCreateOutcome::Created),
                Err(sqlx::Error::Database(database))
                    if database.constraint() == Some(ONE_PENDING_INDEX) =>
                {
                    // 两个请求同时来：另一个先记上了，回答那一个。
                    let existing = self
                        .pending_join_request(record.catalog_id, record.requester)
                        .await?
                        .ok_or_else(|| corrupt_data(operation))?;
                    Ok(JoinRequestCreateOutcome::AlreadyPending(existing))
                }
                Err(error) => Err(map_sqlx_error(operation, &error)),
            }
        })
    }

    fn pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PendingJoinRequest>>> {
        Box::pin(async move {
            let operation = "private_room.join_request.list_pending";
            let query = format!(
                "SELECT {REQUEST_COLUMNS}, principal.display_name AS requester_display_name
                   FROM agent_room.private_room_join_request AS request
                   JOIN agent_room.principal AS principal
                     ON principal.id = request.requester_principal_id
                  WHERE request.catalog_entry_id = $1 AND request.status = 'pending'
                  ORDER BY request.requested_at, request.id"
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| {
                    Ok(PendingJoinRequest {
                        record: decode_request(row, operation)?,
                        requester_display_name: decode_column(
                            row,
                            "requester_display_name",
                            operation,
                        )?,
                    })
                })
                .collect()
        })
    }

    fn decide_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
        status: JoinRequestStatus,
        decided_by: Option<PrincipalId>,
        decided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "private_room.join_request.decide";
            let decided = sqlx::query(
                r"UPDATE agent_room.private_room_join_request
                     SET status = $3,
                         decided_by = $4,
                         decided_at = greatest(
                             requested_at,
                             to_timestamp($5::double precision / 1000.0)
                         )
                   WHERE catalog_entry_id = $1
                     AND requester_principal_id = $2
                     AND status = 'pending'",
            )
            .bind(catalog_id.as_uuid())
            .bind(requester.as_uuid())
            .bind(status.as_str())
            .bind(decided_by.map(PrincipalId::as_uuid))
            .bind(decided_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(decided.rows_affected() == 1)
        })
    }

    fn void_pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
        voided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(async move {
            let operation = "private_room.join_request.void";
            let voided = sqlx::query(VOID_PENDING_OF_ROOM)
                .bind(catalog_id.as_uuid())
                .bind(voided_at.value())
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(voided.rows_affected())
        })
    }

    fn sweep_join_requests(
        &self,
        now: UtcMillis,
        decided_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<JoinRequestSweep>> {
        Box::pin(async move {
            let operation = "private_room.join_request.sweep";
            let voided = sqlx::query(VOID_PENDING_OF_CLOSED_ROOMS)
                .bind(now.value())
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let deleted = sqlx::query(
                r"DELETE FROM agent_room.private_room_join_request
                   WHERE status <> 'pending'
                     AND decided_at < to_timestamp($1::double precision / 1000.0)",
            )
            .bind(decided_before.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(JoinRequestSweep {
                voided: voided.rows_affected(),
                deleted: deleted.rows_affected(),
            })
        })
    }
}

const VOID_PENDING_OF_ROOM: &str = r"
    UPDATE agent_room.private_room_join_request
       SET status = 'voided',
           decided_at = greatest(requested_at, to_timestamp($2::double precision / 1000.0))
     WHERE catalog_entry_id = $1 AND status = 'pending'";

/// 房间不公开了（从没公开过、改回私密）、或者归档了：在等的作废。
const VOID_PENDING_OF_CLOSED_ROOMS: &str = r"
    UPDATE agent_room.private_room_join_request AS request
       SET status = 'voided',
           decided_at = greatest(request.requested_at, to_timestamp($1::double precision / 1000.0))
     WHERE request.status = 'pending'
       AND NOT EXISTS (
           SELECT 1
             FROM agent_room.private_room_listing AS listing
             JOIN agent_room.room_catalog_entry AS catalog
               ON catalog.id = listing.catalog_entry_id
            WHERE listing.catalog_entry_id = request.catalog_entry_id
              AND listing.listed
              AND catalog.status = 'active'
       )";

fn decode_request(row: &PgRow, operation: &'static str) -> RepositoryResult<JoinRequestRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let requester: uuid::Uuid = decode_column(row, "requester_principal_id", operation)?;
    let message: String = decode_column(row, "message", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    let requested_at: i64 = decode_column(row, "requested_at_ms", operation)?;
    let decided_at: Option<i64> = decode_column(row, "decided_at_ms", operation)?;
    let decided_by: Option<uuid::Uuid> = decode_column(row, "decided_by", operation)?;
    let time = |value| UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error));
    Ok(JoinRequestRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        requester: PrincipalId::from_uuid(requester),
        message: JoinRequestMessage::parse(&message).map_err(|_| corrupt_data(operation))?,
        status: JoinRequestStatus::try_from(status.as_str())
            .map_err(|_| corrupt_data(operation))?,
        requested_at: time(requested_at)?,
        decided_at: decided_at.map(time).transpose()?,
        decided_by: decided_by.map(PrincipalId::from_uuid),
    })
}
