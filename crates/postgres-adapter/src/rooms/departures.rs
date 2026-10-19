//! 撤销电脑以后它的 Agent 要离开的房间（`specs/computer-revocation/design.md`）。撤销的事务里记下
//! （`queue_revoked_agent_departures`），控制面一条一条领、做完删掉。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentRoomKind, MatrixUserId, PortFuture, RevokedAgentDepartureAudit,
        RevokedAgentDepartureAuditOutcome, RevokedAgentDepartureClaim, RevokedAgentDepartureFacts,
        RevokedAgentDepartureStore,
    },
};
use agent_room_domain::{
    ids::{AgentId, DeviceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomCatalogStatus},
    time::UtcMillis,
};
use serde_json::json;
use sqlx::{Postgres, Transaction, postgres::PgRow};
use uuid::Uuid;

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

use super::decode::corrupt_data;

/// 审计里写的是谁做的：撤销电脑以后由控制面自己做，没有登录的人。
const AUDIT_ACTOR: &str = "computer-revocation";

/// 撤销的事务里记下这台电脑上的连接占着位置的每个“Agent + Matrix 房间”。只记本机 Agent（网络 Agent
/// 由网关管进出）；只记公开大厅和私人房间。已经记着的不重记，重试的排期照旧。
pub(crate) async fn queue_revoked_agent_departures(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    requested_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.revoked_agent_departure
               (agent_id, room_instance_id, device_id, requested_at, next_attempt_at)
           SELECT DISTINCT reservation.agent_id, reservation.room_instance_id, instance.device_id,
                  to_timestamp($2::double precision / 1000.0),
                  to_timestamp($2::double precision / 1000.0)
             FROM agent_room.room_capacity_reservation AS reservation
             JOIN agent_room.agent_instance AS instance
               ON instance.id = reservation.agent_instance_id
             JOIN agent_room.room_catalog_entry AS catalog
               ON catalog.id = reservation.catalog_entry_id
            WHERE instance.device_id = $1
              AND instance.revoked_at IS NOT NULL
              AND reservation.state = 'committed'
              AND catalog.kind IN ('public_lobby', 'private_room')
              AND NOT EXISTS (
                  SELECT 1 FROM agent_room.network_agent AS network_agent
                   WHERE network_agent.agent_instance_id = instance.id
              )
           ON CONFLICT (agent_id, room_instance_id) DO NOTHING",
    )
    .bind(device_id.as_uuid())
    .bind(requested_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

impl RevokedAgentDepartureStore for PostgresRepositories {
    fn claim_departure(
        &self,
        device_id: Option<DeviceId>,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RevokedAgentDepartureClaim>>> {
        Box::pin(async move {
            let operation = "agent_room.departure.claim";
            let row = sqlx::query(
                r"WITH candidate AS (
                      SELECT agent_id, room_instance_id
                        FROM agent_room.revoked_agent_departure
                       WHERE next_attempt_at <= to_timestamp($1::double precision / 1000.0)
                         AND ($3::uuid IS NULL OR device_id = $3)
                       ORDER BY next_attempt_at, agent_id, room_instance_id
                       LIMIT 1
                       FOR UPDATE SKIP LOCKED
                  )
                  UPDATE agent_room.revoked_agent_departure AS departure
                     SET attempts = departure.attempts + 1,
                         next_attempt_at = to_timestamp($2::double precision / 1000.0)
                    FROM candidate
                   WHERE departure.agent_id = candidate.agent_id
                     AND departure.room_instance_id = candidate.room_instance_id
               RETURNING departure.agent_id, departure.room_instance_id, departure.device_id,
                         departure.attempts, departure.failure_code",
            )
            .bind(now.value())
            .bind(lease_until.value())
            .bind(device_id.map(DeviceId::as_uuid))
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_claim(&row, operation)).transpose()
        })
    }

    fn departure_facts<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
    ) -> PortFuture<'a, RepositoryResult<Option<RevokedAgentDepartureFacts>>> {
        Box::pin(async move {
            let operation = "agent_room.departure.facts";
            let row = sqlx::query(
                r"SELECT agent.matrix_user_id,
                         instance.catalog_entry_id,
                         instance.matrix_room_id,
                         catalog.kind,
                         catalog.status,
                         EXISTS (
                             SELECT 1
                               FROM agent_room.room_capacity_reservation AS reservation
                               JOIN agent_room.agent_instance AS connection
                                 ON connection.id = reservation.agent_instance_id
                              WHERE reservation.agent_id = agent.id
                                AND reservation.room_instance_id = instance.id
                                AND reservation.state = 'committed'
                                AND connection.revoked_at IS NULL
                         ) AS live_connection_remains,
                         ARRAY(
                             SELECT reservation.id
                               FROM agent_room.room_capacity_reservation AS reservation
                               JOIN agent_room.agent_instance AS connection
                                 ON connection.id = reservation.agent_instance_id
                              WHERE reservation.agent_id = agent.id
                                AND reservation.room_instance_id = instance.id
                                AND reservation.state = 'committed'
                                AND connection.revoked_at IS NOT NULL
                              ORDER BY reservation.id
                         ) AS revoked_reservations
                    FROM agent_room.agent AS agent
                    JOIN agent_room.room_instance AS instance ON instance.id = $2
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = instance.catalog_entry_id
                   WHERE agent.id = $1
                     AND catalog.kind IN ('public_lobby', 'private_room')
                     AND NOT EXISTS (
                         SELECT 1
                           FROM agent_room.network_agent AS network_agent
                           JOIN agent_room.agent_instance AS connection
                             ON connection.id = network_agent.agent_instance_id
                          WHERE connection.agent_id = agent.id
                     )",
            )
            .bind(claim.agent_id.as_uuid())
            .bind(claim.room_instance_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_facts(&row, operation)).transpose()
        })
    }

    fn complete_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "agent_room.departure.complete";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let deleted = sqlx::query(
                r"DELETE FROM agent_room.revoked_agent_departure
                   WHERE agent_id = $1 AND room_instance_id = $2 AND attempts = $3",
            )
            .bind(claim.agent_id.as_uuid())
            .bind(claim.room_instance_id.as_uuid())
            .bind(attempt_column(claim.attempt, operation)?)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected()
                == 1;
            if deleted && let Some(audit) = audit {
                insert_audit(&mut transaction, claim, audit, operation).await?;
            }
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(deleted)
        })
    }

    fn retry_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        retry_at: UtcMillis,
        failure_code: &'static str,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "agent_room.departure.retry";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let updated = sqlx::query(
                r"UPDATE agent_room.revoked_agent_departure
                     SET next_attempt_at = to_timestamp($4::double precision / 1000.0),
                         failure_code = $5
                   WHERE agent_id = $1 AND room_instance_id = $2 AND attempts = $3",
            )
            .bind(claim.agent_id.as_uuid())
            .bind(claim.room_instance_id.as_uuid())
            .bind(attempt_column(claim.attempt, operation)?)
            .bind(retry_at.value())
            .bind(failure_code)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected()
                == 1;
            if updated && let Some(audit) = audit {
                insert_audit(&mut transaction, claim, audit, operation).await?;
            }
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(updated)
        })
    }
}

