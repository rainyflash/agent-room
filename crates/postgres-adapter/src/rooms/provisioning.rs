use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixRoomAliasLocalpart, PortFuture, RoomProvisioningClaim, RoomProvisioningClaimOutcome,
        RoomProvisioningFailureCode, RoomProvisioningJob, RoomProvisioningKind,
        RoomProvisioningStore, RoomProvisioningTarget,
    },
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomProvisioningJobId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogKind, RoomCatalogStatus, RoomInstance,
        RoomRegion,
    },
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories, agents::decode_column, error::map_sqlx_error, transaction::finish,
};

use super::decode::{
    CATALOG_COLUMNS, INSTANCE_COLUMNS, corrupt_data, decode_catalog, decode_instance,
};

const JOB_COLUMNS: &str = r"
    job.id AS provisioning_job_id,
    job.target_kind AS provisioning_target_kind,
    job.room_instance_id AS provisioning_room_instance_id,
    job.region_hint AS provisioning_region_hint,
    job.room_alias_localpart AS provisioning_alias_localpart,
    job.matrix_room_id AS provisioning_matrix_room_id,
    job.lease_id AS provisioning_lease_id,
    floor(extract(epoch FROM job.lease_expires_at) * 1000)::bigint
        AS provisioning_lease_expires_at_ms";

impl RoomProvisioningStore for PostgresRepositories {
    fn claim<'a>(
        &'a self,
        claim: &'a RoomProvisioningClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomProvisioningClaimOutcome>> {
        Box::pin(async move { self.claim_room_provisioning(claim).await })
    }

    fn checkpoint_matrix_room<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_room_id: &'a MatrixRoomReference,
        checkpointed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.checkpoint_provisioned_matrix_room(job, matrix_room_id, checkpointed_at)
                .await
        })
    }

    fn complete_space<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_space_id: &'a MatrixRoomReference,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomCatalog>> {
        Box::pin(async move {
            self.complete_provisioned_space(job, matrix_space_id, completed_at)
                .await
        })
    }

    fn complete_instance<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        room: &'a RoomInstance,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomInstance>> {
        Box::pin(async move {
            self.complete_provisioned_instance(job, room, completed_at)
                .await
        })
    }

    fn release<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        failure: RoomProvisioningFailureCode,
        released_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.release_room_provisioning(job, failure, released_at)
                .await
        })
    }
}

