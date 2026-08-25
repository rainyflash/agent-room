use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixRoomId, MatrixUserId, ModerationActionReservationOutcome, ModerationAuthority,
        ModerationReportPolicy, ModerationReportSubmissionOutcome, ModerationRepository,
        ModerationRoomContext, PortFuture,
    },
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationAuditEvent,
        ModerationAuditOutcome, ModerationCase, ModerationCaseState, ModerationEvidence,
        ModerationReason, ModerationRole, ModerationTarget, ModerationTargetKind,
    },
    time::UtcMillis,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_optional_time, decode_time},
    error::{map_domain_error, map_sqlx_error},
    transaction,
};

const CASE_COLUMNS: &str = r"moderation_case.id,
       moderation_case.reporter_principal_id,
       moderation_case.target_kind,
       moderation_case.target_reference,
       moderation_case.reason_code,
       moderation_case.description,
       moderation_case.room_catalog_id,
       moderation_case.matrix_event_id,
       moderation_case.reporter_submitted_excerpt,
       moderation_case.evidence_end_to_end_encrypted,
       moderation_case.state,
       floor(extract(epoch FROM moderation_case.created_at) * 1000)::bigint AS created_at_ms,
       floor(extract(epoch FROM moderation_case.resolved_at) * 1000)::bigint AS resolved_at_ms";

const ACTION_COLUMNS: &str = r"moderation_action.id,
       moderation_action.case_id,
       moderation_action.actor_principal_id,
       moderation_action.room_catalog_id,
       moderation_action.action_type,
       moderation_action.target_kind,
       moderation_action.target_reference,
       moderation_action.reason_code,
       floor(extract(epoch FROM moderation_action.starts_at) * 1000)::bigint AS starts_at_ms,
       floor(extract(epoch FROM moderation_action.expires_at) * 1000)::bigint AS expires_at_ms,
       moderation_action.status,
       moderation_action.failure_code,
       floor(extract(epoch FROM moderation_action.reversed_at) * 1000)::bigint AS reversed_at_ms";

const AUDIT_COLUMNS: &str = r"audit_event.id,
       floor(extract(epoch FROM audit_event.occurred_at) * 1000)::bigint AS occurred_at_ms,
       audit_event.actor_kind,
       audit_event.actor_reference,
       audit_event.action,
       audit_event.target_kind,
       audit_event.target_reference,
       audit_event.outcome,
       audit_event.reason_code,
       audit_event.correlation_id,
       audit_event.metadata";

