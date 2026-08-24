use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ContentAccessPolicy, ContentEventBinding, ContentLifecycleTransition, ContentRepository,
        ContentUploadClaim, ContentUploadClaimOutcome, PortFuture, ReclaimableContentQuery,
    },
};
use agent_room_domain::{
    DomainError, DomainResult,
    content::{ContentLifecycleState, ContentObject, ContentScanState},
    ids::ContentId,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
    transaction::finish,
};

use self::decode::{CONTENT_COLUMNS, POLICY_COLUMNS, decode_content, decode_policy};

mod decode;

impl ContentRepository for PostgresRepositories {
    fn claim_upload<'a>(
        &'a self,
        claim: &'a ContentUploadClaim,
    ) -> PortFuture<'a, RepositoryResult<ContentUploadClaimOutcome>> {
        Box::pin(async move { self.claim_content_upload(claim).await })
    }

    fn find_content(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentObject>>> {
        Box::pin(async move {
            let operation = "content.find";
            let statement = format!(
                "SELECT {CONTENT_COLUMNS} FROM agent_room.content_object AS content WHERE content.id = $1"
            );
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .bind(content_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
                .as_ref()
                .map(|row| decode_content(row, operation))
                .transpose()
        })
    }

    fn find_access_policy(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentAccessPolicy>>> {
        Box::pin(async move {
            let operation = "content.find_access_policy";
            let statement = format!(
                "SELECT {POLICY_COLUMNS} FROM agent_room.content_access_policy AS policy WHERE policy.content_id = $1"
            );
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .bind(content_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
                .as_ref()
                .map(|row| decode_policy(row, operation))
                .transpose()
        })
    }

    fn activate(
        &self,
        content_id: ContentId,
        activated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            self.mutate_content(
                content_id,
                activated_at,
                "content.activate",
                ContentObject::activate,
            )
            .await
        })
    }

    fn record_scan(
        &self,
        content_id: ContentId,
        outcome: ContentScanState,
        scanned_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            self.mutate_content(content_id, scanned_at, "content.record_scan", |content| {
                content.record_scan(outcome)
            })
            .await
        })
    }

    fn bind_event<'a>(
        &'a self,
        binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>> {
        Box::pin(async move { self.bind_content_event(binding).await })
    }

    fn transition<'a>(
        &'a self,
        transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>> {
        Box::pin(async move { self.transition_content(transition).await })
    }

    fn list_reclaimable<'a>(
        &'a self,
        query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>> {
        Box::pin(async move { self.list_reclaimable_content(query).await })
    }

    fn mark_deleted(
        &self,
        content_id: ContentId,
        deleted_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            self.mutate_content(content_id, deleted_at, "content.mark_deleted", |content| {
                content.mark_deleted(deleted_at)
            })
            .await
        })
    }
}