impl PostgresRepositories {
    async fn claim_room_provisioning(
        &self,
        claim: &RoomProvisioningClaim,
    ) -> RepositoryResult<RoomProvisioningClaimOutcome> {
        let operation = "room_provisioning.claim";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            let catalog = lock_catalog(&mut transaction, claim.catalog_id(), operation).await?;
            ensure_provisionable_catalog(&catalog, operation)?;
            match claim.target() {
                RoomProvisioningTarget::Space if catalog.matrix_space_id().is_some() => {
                    return Ok(RoomProvisioningClaimOutcome::SpaceReady { catalog });
                }
                RoomProvisioningTarget::Instance { region, .. } => {
                    if catalog.matrix_space_id().is_none() {
                        return Err(constraint(operation));
                    }
                    if let Some(room) = lock_ready_instance(
                        &mut transaction,
                        catalog.id(),
                        region.as_ref(),
                        operation,
                    )
                    .await?
                    {
                        return Ok(RoomProvisioningClaimOutcome::InstanceReady { room });
                    }
                }
                RoomProvisioningTarget::Space => {}
            }

            if let Some(row) = lock_pending_job(&mut transaction, claim, operation).await? {
                let retry_at: Option<i64> =
                    decode_column(&row, "provisioning_lease_expires_at_ms", operation)?;
                if retry_at.is_some_and(|retry_at| retry_at > claim.claimed_at().value()) {
                    return Ok(RoomProvisioningClaimOutcome::Busy {
                        retry_at: UtcMillis::new(retry_at.expect("已判断租约时间存在"))
                            .map_err(|_| corrupt_data(operation))?,
                    });
                }
                claim_existing_job(&mut transaction, &row, claim, operation).await?;
                return decode_job(&row, catalog, claim, operation)
                    .map(RoomProvisioningClaimOutcome::Claimed);
            }

            insert_job(&mut transaction, claim, operation).await?;
            Ok(RoomProvisioningClaimOutcome::Claimed(
                RoomProvisioningJob::restore(
                    claim.job_id(),
                    claim.lease_id(),
                    catalog,
                    claim.target().clone(),
                    claim.alias_localpart().clone(),
                    None,
                    claim.expires_at(),
                ),
            ))
        }
        .await;
        finish(transaction, result, operation).await
    }

    async fn checkpoint_provisioned_matrix_room(
        &self,
        job: &RoomProvisioningJob,
        matrix_room_id: &MatrixRoomReference,
        checkpointed_at: UtcMillis,
    ) -> RepositoryResult<()> {
        let operation = "room_provisioning.checkpoint_matrix_room";
        let updated = sqlx::query_scalar::<_, uuid::Uuid>(
            r"UPDATE agent_room.room_provisioning_job
               SET matrix_room_id = $3,
                   updated_at = to_timestamp($4::double precision / 1000.0),
                   failure_code = NULL
               WHERE id = $1
                 AND lease_id = $2
                 AND state = 'pending'
                 AND lease_expires_at > to_timestamp($4::double precision / 1000.0)
                 AND (matrix_room_id IS NULL OR matrix_room_id = $3)
               RETURNING id",
        )
        .bind(job.job_id().as_uuid())
        .bind(job.lease_id().as_uuid())
        .bind(matrix_room_id.as_str())
        .bind(checkpointed_at.value())
        .fetch_optional(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        updated.map(|_| ()).ok_or_else(|| conflict(operation))
    }

    async fn complete_provisioned_space(
        &self,
        job: &RoomProvisioningJob,
        matrix_space_id: &MatrixRoomReference,
        completed_at: UtcMillis,
    ) -> RepositoryResult<RoomCatalog> {
        let operation = "room_provisioning.complete_space";
        if job.target().kind() != RoomProvisioningKind::Space {
            return Err(constraint(operation));
        }
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            lock_completable_job(
                &mut transaction,
                job,
                matrix_space_id,
                completed_at,
                operation,
            )
            .await?;
            let updated = sqlx::query_scalar::<_, uuid::Uuid>(
                r"UPDATE agent_room.room_catalog_entry
                   SET matrix_space_id = $2,
                       updated_at = to_timestamp($3::double precision / 1000.0)
                   WHERE id = $1
                     AND (matrix_space_id IS NULL OR matrix_space_id = $2)
                   RETURNING id",
            )
            .bind(job.catalog().id().as_uuid())
            .bind(matrix_space_id.as_str())
            .bind(completed_at.value())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if updated.is_none() {
                return Err(conflict(operation));
            }
            mark_job_completed(&mut transaction, job, completed_at, operation).await?;
            lock_catalog(&mut transaction, job.catalog().id(), operation).await
        }
        .await;
        finish(transaction, result, operation).await
    }

    async fn complete_provisioned_instance(
        &self,
        job: &RoomProvisioningJob,
        room: &RoomInstance,
        completed_at: UtcMillis,
    ) -> RepositoryResult<RoomInstance> {
        let operation = "room_provisioning.complete_instance";
        if job.target().kind() != RoomProvisioningKind::Instance
            || job.target().room_instance_id() != Some(room.id())
            || job.catalog().id() != room.catalog_id()
        {
            return Err(constraint(operation));
        }
        let activity_score =
            i64::try_from(room.activity_score_millis()).map_err(|_| constraint(operation))?;
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            lock_completable_job(
                &mut transaction,
                job,
                room.matrix_room_id(),
                completed_at,
                operation,
            )
            .await?;
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
            .bind(room.id().as_uuid())
            .bind(room.catalog_id().as_uuid())
            .bind(room.matrix_room_id().as_str())
            .bind(room.region().map(RoomRegion::as_str))
            .bind(i32::from(room.capacity().soft()))
            .bind(i32::from(room.capacity().hard()))
            .bind(i32::from(room.projected_member_count()))
            .bind(i32::from(room.allocated_slots()))
            .bind(activity_score)
            .bind(room.state().as_str())
            .bind(completed_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            mark_job_completed(&mut transaction, job, completed_at, operation).await?;
            Ok(room.clone())
        }
        .await;
        finish(transaction, result, operation).await
    }

    async fn release_room_provisioning(
        &self,
        job: &RoomProvisioningJob,
        failure: RoomProvisioningFailureCode,
        released_at: UtcMillis,
    ) -> RepositoryResult<()> {
        let operation = "room_provisioning.release";
        let updated = sqlx::query_scalar::<_, uuid::Uuid>(
            r"UPDATE agent_room.room_provisioning_job
               SET lease_id = NULL,
                   lease_expires_at = NULL,
                   failure_code = $3,
                   updated_at = to_timestamp($4::double precision / 1000.0)
               WHERE id = $1
                 AND lease_id = $2
                 AND state = 'pending'
                 AND lease_expires_at > to_timestamp($4::double precision / 1000.0)
               RETURNING id",
        )
        .bind(job.job_id().as_uuid())
        .bind(job.lease_id().as_uuid())
        .bind(failure.as_str())
        .bind(released_at.value())
        .fetch_optional(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        updated.map(|_| ()).ok_or_else(|| conflict(operation))
    }
}