impl ModerationRepository for PostgresRepositories {
    fn submit_case<'a>(
        &'a self,
        case: &'a ModerationCase,
        audit: &'a ModerationAuditEvent,
        policy: ModerationReportPolicy,
    ) -> PortFuture<'a, RepositoryResult<ModerationReportSubmissionOutcome>> {
        Box::pin(async move { self.submit_moderation_case(case, audit, policy).await })
    }

    fn find_case(
        &self,
        case_id: ModerationCaseId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationCase>>> {
        Box::pin(async move { find_case(&self.pool, case_id).await })
    }

    fn list_cases_for_reporter(
        &self,
        reporter_principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>> {
        Box::pin(async move { list_cases(&self.pool, reporter_principal_id).await })
    }

    fn reserve_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>> {
        Box::pin(async move { self.reserve_moderation_action(action, audit).await })
    }

    fn find_action(
        &self,
        action_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationAction>>> {
        Box::pin(async move { find_action(&self.pool, action_id).await })
    }

    fn finalize_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>> {
        Box::pin(async move { self.finalize_moderation_action(action, audit).await })
    }

    fn list_room_actions(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAction>>> {
        Box::pin(async move { list_room_actions(&self.pool, room_catalog_id).await })
    }

    fn append_audit<'a>(
        &'a self,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move { insert_audit(&self.pool, audit).await })
    }

    fn list_audit(
        &self,
        room_catalog_id: Option<RoomCatalogId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAuditEvent>>> {
        Box::pin(async move { list_audit(&self.pool, room_catalog_id, limit).await })
    }
}

impl ModerationAuthority for PostgresRepositories {
    fn may_report<'a>(
        &'a self,
        principal_id: PrincipalId,
        _target: &'a ModerationTarget,
        room_catalog_id: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move { may_report(&self.pool, principal_id, room_catalog_id).await })
    }

    fn inspect_room<'a>(
        &'a self,
        principal_id: PrincipalId,
        room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        Box::pin(async move {
            inspect_room_authority(&self.pool, principal_id, room_catalog_id, target).await
        })
    }

    fn platform_role(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ModerationRole>> {
        Box::pin(async move { platform_role(&self.pool, principal_id).await })
    }
}

impl PostgresRepositories {
    async fn submit_moderation_case(
        &self,
        case: &ModerationCase,
        audit: &ModerationAuditEvent,
        policy: ModerationReportPolicy,
    ) -> RepositoryResult<ModerationReportSubmissionOutcome> {
        const OPERATION: &str = "moderation.submit_case";
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = submit_case_in_transaction(&mut transaction, case, audit, policy).await;
        transaction::finish(transaction, result, OPERATION).await
    }

    async fn reserve_moderation_action(
        &self,
        action: &ModerationAction,
        audit: &ModerationAuditEvent,
    ) -> RepositoryResult<ModerationActionReservationOutcome> {
        const OPERATION: &str = "moderation.reserve_action";
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = reserve_action_in_transaction(&mut transaction, action, audit).await;
        transaction::finish(transaction, result, OPERATION).await
    }

    async fn finalize_moderation_action(
        &self,
        action: &ModerationAction,
        audit: &ModerationAuditEvent,
    ) -> RepositoryResult<ModerationAction> {
        const OPERATION: &str = "moderation.finalize_action";
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = finalize_action_in_transaction(&mut transaction, action, audit).await;
        transaction::finish(transaction, result, OPERATION).await
    }
}

async fn submit_case_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    case: &ModerationCase,
    audit: &ModerationAuditEvent,
    policy: ModerationReportPolicy,
) -> RepositoryResult<ModerationReportSubmissionOutcome> {
    const OPERATION: &str = "moderation.submit_case";
    if let Some(existing) = find_case_in_transaction(transaction, case.id()).await? {
        return if same_case(&existing, case) {
            Ok(ModerationReportSubmissionOutcome::Existing(existing))
        } else {
            Err(RepositoryError::new(
                OPERATION,
                RepositoryErrorKind::Conflict,
            ))
        };
    }
    let maximum = i32::from(policy.maximum_reports);
    if maximum == 0 {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Constraint,
        ));
    }
    let window_millis = i64::try_from(policy.window.value())
        .map_err(|_| RepositoryError::new(OPERATION, RepositoryErrorKind::Constraint))?;
    sqlx::query(
        r"INSERT INTO agent_room.moderation_report_rate (
               principal_id, window_started_at, report_count
           ) VALUES ($1, to_timestamp($2::double precision / 1000.0), 0)
           ON CONFLICT (principal_id) DO NOTHING",
    )
    .bind(case.reporter_principal_id().as_uuid())
    .bind(case.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let rate = sqlx::query(
        r"SELECT floor(extract(epoch FROM window_started_at) * 1000)::bigint AS window_started_at_ms,
                  report_count
           FROM agent_room.moderation_report_rate
           WHERE principal_id = $1
           FOR UPDATE",
    )
    .bind(case.reporter_principal_id().as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let window_started_at: i64 = decode_column(&rate, "window_started_at_ms", OPERATION)?;
    let report_count: i32 = decode_column(&rate, "report_count", OPERATION)?;
    let window_end = window_started_at
        .checked_add(window_millis)
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Constraint))?;
    let (next_window, next_count) = if case.created_at().value() >= window_end {
        (case.created_at().value(), 1)
    } else if report_count >= maximum {
        let retry_at =
            UtcMillis::new(window_end).map_err(|error| map_domain_error(OPERATION, &error))?;
        return Ok(ModerationReportSubmissionOutcome::RateLimited { retry_at });
    } else {
        (window_started_at, report_count.saturating_add(1))
    };
    sqlx::query(
        r"UPDATE agent_room.moderation_report_rate
           SET window_started_at = to_timestamp($2::double precision / 1000.0),
               report_count = $3
           WHERE principal_id = $1",
    )
    .bind(case.reporter_principal_id().as_uuid())
    .bind(next_window)
    .bind(next_count)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    insert_case(transaction, case).await?;
    insert_audit_in_transaction(transaction, audit).await?;
    Ok(ModerationReportSubmissionOutcome::Created(case.clone()))
}