impl PostgresRepositories {
    async fn claim_content_upload(
        &self,
        claim: &ContentUploadClaim,
    ) -> RepositoryResult<ContentUploadClaimOutcome> {
        let operation = "content.claim_upload";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            lock_upload_request(&mut transaction, claim, operation).await?;
            if let Some(row) = find_upload_request(&mut transaction, claim, operation).await? {
                return decode_existing_claim(&row, claim, operation);
            }
            insert_content(&mut transaction, &claim.content, operation).await?;
            insert_access_policy(&mut transaction, &claim.access_policy, operation).await?;
            insert_upload_request(&mut transaction, claim, operation).await?;
            Ok(ContentUploadClaimOutcome::Created {
                content: claim.content.clone(),
                access_policy: claim.access_policy.clone(),
            })
        }
        .await;
        finish(transaction, result, operation).await
    }

    async fn mutate_content<F>(
        &self,
        content_id: ContentId,
        changed_at: UtcMillis,
        operation: &'static str,
        mutation: F,
    ) -> RepositoryResult<ContentObject>
    where
        F: FnOnce(&mut ContentObject) -> DomainResult<()>,
    {
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            let mut content =
                load_content_for_update(&mut transaction, content_id, operation).await?;
            mutation(&mut content).map_err(|error| map_domain_error(operation, &error))?;
            save_content(&mut transaction, &content, changed_at, operation).await?;
            Ok(content)
        }
        .await;
        finish(transaction, result, operation).await
    }

    async fn transition_content(
        &self,
        transition: &ContentLifecycleTransition,
    ) -> RepositoryResult<ContentObject> {
        let expected = transition.expected;
        let target = transition.target;
        self.mutate_content(
            transition.content_id,
            transition.changed_at,
            "content.transition",
            move |content| {
                if content.lifecycle_state() != expected {
                    return Err(DomainError::InvalidTransition {
                        entity: "content_object",
                        from: content.lifecycle_state().as_str(),
                        to: target.as_str(),
                    });
                }
                apply_transition(content, target, transition.changed_at)
            },
        )
        .await
    }

    async fn bind_content_event(
        &self,
        binding: &ContentEventBinding,
    ) -> RepositoryResult<ContentAccessPolicy> {
        let operation = "content.bind_event";
        let statement = format!(
            r"UPDATE agent_room.content_access_policy AS policy
               SET matrix_event_id = COALESCE(policy.matrix_event_id, $3),
                   updated_at = to_timestamp($4::double precision / 1000.0)
               WHERE policy.content_id = $1
                 AND policy.matrix_room_id = $2
                 AND policy.revoked_at IS NULL
                 AND (policy.matrix_event_id IS NULL OR policy.matrix_event_id = $3)
                 AND EXISTS (
                     SELECT 1 FROM agent_room.content_object AS content
                     WHERE content.id = policy.content_id
                       AND content.lifecycle_state = 'active'
                 )
               RETURNING {POLICY_COLUMNS}"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(binding.content_id.as_uuid())
            .bind(binding.matrix_room_id.as_str())
            .bind(binding.matrix_event_id.as_str())
            .bind(binding.bound_at.value())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .ok_or_else(|| conflict(operation))?;
        decode_policy(&row, operation)
    }

    async fn list_reclaimable_content(
        &self,
        query: &ReclaimableContentQuery,
    ) -> RepositoryResult<Vec<ContentObject>> {
        let operation = "content.list_reclaimable";
        let statement = format!(
            r"SELECT {CONTENT_COLUMNS}
               FROM agent_room.content_object AS content
               LEFT JOIN agent_room.content_access_policy AS policy
                 ON policy.content_id = content.id
               WHERE content.lifecycle_state IN ('orphaned', 'redacted', 'expired')
                  OR (
                      content.lifecycle_state = 'uploading'
                      AND content.updated_at <= to_timestamp($1::double precision / 1000.0)
                  )
                  OR (
                      content.lifecycle_state = 'active'
                      AND content.expires_at <= to_timestamp($2::double precision / 1000.0)
                  )
                  OR (
                      content.lifecycle_state = 'active'
                      AND policy.matrix_event_id IS NULL
                      AND content.updated_at <= to_timestamp($1::double precision / 1000.0)
                  )
               ORDER BY COALESCE(content.expires_at, content.updated_at), content.id
               LIMIT $3"
        );
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(query.orphaned_before.value())
            .bind(query.now.value())
            .bind(i64::from(query.limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .iter()
            .map(|row| decode_content(row, operation))
            .collect()
    }
}

async fn lock_upload_request(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ContentUploadClaim,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(claim.request_id.to_string())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn find_upload_request(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ContentUploadClaim,
    operation: &'static str,
) -> RepositoryResult<Option<PgRow>> {
    let statement = format!(
        r"SELECT request.declaration_fingerprint AS upload_fingerprint,
                  request.owner_principal_id AS upload_owner_principal_id,
                  {CONTENT_COLUMNS},
                  {POLICY_COLUMNS}
           FROM agent_room.content_upload_request AS request
           JOIN agent_room.content_object AS content ON content.id = request.content_id
           JOIN agent_room.content_access_policy AS policy ON policy.content_id = content.id
           WHERE request.request_id = $1
           FOR UPDATE OF request"
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(claim.request_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn decode_existing_claim(
    row: &PgRow,
    claim: &ContentUploadClaim,
    operation: &'static str,
) -> RepositoryResult<ContentUploadClaimOutcome> {
    let fingerprint: Vec<u8> = decode_column(row, "upload_fingerprint", operation)?;
    let owner_principal_id: uuid::Uuid =
        decode_column(row, "upload_owner_principal_id", operation)?;
    if fingerprint.as_slice() != claim.fingerprint.as_bytes()
        || owner_principal_id != claim.content.owner_principal_id().as_uuid()
    {
        return Err(conflict(operation));
    }
    Ok(ContentUploadClaimOutcome::Existing {
        content: decode_content(row, operation)?,
        access_policy: decode_policy(row, operation)?,
    })
}

async fn insert_content(
    transaction: &mut Transaction<'_, Postgres>,
    content: &ContentObject,
    operation: &'static str,
) -> RepositoryResult<()> {
    let byte_length =
        i64::try_from(content.byte_length().value()).map_err(|_| constraint(operation))?;
    sqlx::query(
        r"INSERT INTO agent_room.content_object (
               id, owner_principal_id, storage_key, sha256_digest, byte_length,
               media_type, encryption_mode, scan_state, lifecycle_state,
               expires_at, created_at, deleted_at, updated_at, version
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8, $9,
               to_timestamp($10::double precision / 1000.0),
               to_timestamp($11::double precision / 1000.0),
               NULL,
               to_timestamp($11::double precision / 1000.0),
               0
           )",
    )
    .bind(content.id().as_uuid())
    .bind(content.owner_principal_id().as_uuid())
    .bind(content.storage_key().as_str())
    .bind(content.digest().as_bytes().as_slice())
    .bind(byte_length)
    .bind(content.media_type().as_str())
    .bind(content.encryption_mode().as_str())
    .bind(content.scan_state().as_str())
    .bind(content.lifecycle_state().as_str())
    .bind(content.expires_at().map(UtcMillis::value))
    .bind(content.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn insert_access_policy(
    transaction: &mut Transaction<'_, Postgres>,
    policy: &ContentAccessPolicy,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.content_access_policy (
               id, content_id, matrix_room_id, matrix_event_id,
               access_mode, created_at, revoked_at, updated_at
           ) VALUES (
               $1, $1, $2, NULL, $3,
               to_timestamp($4::double precision / 1000.0),
               NULL,
               to_timestamp($4::double precision / 1000.0)
           )",
    )
    .bind(policy.content_id().as_uuid())
    .bind(policy.matrix_room_id().as_str())
    .bind(policy.access_mode().as_str())
    .bind(policy.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn insert_upload_request(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ContentUploadClaim,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.content_upload_request (
               request_id, owner_principal_id, content_id,
               declaration_fingerprint, created_at, updated_at
           ) VALUES (
               $1, $2, $3, $4,
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0)
           )",
    )
    .bind(claim.request_id.as_uuid())
    .bind(claim.content.owner_principal_id().as_uuid())
    .bind(claim.content.id().as_uuid())
    .bind(claim.fingerprint.as_bytes().as_slice())
    .bind(claim.content.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn load_content_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    content_id: ContentId,
    operation: &'static str,
) -> RepositoryResult<ContentObject> {
    let statement = format!(
        r"SELECT {CONTENT_COLUMNS}
           FROM agent_room.content_object AS content
           WHERE content.id = $1
           FOR UPDATE"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(content_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
        .ok_or_else(|| not_found(operation))?;
    decode_content(&row, operation)
}

async fn save_content(
    transaction: &mut Transaction<'_, Postgres>,
    content: &ContentObject,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let updated = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.content_object
           SET scan_state = $2,
               lifecycle_state = $3,
               deleted_at = to_timestamp($4::double precision / 1000.0),
               updated_at = to_timestamp($5::double precision / 1000.0),
               version = version + 1
           WHERE id = $1
           RETURNING id",
    )
    .bind(content.id().as_uuid())
    .bind(content.scan_state().as_str())
    .bind(content.lifecycle_state().as_str())
    .bind(content.deleted_at().map(UtcMillis::value))
    .bind(changed_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    updated.map(|_| ()).ok_or_else(|| conflict(operation))
}

fn apply_transition(
    content: &mut ContentObject,
    target: ContentLifecycleState,
    changed_at: UtcMillis,
) -> DomainResult<()> {
    match target {
        ContentLifecycleState::Active => content.activate(),
        ContentLifecycleState::Orphaned => content.mark_orphaned(),
        ContentLifecycleState::Redacted => content.redact(),
        ContentLifecycleState::Expired => content.expire(changed_at),
        ContentLifecycleState::Deleted => content.mark_deleted(changed_at),
        ContentLifecycleState::Uploading => Err(DomainError::InvalidTransition {
            entity: "content_object",
            from: content.lifecycle_state().as_str(),
            to: ContentLifecycleState::Uploading.as_str(),
        }),
    }
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