async fn lock_catalog(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<RoomCatalog> {
    let statement = format!(
        r"SELECT {CATALOG_COLUMNS}
           FROM agent_room.room_catalog_entry AS catalog
           WHERE catalog.id = $1
           FOR UPDATE"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(catalog_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
        .ok_or_else(|| not_found(operation))?;
    decode_catalog(&row, operation)
}

fn ensure_provisionable_catalog(
    catalog: &RoomCatalog,
    operation: &'static str,
) -> RepositoryResult<()> {
    if catalog.kind() != RoomCatalogKind::PublicLobby
        || catalog.status() != RoomCatalogStatus::Active
    {
        return Err(constraint(operation));
    }
    Ok(())
}

async fn lock_ready_instance(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    region: Option<&RoomRegion>,
    operation: &'static str,
) -> RepositoryResult<Option<RoomInstance>> {
    let statement = format!(
        r"SELECT {INSTANCE_COLUMNS}
           FROM agent_room.room_instance AS instance
           WHERE instance.catalog_entry_id = $1
             AND instance.region_hint IS NOT DISTINCT FROM $2
             AND instance.state = 'active'
             AND instance.allocated_slots < instance.soft_capacity
           ORDER BY instance.allocated_slots ASC,
                    instance.activity_score DESC,
                    instance.id ASC
           LIMIT 1
           FOR UPDATE"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(catalog_id.as_uuid())
        .bind(region.map(RoomRegion::as_str))
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_instance(row, operation))
        .transpose()
}

async fn lock_pending_job(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomProvisioningClaim,
    operation: &'static str,
) -> RepositoryResult<Option<PgRow>> {
    let (region, instance) = match claim.target() {
        RoomProvisioningTarget::Space => (None, false),
        RoomProvisioningTarget::Instance { region, .. } => {
            (region.as_ref().map(RoomRegion::as_str), true)
        }
    };
    let statement = format!(
        r"SELECT {JOB_COLUMNS}
           FROM agent_room.room_provisioning_job AS job
           WHERE job.catalog_entry_id = $1
             AND job.target_kind = $2
             AND job.state = 'pending'
             AND ($3::boolean = false OR job.region_hint IS NOT DISTINCT FROM $4)
           FOR UPDATE"
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(claim.catalog_id().as_uuid())
        .bind(claim.target().kind().as_str())
        .bind(instance)
        .bind(region)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))
}

async fn claim_existing_job(
    transaction: &mut Transaction<'_, Postgres>,
    row: &PgRow,
    claim: &RoomProvisioningClaim,
    operation: &'static str,
) -> RepositoryResult<()> {
    let job_id: uuid::Uuid = decode_column(row, "provisioning_job_id", operation)?;
    let updated = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.room_provisioning_job
           SET lease_id = $2,
               lease_expires_at = to_timestamp($3::double precision / 1000.0),
               failure_code = NULL,
               updated_at = to_timestamp($4::double precision / 1000.0)
           WHERE id = $1 AND state = 'pending'
           RETURNING id",
    )
    .bind(job_id)
    .bind(claim.lease_id().as_uuid())
    .bind(claim.expires_at().value())
    .bind(claim.claimed_at().value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    updated.map(|_| ()).ok_or_else(|| conflict(operation))
}

fn decode_job(
    row: &PgRow,
    catalog: RoomCatalog,
    claim: &RoomProvisioningClaim,
    operation: &'static str,
) -> RepositoryResult<RoomProvisioningJob> {
    let job_id: uuid::Uuid = decode_column(row, "provisioning_job_id", operation)?;
    let target_kind: String = decode_column(row, "provisioning_target_kind", operation)?;
    let room_instance_id: Option<uuid::Uuid> =
        decode_column(row, "provisioning_room_instance_id", operation)?;
    let region: Option<String> = decode_column(row, "provisioning_region_hint", operation)?;
    let alias: String = decode_column(row, "provisioning_alias_localpart", operation)?;
    let matrix_room_id: Option<String> =
        decode_column(row, "provisioning_matrix_room_id", operation)?;
    let target = match (target_kind.as_str(), room_instance_id) {
        ("space", None) => RoomProvisioningTarget::Space,
        ("instance", Some(room_instance_id)) => RoomProvisioningTarget::Instance {
            room_instance_id: RoomInstanceId::from_uuid(room_instance_id),
            region: region
                .map(RoomRegion::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
        },
        _ => return Err(corrupt_data(operation)),
    };
    let lease_expires_at =
        UtcMillis::new(claim.expires_at().value()).map_err(|_| corrupt_data(operation))?;
    Ok(RoomProvisioningJob::restore(
        RoomProvisioningJobId::from_uuid(job_id),
        claim.lease_id(),
        catalog,
        target,
        MatrixRoomAliasLocalpart::new(alias).map_err(|_| corrupt_data(operation))?,
        matrix_room_id
            .map(MatrixRoomReference::new)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        lease_expires_at,
    ))
}

async fn insert_job(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomProvisioningClaim,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.room_provisioning_job (
               id, catalog_entry_id, target_kind, room_instance_id, region_hint,
               room_alias_localpart, matrix_room_id, state,
               lease_id, lease_expires_at, failure_code,
               created_at, updated_at, completed_at
           ) VALUES (
               $1, $2, $3, $4, $5, $6, NULL, 'pending',
               $7, to_timestamp($8::double precision / 1000.0), NULL,
               to_timestamp($9::double precision / 1000.0),
               to_timestamp($9::double precision / 1000.0), NULL
           )",
    )
    .bind(claim.job_id().as_uuid())
    .bind(claim.catalog_id().as_uuid())
    .bind(claim.target().kind().as_str())
    .bind(
        claim
            .target()
            .room_instance_id()
            .map(RoomInstanceId::as_uuid),
    )
    .bind(claim.target().region().map(RoomRegion::as_str))
    .bind(claim.alias_localpart().as_str())
    .bind(claim.lease_id().as_uuid())
    .bind(claim.expires_at().value())
    .bind(claim.claimed_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn lock_completable_job(
    transaction: &mut Transaction<'_, Postgres>,
    job: &RoomProvisioningJob,
    matrix_room_id: &MatrixRoomReference,
    completed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let found = sqlx::query_scalar::<_, uuid::Uuid>(
        r"SELECT id
           FROM agent_room.room_provisioning_job
           WHERE id = $1
             AND lease_id = $2
             AND state = 'pending'
             AND matrix_room_id = $3
             AND lease_expires_at > to_timestamp($4::double precision / 1000.0)
           FOR UPDATE",
    )
    .bind(job.job_id().as_uuid())
    .bind(job.lease_id().as_uuid())
    .bind(matrix_room_id.as_str())
    .bind(completed_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    found.map(|_| ()).ok_or_else(|| conflict(operation))
}

async fn mark_job_completed(
    transaction: &mut Transaction<'_, Postgres>,
    job: &RoomProvisioningJob,
    completed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let updated = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.room_provisioning_job
           SET state = 'completed',
               lease_id = NULL,
               lease_expires_at = NULL,
               failure_code = NULL,
               completed_at = to_timestamp($3::double precision / 1000.0),
               updated_at = to_timestamp($3::double precision / 1000.0)
           WHERE id = $1 AND lease_id = $2 AND state = 'pending'
           RETURNING id",
    )
    .bind(job.job_id().as_uuid())
    .bind(job.lease_id().as_uuid())
    .bind(completed_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    updated.map(|_| ()).ok_or_else(|| conflict(operation))
}

const fn not_found(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::NotFound)
}

const fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}

const fn constraint(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Constraint)
}