async fn insert_case(
    transaction: &mut Transaction<'_, Postgres>,
    case: &ModerationCase,
) -> RepositoryResult<()> {
    const OPERATION: &str = "moderation.insert_case";
    sqlx::query(
        r"INSERT INTO agent_room.moderation_case (
               id, reporter_principal_id, target_kind, target_reference,
               reason_code, description, state, created_at, resolved_at,
               room_catalog_id, matrix_event_id, reporter_submitted_excerpt,
               evidence_end_to_end_encrypted
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7,
               to_timestamp($8::double precision / 1000.0), NULL,
               $9, $10, $11, $12
           )",
    )
    .bind(case.id().as_uuid())
    .bind(case.reporter_principal_id().as_uuid())
    .bind(case.target().kind().as_str())
    .bind(case.target().reference())
    .bind(case.reason().as_str())
    .bind(case.description())
    .bind(case.state().as_str())
    .bind(case.created_at().value())
    .bind(
        case.evidence()
            .room_catalog_id()
            .map(RoomCatalogId::as_uuid),
    )
    .bind(case.evidence().matrix_event_id())
    .bind(case.evidence().reporter_submitted_excerpt())
    .bind(case.evidence().end_to_end_encrypted())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(())
}

async fn reserve_action_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    action: &ModerationAction,
    audit: &ModerationAuditEvent,
) -> RepositoryResult<ModerationActionReservationOutcome> {
    const OPERATION: &str = "moderation.reserve_action";
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.moderation_action (
               id, case_id, actor_principal_id, action_type, target_reference,
               reason_code, starts_at, expires_at, reversed_at,
               room_catalog_id, target_kind, status, failure_code
           ) VALUES (
               $1, $2, $3, $4, $5, $6,
               to_timestamp($7::double precision / 1000.0),
               CASE WHEN $8::bigint IS NULL THEN NULL
                    ELSE to_timestamp($8::double precision / 1000.0) END,
               NULL, $9, $10, $11, NULL
           ) ON CONFLICT (id) DO NOTHING",
    )
    .bind(action.id().as_uuid())
    .bind(action.case_id().map(ModerationCaseId::as_uuid))
    .bind(action.actor_principal_id().as_uuid())
    .bind(action.kind().as_str())
    .bind(action.target().reference())
    .bind(action.reason().as_str())
    .bind(action.starts_at().value())
    .bind(action.expires_at().map(UtcMillis::value))
    .bind(action.room_catalog_id().as_uuid())
    .bind(action.target().kind().as_str())
    .bind(action.status().as_str())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let existing = find_action_for_update(transaction, action.id())
        .await?
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Unavailable))?;
    if !same_action_identity(&existing, action) {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }
    if inserted.rows_affected() == 0 {
        return Ok(ModerationActionReservationOutcome::Existing(existing));
    }
    insert_audit_in_transaction(transaction, audit).await?;
    Ok(ModerationActionReservationOutcome::Reserved(existing))
}

async fn finalize_action_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    action: &ModerationAction,
    audit: &ModerationAuditEvent,
) -> RepositoryResult<ModerationAction> {
    const OPERATION: &str = "moderation.finalize_action";
    let current = find_action_for_update(transaction, action.id())
        .await?
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::NotFound))?;
    if !same_action_identity(&current, action) {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }
    if current.status() == action.status()
        && current.failure_code() == action.failure_code()
        && current.reversed_at() == action.reversed_at()
    {
        return Ok(current);
    }
    let valid_transition = matches!(
        (current.status(), action.status()),
        (
            ModerationActionStatus::Pending,
            ModerationActionStatus::Applied | ModerationActionStatus::Failed
        ) | (
            ModerationActionStatus::Applied,
            ModerationActionStatus::Reversed
        )
    );
    if !valid_transition {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }
    let updated = sqlx::query(
        r"UPDATE agent_room.moderation_action
           SET status = $2,
               failure_code = $3,
               reversed_at = CASE WHEN $4::bigint IS NULL THEN NULL
                                  ELSE to_timestamp($4::double precision / 1000.0) END
           WHERE id = $1 AND status = $5",
    )
    .bind(action.id().as_uuid())
    .bind(action.status().as_str())
    .bind(action.failure_code())
    .bind(action.reversed_at().map(UtcMillis::value))
    .bind(current.status().as_str())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if updated.rows_affected() != 1 {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }
    insert_audit_in_transaction(transaction, audit).await?;
    find_action_for_update(transaction, action.id())
        .await?
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Unavailable))
}

