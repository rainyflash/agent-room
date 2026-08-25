use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{PortFuture, PrivateRoomSnapshot, PrivateRoomStore},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{
        PrivateRoom, PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus,
        PrivateRoomPermissions,
    },
    rooms::{
        MatrixRoomReference, RoomCatalogKind, RoomCatalogStatus, RoomInstanceState, RoomLanguage,
        RoomRegion,
    },
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

const PRIVATE_ROOM_COLUMNS: &str = r"
    private_state.version AS private_room_version,
    membership.principal_id AS private_member_principal_id,
    membership.membership_status AS private_member_status,
    membership.permission_bits AS private_member_permission_bits";

impl PrivateRoomStore for PostgresRepositories {
    fn create<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.create";
            ensure_creatable(snapshot, operation)?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result =
                create_private_room(&mut transaction, snapshot, created_at, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            load_private_room(self, Some(catalog_id), None, "private_room.find_by_catalog").await
        })
    }

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            let operation = "private_room.list_for_principal";
            let catalog_ids = sqlx::query_scalar::<_, uuid::Uuid>(
                r"SELECT membership.catalog_entry_id
                   FROM agent_room.private_room_membership AS membership
                   JOIN agent_room.room_catalog_entry AS catalog
                     ON catalog.id = membership.catalog_entry_id
                   WHERE membership.principal_id = $1
                     AND membership.membership_status IN ('invited', 'joined')
                     AND catalog.kind = 'private_room'
                   ORDER BY catalog.updated_at DESC, catalog.id DESC",
            )
            .bind(principal_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

            let mut rooms = Vec::with_capacity(catalog_ids.len());
            for catalog_id in catalog_ids {
                let room = load_private_room(
                    self,
                    Some(RoomCatalogId::from_uuid(catalog_id)),
                    None,
                    operation,
                )
                .await?
                .ok_or_else(|| corrupt_data(operation))?;
                rooms.push(room);
            }
            Ok(rooms)
        })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            load_private_room(
                self,
                None,
                Some(matrix_room_id),
                "private_room.find_by_matrix_room",
            )
            .await
        })
    }

    fn save<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected_version: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.save";
            if expected_version
                .next()
                .map_err(|error| map_domain_error(operation, &error))?
                != room.version()
            {
                return Err(constraint(operation));
            }
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = save_private_room(
                &mut transaction,
                room,
                expected_version,
                changed_at,
                operation,
            )
            .await;
            finish(transaction, result, operation).await
        })
    }
}

async fn create_private_room(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot: &PrivateRoomSnapshot,
    created_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let catalog = snapshot.catalog();
    let instance = snapshot.instance();
    let room = snapshot.room();
    let activity_score =
        i64::try_from(instance.activity_score_millis()).map_err(|_| constraint(operation))?;

    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
               id, kind, slug, name, description, language, matrix_space_id,
               owner_principal_id, visibility, retention_days, status,
               created_at, updated_at
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
               to_timestamp($12::double precision / 1000.0),
               to_timestamp($12::double precision / 1000.0)
           )",
    )
    .bind(catalog.id().as_uuid())
    .bind(catalog.kind().as_str())
    .bind(
        catalog
            .slug()
            .map(agent_room_domain::rooms::RoomSlug::as_str),
    )
    .bind(catalog.name())
    .bind(catalog.description())
    .bind(catalog.language().map(RoomLanguage::as_str))
    .bind(catalog.matrix_space_id().map(MatrixRoomReference::as_str))
    .bind(catalog.owner_principal_id().map(PrincipalId::as_uuid))
    .bind(catalog.visibility().as_str())
    .bind(catalog.retention_days().map(i32::from))
    .bind(catalog.status().as_str())
    .bind(created_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;

    sqlx::query(
        r"INSERT INTO agent_room.room_instance (
               id, catalog_entry_id, matrix_room_id, region_hint,
               soft_capacity, hard_capacity, member_count_projection,
               allocated_slots, activity_score, state,
               created_at, updated_at, version
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8,
               $9::double precision / 1000.0, $10,
               to_timestamp($11::double precision / 1000.0),
               to_timestamp($11::double precision / 1000.0), 0
           )",
    )
    .bind(instance.id().as_uuid())
    .bind(instance.catalog_id().as_uuid())
    .bind(instance.matrix_room_id().as_str())
    .bind(instance.region().map(RoomRegion::as_str))
    .bind(i32::from(instance.capacity().soft()))
    .bind(i32::from(instance.capacity().hard()))
    .bind(i32::from(instance.projected_member_count()))
    .bind(i32::from(instance.allocated_slots()))
    .bind(activity_score)
    .bind(instance.state().as_str())
    .bind(created_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;

    sqlx::query(
        r"INSERT INTO agent_room.private_room_state (
               catalog_entry_id, room_instance_id, version, created_at, updated_at
           ) VALUES (
               $1, $2, $3,
               to_timestamp($4::double precision / 1000.0),
               to_timestamp($4::double precision / 1000.0)
           )",
    )
    .bind(room.catalog_id().as_uuid())
    .bind(instance.id().as_uuid())
    .bind(room.version().value())
    .bind(created_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;

    for member in room.members() {
        insert_member(
            transaction,
            room.catalog_id(),
            member,
            created_at,
            operation,
        )
        .await?;
    }
    Ok(())
}