/// 治理台账只读 `moderation.%` 的审计，这几条不会出现在台账里。
async fn insert_audit(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RevokedAgentDepartureClaim,
    audit: &RevokedAgentDepartureAudit,
    operation: &'static str,
) -> RepositoryResult<()> {
    let (action, outcome, reason, details) = match audit.outcome {
        RevokedAgentDepartureAuditOutcome::Departed {
            left_room,
            released_reservations,
        } => (
            "agent.room.departed",
            "allowed",
            "device_revoked",
            json!({ "leftRoom": left_room, "releasedReservations": released_reservations }),
        ),
        RevokedAgentDepartureAuditOutcome::Failed { failure_code } => (
            "agent.room.depart_failed",
            "failed",
            failure_code,
            json!({}),
        ),
    };
    let mut metadata = json!({
        "roomCatalogId": audit.catalog_id.to_string(),
        "matrixRoomId": audit.matrix_room_id.as_str(),
        "deviceId": claim.device_id.to_string(),
    });
    if let (Some(metadata), Some(details)) = (metadata.as_object_mut(), details.as_object()) {
        metadata.extend(details.clone());
    }
    let id = Uuid::now_v7();
    sqlx::query(
        r"INSERT INTO agent_room.audit_event (
               id, occurred_at, actor_kind, actor_reference, action,
               target_kind, target_reference, outcome, reason_code,
               correlation_id, metadata
           ) VALUES (
               $1, to_timestamp($2::double precision / 1000.0),
               'service', $3, $4, 'agent', $5, $6, $7, $1, $8
           )",
    )
    .bind(id)
    .bind(audit.occurred_at.value())
    .bind(AUDIT_ACTOR)
    .bind(action)
    .bind(claim.agent_id.to_string())
    .bind(outcome)
    .bind(reason)
    .bind(metadata)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn attempt_column(attempt: u32, operation: &'static str) -> RepositoryResult<i32> {
    i32::try_from(attempt)
        .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::Constraint))
}

fn decode_claim(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RevokedAgentDepartureClaim> {
    let agent_id: Uuid = decode_column(row, "agent_id", operation)?;
    let room_instance_id: Uuid = decode_column(row, "room_instance_id", operation)?;
    let device_id: Uuid = decode_column(row, "device_id", operation)?;
    let attempts: i32 = decode_column(row, "attempts", operation)?;
    Ok(RevokedAgentDepartureClaim {
        agent_id: AgentId::from_uuid(agent_id),
        room_instance_id: RoomInstanceId::from_uuid(room_instance_id),
        device_id: DeviceId::from_uuid(device_id),
        attempt: u32::try_from(attempts).map_err(|_| corrupt_data(operation))?,
        previous_failure_code: decode_column(row, "failure_code", operation)?,
    })
}

fn decode_facts(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RevokedAgentDepartureFacts> {
    let matrix_user_id: String = decode_column(row, "matrix_user_id", operation)?;
    let catalog_id: Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let matrix_room_id: String = decode_column(row, "matrix_room_id", operation)?;
    let kind: String = decode_column(row, "kind", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    let revoked_reservations: Vec<Uuid> = decode_column(row, "revoked_reservations", operation)?;
    Ok(RevokedAgentDepartureFacts {
        matrix_user_id: MatrixUserId::new(matrix_user_id).map_err(|_| corrupt_data(operation))?,
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        matrix_room_id: MatrixRoomReference::new(matrix_room_id)
            .map_err(|_| corrupt_data(operation))?,
        kind: match kind.as_str() {
            "public_lobby" => AgentRoomKind::PublicLobby,
            "private_room" => AgentRoomKind::PrivateRoom,
            _ => return Err(corrupt_data(operation)),
        },
        status: RoomCatalogStatus::try_from(status.as_str())
            .map_err(|_| corrupt_data(operation))?,
        live_connection_remains: decode_column(row, "live_connection_remains", operation)?,
        revoked_reservations: revoked_reservations
            .into_iter()
            .map(RoomReservationId::from_uuid)
            .collect(),
    })
}