async fn find_case(
    pool: &PgPool,
    id: ModerationCaseId,
) -> RepositoryResult<Option<ModerationCase>> {
    let operation = "moderation.find_case";
    let statement = format!("SELECT {CASE_COLUMNS} FROM agent_room.moderation_case WHERE id = $1");
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_case(row, operation))
        .transpose()
}

async fn find_case_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    id: ModerationCaseId,
) -> RepositoryResult<Option<ModerationCase>> {
    let operation = "moderation.find_case_for_update";
    let statement =
        format!("SELECT {CASE_COLUMNS} FROM agent_room.moderation_case WHERE id = $1 FOR UPDATE");
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_case(row, operation))
        .transpose()
}

async fn list_cases(
    pool: &PgPool,
    reporter_principal_id: PrincipalId,
) -> RepositoryResult<Vec<ModerationCase>> {
    let operation = "moderation.list_cases";
    let statement = format!(
        "SELECT {CASE_COLUMNS}
           FROM agent_room.moderation_case
           WHERE reporter_principal_id = $1
           ORDER BY created_at DESC, id DESC
           LIMIT 200"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(reporter_principal_id.as_uuid())
        .fetch_all(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter().map(|row| decode_case(row, operation)).collect()
}

async fn find_action(
    pool: &PgPool,
    id: ModerationActionId,
) -> RepositoryResult<Option<ModerationAction>> {
    let operation = "moderation.find_action";
    let statement =
        format!("SELECT {ACTION_COLUMNS} FROM agent_room.moderation_action WHERE id = $1");
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_action(row, operation))
        .transpose()
}

async fn find_action_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    id: ModerationActionId,
) -> RepositoryResult<Option<ModerationAction>> {
    let operation = "moderation.find_action_for_update";
    let statement = format!(
        "SELECT {ACTION_COLUMNS} FROM agent_room.moderation_action WHERE id = $1 FOR UPDATE"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_action(row, operation))
        .transpose()
}

