use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{DirectSessionRecord, DirectSessionStore, PortFuture},
};
use agent_room_domain::{
    direct_sessions::{DirectContactPolicy, DirectSession, DirectSessionLifecycle},
    ids::{AgentId, PrincipalId, RoomCatalogId},
    rooms::{MatrixRoomReference, RoomCatalogKind, RoomCatalogStatus, RoomInstanceState},
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
    transaction::finish,
};

use super::decode::{
    CATALOG_COLUMNS, INSTANCE_COLUMNS, corrupt_data, decode_catalog, decode_instance,
};

const DIRECT_SESSION_COLUMNS: &str = r"
    direct.principal_id AS direct_principal_id,
    direct.target_agent_id AS direct_target_agent_id,
    direct.lifecycle_state AS direct_lifecycle_state,
    direct.version AS direct_version";

impl DirectSessionStore for PostgresRepositories {
    fn reserve<'a>(
        &'a self,
        record: &'a DirectSessionRecord,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>> {
        Box::pin(async move {
            let operation = "direct_session.reserve";
            ensure_reservable(record, operation)?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let inserted = reserve_record(&mut transaction, record, created_at, operation).await;
            let inserted = finish(transaction, inserted, operation).await?;
            if inserted {
                Ok(record.clone())
            } else {
                self.find_by_participants(
                    record.session().principal_id(),
                    record.session().target_agent_id(),
                )
                .await?
                .ok_or_else(|| corrupt_data(operation))
            }
        })
    }

    fn activate<'a>(
        &'a self,
        record: &'a DirectSessionRecord,
        expected_version: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>> {
        Box::pin(async move {
            let operation = "direct_session.activate";
            ensure_activatable(record, expected_version, operation)?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = activate_record(
                &mut transaction,
                record,
                expected_version,
                changed_at,
                operation,
            )
            .await;
            finish(transaction, result, operation).await?;
            Ok(record.clone())
        })
    }

    fn find_by_participants(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>> {
        Box::pin(load_record(
            self,
            DirectLookup::Participants {
                principal_id,
                agent_id,
            },
            "direct_session.find_by_participants",
        ))
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>> {
        Box::pin(load_record(
            self,
            DirectLookup::Catalog(catalog_id),
            "direct_session.find_by_catalog",
        ))
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<DirectSessionRecord>>> {
        Box::pin(load_record(
            self,
            DirectLookup::MatrixRoom(matrix_room_id),
            "direct_session.find_by_matrix_room",
        ))
    }

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<DirectSessionRecord>>> {
        Box::pin(async move {
            let operation = "direct_session.list_for_principal";
            let catalog_ids = sqlx::query_scalar::<_, uuid::Uuid>(
                r"SELECT catalog_entry_id
                   FROM agent_room.direct_session
                   WHERE principal_id = $1 AND lifecycle_state = 'active'
                   ORDER BY updated_at DESC, catalog_entry_id DESC",
            )
            .bind(principal_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let mut records = Vec::with_capacity(catalog_ids.len());
            for catalog_id in catalog_ids {
                let record = load_record(
                    self,
                    DirectLookup::Catalog(RoomCatalogId::from_uuid(catalog_id)),
                    operation,
                )
                .await?
                .ok_or_else(|| corrupt_data(operation))?;
                records.push(record);
            }
            Ok(records)
        })
    }

    fn contact_policy(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>> {
        Box::pin(load_contact_policy(
            self,
            principal_id,
            agent_id,
            "direct_session.contact_policy",
        ))
    }

    fn set_principal_block(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
        blocked: bool,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>> {
        Box::pin(async move {
            let operation = "direct_session.set_principal_block";
            if blocked {
                sqlx::query(
                    r"INSERT INTO agent_room.direct_contact_block (
                           principal_id, agent_id, blocker_kind, blocked_at, revoked_at
                       ) VALUES (
                           $1, $2, 'principal',
                           to_timestamp($3::double precision / 1000.0), NULL
                       )
                       ON CONFLICT (principal_id, agent_id, blocker_kind) DO UPDATE
                       SET blocked_at = CASE
                               WHEN direct_contact_block.revoked_at IS NULL
                               THEN direct_contact_block.blocked_at
                               ELSE EXCLUDED.blocked_at
                           END,
                           revoked_at = NULL",
                )
                .bind(principal_id.as_uuid())
                .bind(agent_id.as_uuid())
                .bind(changed_at.value())
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            } else {
                sqlx::query(
                    r"UPDATE agent_room.direct_contact_block
                       SET revoked_at = to_timestamp($3::double precision / 1000.0)
                       WHERE principal_id = $1
                         AND agent_id = $2
                         AND blocker_kind = 'principal'
                         AND revoked_at IS NULL",
                )
                .bind(principal_id.as_uuid())
                .bind(agent_id.as_uuid())
                .bind(changed_at.value())
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            load_contact_policy(self, principal_id, agent_id, operation).await
        })
    }
}

async fn reserve_record(
    transaction: &mut Transaction<'_, Postgres>,
    record: &DirectSessionRecord,
    created_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let catalog = record.catalog();
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
               id, kind, slug, name, description, language, matrix_space_id,
               owner_principal_id, visibility, retention_days, status,
               created_at, updated_at
           ) VALUES (
               $1, 'direct', NULL, $2, $3, NULL, NULL,
               $4, 'private', NULL, 'frozen',
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0)
           )",
    )
    .bind(catalog.id().as_uuid())
    .bind(catalog.name())
    .bind(catalog.description())
    .bind(record.session().principal_id().as_uuid())
    .bind(created_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;

    let inserted = sqlx::query_scalar::<_, uuid::Uuid>(
        r"INSERT INTO agent_room.direct_session (
               catalog_entry_id, principal_id, target_agent_id, lifecycle_state,
               version, created_at, updated_at
           ) VALUES (
               $1, $2, $3, 'provisioning', 0,
               to_timestamp($4::double precision / 1000.0),
               to_timestamp($4::double precision / 1000.0)
           )
           ON CONFLICT (principal_id, target_agent_id) DO NOTHING
           RETURNING catalog_entry_id",
    )
    .bind(catalog.id().as_uuid())
    .bind(record.session().principal_id().as_uuid())
    .bind(record.session().target_agent_id().as_uuid())
    .bind(created_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .is_some();
    if !inserted {
        sqlx::query("DELETE FROM agent_room.room_catalog_entry WHERE id = $1 AND kind = 'direct'")
            .bind(catalog.id().as_uuid())
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    Ok(inserted)
}

async fn activate_record(
    transaction: &mut Transaction<'_, Postgres>,
    record: &DirectSessionRecord,
    expected_version: AggregateVersion,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let instance = record.instance().ok_or_else(|| constraint(operation))?;
    let updated = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.direct_session
           SET lifecycle_state = 'active',
               version = $2,
               updated_at = to_timestamp($3::double precision / 1000.0)
           WHERE catalog_entry_id = $1
             AND lifecycle_state = 'provisioning'
             AND version = $4
           RETURNING catalog_entry_id",
    )
    .bind(record.session().catalog_id().as_uuid())
    .bind(record.session().version().value())
    .bind(changed_at.value())
    .bind(expected_version.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if updated.is_none() {
        return Err(conflict(operation));
    }

    sqlx::query(
        r"UPDATE agent_room.room_catalog_entry
           SET status = 'active',
               updated_at = to_timestamp($2::double precision / 1000.0)
           WHERE id = $1 AND kind = 'direct'",
    )
    .bind(record.session().catalog_id().as_uuid())
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;

    let activity_score =
        i64::try_from(instance.activity_score_millis()).map_err(|_| constraint(operation))?;
    sqlx::query(
        r"INSERT INTO agent_room.room_instance (
               id, catalog_entry_id, matrix_room_id, region_hint,
               soft_capacity, hard_capacity, member_count_projection,
               allocated_slots, activity_score, state,
               created_at, updated_at, version
           ) VALUES (
               $1, $2, $3, NULL, $4, $5, $6, $7,
               $8::double precision / 1000.0, 'active',
               to_timestamp($9::double precision / 1000.0),
               to_timestamp($9::double precision / 1000.0), 0
           )",
    )
    .bind(instance.id().as_uuid())
    .bind(instance.catalog_id().as_uuid())
    .bind(instance.matrix_room_id().as_str())
    .bind(i32::from(instance.capacity().soft()))
    .bind(i32::from(instance.capacity().hard()))
    .bind(i32::from(instance.projected_member_count()))
    .bind(i32::from(instance.allocated_slots()))
    .bind(activity_score)
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

enum DirectLookup<'a> {
    Participants {
        principal_id: PrincipalId,
        agent_id: AgentId,
    },
    Catalog(RoomCatalogId),
    MatrixRoom(&'a MatrixRoomReference),
}

async fn load_record(
    repositories: &PostgresRepositories,
    lookup: DirectLookup<'_>,
    operation: &'static str,
) -> RepositoryResult<Option<DirectSessionRecord>> {
    let (principal_id, agent_id, catalog_id, matrix_room_id) = match lookup {
        DirectLookup::Participants {
            principal_id,
            agent_id,
        } => (Some(principal_id), Some(agent_id), None, None),
        DirectLookup::Catalog(catalog_id) => (None, None, Some(catalog_id), None),
        DirectLookup::MatrixRoom(matrix_room_id) => (None, None, None, Some(matrix_room_id)),
    };
    let statement = format!(
        r"SELECT {CATALOG_COLUMNS}, {INSTANCE_COLUMNS}, {DIRECT_SESSION_COLUMNS}
           FROM agent_room.direct_session AS direct
           JOIN agent_room.room_catalog_entry AS catalog
             ON catalog.id = direct.catalog_entry_id
           LEFT JOIN agent_room.room_instance AS instance
             ON instance.catalog_entry_id = direct.catalog_entry_id
           WHERE ($1::uuid IS NOT NULL AND direct.principal_id = $1 AND direct.target_agent_id = $2)
              OR ($3::uuid IS NOT NULL AND direct.catalog_entry_id = $3)
              OR ($4::text IS NOT NULL AND instance.matrix_room_id = $4)"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(principal_id.map(PrincipalId::as_uuid))
        .bind(agent_id.map(AgentId::as_uuid))
        .bind(catalog_id.map(RoomCatalogId::as_uuid))
        .bind(matrix_room_id.map(MatrixRoomReference::as_str))
        .fetch_optional(repositories.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| decode_record(&row, operation)).transpose()
}

fn decode_record(row: &PgRow, operation: &'static str) -> RepositoryResult<DirectSessionRecord> {
    let catalog = decode_catalog(row, operation)?;
    let principal_id: uuid::Uuid = decode_column(row, "direct_principal_id", operation)?;
    let target_agent_id: uuid::Uuid = decode_column(row, "direct_target_agent_id", operation)?;
    let lifecycle: String = decode_column(row, "direct_lifecycle_state", operation)?;
    let version: i64 = decode_column(row, "direct_version", operation)?;
    let lifecycle = DirectSessionLifecycle::try_from(lifecycle.as_str())
        .map_err(|_| corrupt_data(operation))?;
    let session = DirectSession::restore(
        catalog.id(),
        PrincipalId::from_uuid(principal_id),
        AgentId::from_uuid(target_agent_id),
        lifecycle,
        AggregateVersion::new(version).map_err(|_| corrupt_data(operation))?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let instance_id: Option<uuid::Uuid> = decode_column(row, "room_instance_id", operation)?;
    let instance = if instance_id.is_some() {
        Some(decode_instance(row, operation)?)
    } else {
        None
    };
    DirectSessionRecord::new(catalog, instance, session).map_err(|_| corrupt_data(operation))
}

async fn load_contact_policy(
    repositories: &PostgresRepositories,
    principal_id: PrincipalId,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<DirectContactPolicy> {
    let blockers = sqlx::query_scalar::<_, String>(
        r"SELECT blocker_kind
           FROM agent_room.direct_contact_block
           WHERE principal_id = $1 AND agent_id = $2 AND revoked_at IS NULL",
    )
    .bind(principal_id.as_uuid())
    .bind(agent_id.as_uuid())
    .fetch_all(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let principal_blocks_agent = blockers.iter().any(|kind| kind == "principal");
    let agent_blocks_principal = blockers.iter().any(|kind| kind == "agent");
    if blockers
        .iter()
        .any(|kind| kind != "principal" && kind != "agent")
    {
        return Err(corrupt_data(operation));
    }
    Ok(DirectContactPolicy::restore(
        principal_id,
        agent_id,
        principal_blocks_agent,
        agent_blocks_principal,
    ))
}

fn ensure_reservable(
    record: &DirectSessionRecord,
    operation: &'static str,
) -> RepositoryResult<()> {
    if record.catalog().kind() != RoomCatalogKind::Direct
        || record.catalog().status() != RoomCatalogStatus::Frozen
        || record.instance().is_some()
        || record.session().lifecycle() != DirectSessionLifecycle::Provisioning
    {
        return Err(constraint(operation));
    }
    Ok(())
}

fn ensure_activatable(
    record: &DirectSessionRecord,
    expected_version: AggregateVersion,
    operation: &'static str,
) -> RepositoryResult<()> {
    if record.catalog().status() != RoomCatalogStatus::Active
        || record
            .instance()
            .is_none_or(|instance| instance.state() != RoomInstanceState::Active)
        || record.session().lifecycle() != DirectSessionLifecycle::Active
        || expected_version
            .next()
            .map_err(|error| map_domain_error(operation, &error))?
            != record.session().version()
    {
        return Err(constraint(operation));
    }
    Ok(())
}

const fn constraint(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Constraint)
}

const fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}