async fn save_private_room(
    transaction: &mut Transaction<'_, Postgres>,
    room: &PrivateRoom,
    expected_version: AggregateVersion,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let instance_id = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.private_room_state
           SET version = $2,
               updated_at = to_timestamp($3::double precision / 1000.0)
           WHERE catalog_entry_id = $1 AND version = $4
           RETURNING room_instance_id",
    )
    .bind(room.catalog_id().as_uuid())
    .bind(room.version().value())
    .bind(changed_at.value())
    .bind(expected_version.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| conflict(operation))?;

    let catalog_status = lifecycle_catalog_status(room.status());
    let updated_catalog = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.room_catalog_entry
           SET owner_principal_id = $2,
               status = $3,
               updated_at = to_timestamp($4::double precision / 1000.0)
           WHERE id = $1 AND kind = 'private_room'
           RETURNING id",
    )
    .bind(room.catalog_id().as_uuid())
    .bind(room.owner_principal_id().as_uuid())
    .bind(catalog_status.as_str())
    .bind(changed_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if updated_catalog.is_none() {
        return Err(corrupt_data(operation));
    }

    let instance_state = lifecycle_instance_state(room.status());
    let updated_instance = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.room_instance
           SET state = $2,
               updated_at = to_timestamp($3::double precision / 1000.0),
               version = version + 1
           WHERE id = $1 AND catalog_entry_id = $4
           RETURNING id",
    )
    .bind(instance_id)
    .bind(instance_state.as_str())
    .bind(changed_at.value())
    .bind(room.catalog_id().as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if updated_instance.is_none() {
        return Err(corrupt_data(operation));
    }

    for member in room.members() {
        upsert_member(
            transaction,
            room.catalog_id(),
            member,
            changed_at,
            operation,
        )
        .await?;
    }
    Ok(())
}