async fn list_room_actions(
    pool: &PgPool,
    room_catalog_id: RoomCatalogId,
) -> RepositoryResult<Vec<ModerationAction>> {
    let operation = "moderation.list_room_actions";
    let statement = format!(
        "SELECT {ACTION_COLUMNS}
           FROM agent_room.moderation_action
           WHERE room_catalog_id = $1
           ORDER BY starts_at DESC, id DESC
           LIMIT 200"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(room_catalog_id.as_uuid())
        .fetch_all(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| decode_action(row, operation))
        .collect()
}

async fn insert_audit(pool: &PgPool, audit: &ModerationAuditEvent) -> RepositoryResult<()> {
    let operation = "moderation.append_audit";
    insert_audit_query(audit)
        .execute(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn insert_audit_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    audit: &ModerationAuditEvent,
) -> RepositoryResult<()> {
    let operation = "moderation.append_audit";
    insert_audit_query(audit)
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn insert_audit_query(
    audit: &ModerationAuditEvent,
) -> sqlx::query::Query<'_, Postgres, sqlx::postgres::PgArguments> {
    sqlx::query(
        r"INSERT INTO agent_room.audit_event (
               id, occurred_at, actor_kind, actor_reference, action,
               target_kind, target_reference, outcome, reason_code,
               correlation_id, metadata
           ) VALUES (
               $1, to_timestamp($2::double precision / 1000.0),
               'principal', $3, $4, $5, $6, $7, $8, $9, $10
           ) ON CONFLICT (id) DO NOTHING",
    )
    .bind(audit.id.as_uuid())
    .bind(audit.occurred_at.value())
    .bind(audit.actor_principal_id.to_string())
    .bind(&audit.action)
    .bind(audit.target.kind().as_str())
    .bind(audit.target.reference())
    .bind(audit.outcome.as_str())
    .bind(audit.reason.map(ModerationReason::as_str))
    .bind(audit.correlation_id.as_uuid())
    .bind(audit_metadata(audit.room_catalog_id))
}

async fn list_audit(
    pool: &PgPool,
    room_catalog_id: Option<RoomCatalogId>,
    limit: u16,
) -> RepositoryResult<Vec<ModerationAuditEvent>> {
    let operation = "moderation.list_audit";
    let statement = format!(
        "SELECT {AUDIT_COLUMNS}
           FROM agent_room.audit_event
           WHERE action LIKE 'moderation.%'
             AND ($1::text IS NULL OR metadata->>'roomCatalogId' = $1)
           ORDER BY occurred_at DESC, id DESC
           LIMIT $2"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(room_catalog_id.map(|id| id.to_string()))
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| decode_audit(row, operation))
        .collect()
}

async fn may_report(
    pool: &PgPool,
    principal_id: PrincipalId,
    room_catalog_id: Option<RoomCatalogId>,
) -> RepositoryResult<bool> {
    let operation = "moderation.authority.may_report";
    let allowed: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
             SELECT 1
             FROM agent_room.principal AS principal
             WHERE principal.id = $1 AND principal.status = 'active'
               AND (
                 $2::uuid IS NULL
                 OR EXISTS (
                   SELECT 1
                   FROM agent_room.room_catalog_entry AS catalog
                   WHERE catalog.id = $2 AND catalog.status = 'active'
                     AND (
                       catalog.kind = 'public_lobby'
                       OR (
                         catalog.kind = 'private_room'
                         AND EXISTS (
                           SELECT 1
                           FROM agent_room.private_room_membership AS membership
                           WHERE membership.catalog_entry_id = catalog.id
                             AND membership.principal_id = principal.id
                             AND membership.membership_status = 'joined'
                         )
                       )
                       OR (
                         catalog.kind = 'direct'
                         AND EXISTS (
                           SELECT 1
                           FROM agent_room.direct_session AS direct
                           WHERE direct.catalog_entry_id = catalog.id
                             AND direct.principal_id = principal.id
                             AND direct.lifecycle_state = 'active'
                         )
                       )
                     )
                 )
               )
           )",
    )
    .bind(principal_id.as_uuid())
    .bind(room_catalog_id.map(RoomCatalogId::as_uuid))
    .fetch_one(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(allowed)
}

async fn inspect_room_authority(
    pool: &PgPool,
    principal_id: PrincipalId,
    room_catalog_id: RoomCatalogId,
    target: &ModerationTarget,
) -> RepositoryResult<Option<ModerationRoomContext>> {
    let operation = "moderation.authority.inspect_room";
    let target_principal_id = if target.kind() == ModerationTargetKind::Principal {
        let parsed = uuid::Uuid::parse_str(target.reference()).ok();
        if parsed.is_none() {
            return Ok(None);
        }
        parsed
    } else {
        None
    };
    let row = sqlx::query(
        r"SELECT catalog.kind,
                  room.matrix_room_id,
                  target.matrix_user_id AS target_matrix_user_id,
                  EXISTS (
                    SELECT 1 FROM agent_room.moderation_operator AS operator
                    WHERE operator.principal_id = $1
                      AND operator.role = 'moderator'
                      AND operator.revoked_at IS NULL
                  ) AS is_platform_moderator,
                  EXISTS (
                    SELECT 1 FROM agent_room.private_room_membership AS membership
                    WHERE membership.catalog_entry_id = catalog.id
                      AND membership.principal_id = $1
                      AND membership.membership_status = 'joined'
                      AND (membership.permission_bits & 8) = 8
                  ) AS is_room_manager
           FROM agent_room.room_catalog_entry AS catalog
           JOIN agent_room.room_instance AS room
             ON room.catalog_entry_id = catalog.id AND room.state = 'active'
           LEFT JOIN agent_room.principal AS target
             ON target.id = $3 AND target.status = 'active'
           WHERE catalog.id = $2 AND catalog.status = 'active'
           ORDER BY room.updated_at DESC, room.id DESC
           LIMIT 1",
    )
    .bind(principal_id.as_uuid())
    .bind(room_catalog_id.as_uuid())
    .bind(target_principal_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let target_matrix_user: Option<String> =
        decode_column(&row, "target_matrix_user_id", operation)?;
    if target.kind() == ModerationTargetKind::Principal && target_matrix_user.is_none() {
        return Ok(None);
    }
    let catalog_kind: String = decode_column(&row, "kind", operation)?;
    let is_platform_moderator: bool = decode_column(&row, "is_platform_moderator", operation)?;
    let is_room_manager: bool = decode_column(&row, "is_room_manager", operation)?;
    let role = if is_platform_moderator {
        ModerationRole::PlatformModerator
    } else if catalog_kind == "private_room" && is_room_manager {
        ModerationRole::RoomManager
    } else {
        ModerationRole::None
    };
    let matrix_room_id: String = decode_column(&row, "matrix_room_id", operation)?;
    Ok(Some(ModerationRoomContext {
        role,
        matrix_room_id: MatrixRoomId::new(matrix_room_id).map_err(|_| corrupt_data(operation))?,
        target_matrix_user_id: target_matrix_user
            .map(MatrixUserId::new)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
    }))
}

async fn platform_role(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> RepositoryResult<ModerationRole> {
    let operation = "moderation.authority.platform_role";
    let roles = sqlx::query_scalar::<_, String>(
        r"SELECT role
           FROM agent_room.moderation_operator
           WHERE principal_id = $1 AND revoked_at IS NULL
           ORDER BY CASE role WHEN 'moderator' THEN 0 ELSE 1 END
           LIMIT 1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    match roles.as_deref() {
        Some("moderator") => Ok(ModerationRole::PlatformModerator),
        Some("audit_reader") => Ok(ModerationRole::AuditReader),
        None => Ok(ModerationRole::None),
        Some(_) => Err(corrupt_data(operation)),
    }
}

fn decode_case(row: &PgRow, operation: &'static str) -> RepositoryResult<ModerationCase> {
    let target_kind: String = decode_column(row, "target_kind", operation)?;
    let target = ModerationTarget::new(
        ModerationTargetKind::try_from(target_kind.as_str())
            .map_err(|_| corrupt_data(operation))?,
        decode_column::<String>(row, "target_reference", operation)?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let reason: String = decode_column(row, "reason_code", operation)?;
    let state: String = decode_column(row, "state", operation)?;
    let room_catalog_id: Option<uuid::Uuid> = decode_column(row, "room_catalog_id", operation)?;
    let evidence = ModerationEvidence::new(
        room_catalog_id.map(RoomCatalogId::from_uuid),
        decode_column(row, "matrix_event_id", operation)?,
        decode_column(row, "reporter_submitted_excerpt", operation)?,
        decode_column(row, "evidence_end_to_end_encrypted", operation)?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let reporter: uuid::Uuid = decode_column(row, "reporter_principal_id", operation)?;
    ModerationCase::restore(
        ModerationCaseId::from_uuid(id),
        PrincipalId::from_uuid(reporter),
        target,
        ModerationReason::try_from(reason.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_column::<String>(row, "description", operation)?,
        evidence,
        ModerationCaseState::try_from(state.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_time(row, "created_at_ms", operation)?,
        decode_optional_time(row, "resolved_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn decode_action(row: &PgRow, operation: &'static str) -> RepositoryResult<ModerationAction> {
    let target_kind: Option<String> = decode_column(row, "target_kind", operation)?;
    let room_catalog_id: Option<uuid::Uuid> = decode_column(row, "room_catalog_id", operation)?;
    let target_kind = target_kind.ok_or_else(|| corrupt_data(operation))?;
    let room_catalog_id = room_catalog_id.ok_or_else(|| corrupt_data(operation))?;
    let target = ModerationTarget::new(
        ModerationTargetKind::try_from(target_kind.as_str())
            .map_err(|_| corrupt_data(operation))?,
        decode_column::<String>(row, "target_reference", operation)?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let case_id: Option<uuid::Uuid> = decode_column(row, "case_id", operation)?;
    let actor: uuid::Uuid = decode_column(row, "actor_principal_id", operation)?;
    let kind: String = decode_column(row, "action_type", operation)?;
    let reason: String = decode_column(row, "reason_code", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    ModerationAction::restore(
        ModerationActionId::from_uuid(id),
        case_id.map(ModerationCaseId::from_uuid),
        PrincipalId::from_uuid(actor),
        RoomCatalogId::from_uuid(room_catalog_id),
        ModerationActionKind::try_from(kind.as_str()).map_err(|_| corrupt_data(operation))?,
        target,
        ModerationReason::try_from(reason.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_time(row, "starts_at_ms", operation)?,
        decode_optional_time(row, "expires_at_ms", operation)?,
        ModerationActionStatus::try_from(status.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_column(row, "failure_code", operation)?,
        decode_optional_time(row, "reversed_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn decode_audit(row: &PgRow, operation: &'static str) -> RepositoryResult<ModerationAuditEvent> {
    let actor_kind: String = decode_column(row, "actor_kind", operation)?;
    if actor_kind != "principal" {
        return Err(corrupt_data(operation));
    }
    let actor_reference: String = decode_column(row, "actor_reference", operation)?;
    let actor = uuid::Uuid::parse_str(&actor_reference).map_err(|_| corrupt_data(operation))?;
    let target_kind: String = decode_column(row, "target_kind", operation)?;
    let target = ModerationTarget::new(
        ModerationTargetKind::try_from(target_kind.as_str())
            .map_err(|_| corrupt_data(operation))?,
        decode_column::<String>(row, "target_reference", operation)?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let outcome: String = decode_column(row, "outcome", operation)?;
    let reason: Option<String> = decode_column(row, "reason_code", operation)?;
    let metadata: Value = decode_column(row, "metadata", operation)?;
    let room_catalog_id = metadata
        .get("roomCatalogId")
        .and_then(Value::as_str)
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| corrupt_data(operation))?
        .map(RoomCatalogId::from_uuid);
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let correlation_id: uuid::Uuid = decode_column(row, "correlation_id", operation)?;
    ModerationAuditEvent::new(
        AuditEventId::from_uuid(id),
        decode_time(row, "occurred_at_ms", operation)?,
        PrincipalId::from_uuid(actor),
        decode_column::<String>(row, "action", operation)?,
        target,
        match outcome.as_str() {
            "allowed" => ModerationAuditOutcome::Allowed,
            "denied" => ModerationAuditOutcome::Denied,
            "failed" => ModerationAuditOutcome::Failed,
            _ => return Err(corrupt_data(operation)),
        },
        reason
            .as_deref()
            .map(ModerationReason::try_from)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        AuditEventId::from_uuid(correlation_id),
        room_catalog_id,
    )
    .map_err(|_| corrupt_data(operation))
}

fn same_case(left: &ModerationCase, right: &ModerationCase) -> bool {
    left.id() == right.id()
        && left.reporter_principal_id() == right.reporter_principal_id()
        && left.target() == right.target()
        && left.reason() == right.reason()
        && left.description() == right.description()
        && left.evidence() == right.evidence()
        && left.created_at() == right.created_at()
}

fn same_action_identity(left: &ModerationAction, right: &ModerationAction) -> bool {
    left.id() == right.id()
        && left.case_id() == right.case_id()
        && left.actor_principal_id() == right.actor_principal_id()
        && left.room_catalog_id() == right.room_catalog_id()
        && left.kind() == right.kind()
        && left.target() == right.target()
        && left.reason() == right.reason()
        && left.starts_at() == right.starts_at()
        && left.expires_at() == right.expires_at()
}

fn audit_metadata(room_catalog_id: Option<RoomCatalogId>) -> Value {
    room_catalog_id.map_or_else(
        || json!({}),
        |id| json!({ "roomCatalogId": id.to_string() }),
    )
}

const fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