async fn insert_member(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    member: &PrivateRoomMember,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_membership (
               catalog_entry_id, principal_id, membership_status, permission_bits,
               created_at, status_changed_at
           ) VALUES (
               $1, $2, $3, $4,
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0)
           )",
    )
    .bind(catalog_id.as_uuid())
    .bind(member.principal_id().as_uuid())
    .bind(member.status().as_str())
    .bind(i16::from(member.permissions().bits()))
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn upsert_member(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    member: &PrivateRoomMember,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_membership AS existing (
               catalog_entry_id, principal_id, membership_status, permission_bits,
               created_at, status_changed_at
           ) VALUES (
               $1, $2, $3, $4,
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0)
           )
           ON CONFLICT (catalog_entry_id, principal_id) DO UPDATE
           SET membership_status = EXCLUDED.membership_status,
               permission_bits = EXCLUDED.permission_bits,
               status_changed_at = CASE
                   WHEN existing.membership_status IS DISTINCT FROM EXCLUDED.membership_status
                     OR existing.permission_bits IS DISTINCT FROM EXCLUDED.permission_bits
                   THEN EXCLUDED.status_changed_at
                   ELSE existing.status_changed_at
               END",
    )
    .bind(catalog_id.as_uuid())
    .bind(member.principal_id().as_uuid())
    .bind(member.status().as_str())
    .bind(i16::from(member.permissions().bits()))
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn load_private_room(
    repositories: &PostgresRepositories,
    catalog_id: Option<RoomCatalogId>,
    matrix_room_id: Option<&MatrixRoomReference>,
    operation: &'static str,
) -> RepositoryResult<Option<PrivateRoomSnapshot>> {
    if catalog_id.is_some() == matrix_room_id.is_some() {
        return Err(constraint(operation));
    }
    let statement = format!(
        r"SELECT {CATALOG_COLUMNS}, {INSTANCE_COLUMNS}, {PRIVATE_ROOM_COLUMNS}
           FROM agent_room.private_room_state AS private_state
           JOIN agent_room.room_catalog_entry AS catalog
             ON catalog.id = private_state.catalog_entry_id
           JOIN agent_room.room_instance AS instance
             ON instance.id = private_state.room_instance_id
           JOIN agent_room.private_room_membership AS membership
             ON membership.catalog_entry_id = private_state.catalog_entry_id
           WHERE ($1::uuid IS NOT NULL AND catalog.id = $1)
              OR ($2::text IS NOT NULL AND instance.matrix_room_id = $2)
           ORDER BY membership.principal_id ASC"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(catalog_id.map(RoomCatalogId::as_uuid))
        .bind(matrix_room_id.map(MatrixRoomReference::as_str))
        .fetch_all(repositories.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    if rows.is_empty() {
        return Ok(None);
    }
    decode_private_room(&rows, operation).map(Some)
}

fn decode_private_room(
    rows: &[PgRow],
    operation: &'static str,
) -> RepositoryResult<PrivateRoomSnapshot> {
    let first = rows.first().ok_or_else(|| corrupt_data(operation))?;
    let catalog = decode_catalog(first, operation)?;
    let instance = decode_instance(first, operation)?;
    let version: i64 = decode_column(first, "private_room_version", operation)?;
    let members = rows
        .iter()
        .map(|row| decode_member(row, operation))
        .collect::<RepositoryResult<Vec<_>>>()?;
    let lifecycle = match catalog.status() {
        RoomCatalogStatus::Active => PrivateRoomLifecycleStatus::Active,
        RoomCatalogStatus::Archived => PrivateRoomLifecycleStatus::Archived,
        RoomCatalogStatus::Frozen => return Err(corrupt_data(operation)),
    };
    let owner = catalog
        .owner_principal_id()
        .ok_or_else(|| corrupt_data(operation))?;
    let room = PrivateRoom::restore(
        catalog.id(),
        owner,
        lifecycle,
        members,
        AggregateVersion::new(version).map_err(|_| corrupt_data(operation))?,
    )
    .map_err(|_| corrupt_data(operation))?;
    PrivateRoomSnapshot::new(catalog, instance, room).map_err(|_| corrupt_data(operation))
}

fn decode_member(row: &PgRow, operation: &'static str) -> RepositoryResult<PrivateRoomMember> {
    let principal_id: uuid::Uuid = decode_column(row, "private_member_principal_id", operation)?;
    let status: String = decode_column(row, "private_member_status", operation)?;
    let permission_bits: i16 = decode_column(row, "private_member_permission_bits", operation)?;
    let bits = u8::try_from(permission_bits).map_err(|_| corrupt_data(operation))?;
    PrivateRoomMember::restore(
        PrincipalId::from_uuid(principal_id),
        PrivateRoomMembershipStatus::try_from(status.as_str())
            .map_err(|_| corrupt_data(operation))?,
        PrivateRoomPermissions::from_bits(bits).map_err(|_| corrupt_data(operation))?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn ensure_creatable(
    snapshot: &PrivateRoomSnapshot,
    operation: &'static str,
) -> RepositoryResult<()> {
    if snapshot.catalog().kind() != RoomCatalogKind::PrivateRoom
        || snapshot.catalog().status() != RoomCatalogStatus::Active
        || snapshot.instance().state() != RoomInstanceState::Active
        || snapshot.room().status() != PrivateRoomLifecycleStatus::Active
    {
        return Err(constraint(operation));
    }
    Ok(())
}

const fn lifecycle_catalog_status(status: PrivateRoomLifecycleStatus) -> RoomCatalogStatus {
    match status {
        PrivateRoomLifecycleStatus::Active => RoomCatalogStatus::Active,
        PrivateRoomLifecycleStatus::Archived => RoomCatalogStatus::Archived,
    }
}

const fn lifecycle_instance_state(status: PrivateRoomLifecycleStatus) -> RoomInstanceState {
    match status {
        PrivateRoomLifecycleStatus::Active => RoomInstanceState::Active,
        PrivateRoomLifecycleStatus::Archived => RoomInstanceState::Archived,
    }
}

const fn constraint(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Constraint)
}

const fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}
