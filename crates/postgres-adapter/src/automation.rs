use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AutomationConsumptionOutcome, AutomationConsumptionRequest, AutomationDecisionRecord,
        AutomationGrantRecord, AutomationGrantRepository, AutomationGrantRevocationOutcome,
        AutomationScopeAuthority, AutomationScopeAuthorityRequest, AutomationSendAuthority,
        AutomationSendAuthorityRequest, MatrixUserId, PortFuture,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, AutomationGrantId, PrincipalId, RoomCatalogId},
    policy::{
        AutomationAudience, AutomationGrant, AutomationGrantDecision, AutomationGrantFields,
        AutomationGrantLimits, AutomationGrantScope, AutomationGrantStatus, AutomationMessageKind,
        AutomationMessageKinds, AutomationRiskScanOutcome, AutomationUsageSnapshot,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    error::{map_domain_error, map_sqlx_error},
    transaction,
};

const GRANT_COLUMNS: &str = r"ag.id, ag.principal_id, ag.agent_id,
       ag.agent_instance_id, ag.room_catalog_id, ag.allowed_message_kinds,
       ag.max_messages_per_minute, ag.max_total_messages,
       ag.allow_unknown_recipients, ag.requires_risk_scan,
       floor(extract(epoch FROM ag.starts_at) * 1000)::bigint AS starts_at_ms,
       floor(extract(epoch FROM ag.expires_at) * 1000)::bigint AS expires_at_ms,
       ag.state,
       floor(extract(epoch FROM ag.created_at) * 1000)::bigint AS created_at_ms,
       floor(extract(epoch FROM ag.revoked_at) * 1000)::bigint AS revoked_at_ms,
       ag.version,
       usage.total_messages,
       usage.messages_in_current_minute";

const GRANT_USAGE_JOIN: &str = r"LEFT JOIN LATERAL (
           SELECT count(consumption.submission_id)::bigint AS total_messages,
                  count(consumption.submission_id) FILTER (
                      WHERE consumption.minute_window_start =
                          to_timestamp($2::double precision / 1000.0)
                  )::bigint AS messages_in_current_minute
           FROM agent_room.automation_consumption AS consumption
           WHERE consumption.grant_id = ag.id
       ) AS usage ON true";

impl AutomationGrantRepository for PostgresRepositories {
    fn create<'a>(
        &'a self,
        grant: &'a AutomationGrant,
    ) -> PortFuture<'a, RepositoryResult<AutomationGrantRecord>> {
        Box::pin(async move { self.create_automation_grant(grant).await })
    }

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<AutomationGrantRecord>>> {
        Box::pin(async move {
            expire_grants(&self.pool, Some(principal_id), None, now).await?;
            list_grants(&self.pool, principal_id, now).await
        })
    }

    fn find(
        &self,
        grant_id: AutomationGrantId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AutomationGrantRecord>>> {
        Box::pin(async move {
            expire_grants(&self.pool, None, Some(grant_id), now).await?;
            find_grant(&self.pool, grant_id, now).await
        })
    }

    fn revoke(
        &self,
        principal_id: PrincipalId,
        grant_id: AutomationGrantId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<AutomationGrantRevocationOutcome>> {
        Box::pin(async move {
            self.revoke_automation_grant(principal_id, grant_id, revoked_at)
                .await
        })
    }

    fn consume<'a>(
        &'a self,
        request: &'a AutomationConsumptionRequest,
    ) -> PortFuture<'a, RepositoryResult<AutomationConsumptionOutcome>> {
        Box::pin(async move { self.consume_automation_grant(request).await })
    }

    fn record_decision<'a>(
        &'a self,
        record: &'a AutomationDecisionRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move { record_denial(&self.pool, record).await })
    }
}

impl AutomationScopeAuthority for PostgresRepositories {
    fn may_create<'a>(
        &'a self,
        request: &'a AutomationScopeAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move { may_create_grant(&self.pool, request).await })
    }

    fn inspect_send<'a>(
        &'a self,
        request: &'a AutomationSendAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<Option<AutomationSendAuthority>>> {
        Box::pin(async move { inspect_send_authority(&self.pool, request).await })
    }
}

impl PostgresRepositories {
    async fn create_automation_grant(
        &self,
        grant: &AutomationGrant,
    ) -> RepositoryResult<AutomationGrantRecord> {
        let operation = "automation_grant.create";
        let scope = grant.scope();
        let limits = grant.limits();
        let message_kinds = scope
            .message_kinds()
            .iter()
            .map(AutomationMessageKind::as_str)
            .collect::<Vec<_>>();
        let max_total_messages = limits
            .max_total_messages()
            .map(i32::try_from)
            .transpose()
            .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::Constraint))?;
        sqlx::query(
            r"INSERT INTO agent_room.automation_grant (
                id, principal_id, agent_id, agent_instance_id, room_catalog_id,
                allowed_message_kinds, max_messages_per_minute, max_total_messages,
                allow_unknown_recipients, requires_risk_scan, starts_at, expires_at,
                state, created_at, revoked_at, version
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                to_timestamp($11::double precision / 1000.0),
                to_timestamp($12::double precision / 1000.0),
                $13, to_timestamp($14::double precision / 1000.0), NULL, $15
            ) ON CONFLICT (id) DO NOTHING",
        )
        .bind(grant.id().as_uuid())
        .bind(grant.grantor_id().as_uuid())
        .bind(scope.agent_id().as_uuid())
        .bind(scope.agent_instance_id().map(AgentInstanceId::as_uuid))
        .bind(scope.room_catalog_id().as_uuid())
        .bind(message_kinds)
        .bind(i32::from(limits.max_messages_per_minute()))
        .bind(max_total_messages)
        .bind(scope.audience().allows_unknown_recipients())
        .bind(scope.requires_risk_scan())
        .bind(limits.starts_at().value())
        .bind(limits.expires_at().value())
        .bind(grant.status().as_str())
        .bind(grant.created_at().value())
        .bind(grant.version().value())
        .execute(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;

        let record = find_grant(&self.pool, grant.id(), grant.created_at())
            .await?
            .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::Unavailable))?;
        if !same_creation(&record.grant, grant) {
            return Err(RepositoryError::new(
                operation,
                RepositoryErrorKind::Conflict,
            ));
        }
        Ok(record)
    }

    async fn revoke_automation_grant(
        &self,
        principal_id: PrincipalId,
        grant_id: AutomationGrantId,
        revoked_at: UtcMillis,
    ) -> RepositoryResult<AutomationGrantRevocationOutcome> {
        let operation = "automation_grant.revoke";
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            expire_grant_in_transaction(&mut transaction, grant_id, revoked_at).await?;
            let Some(mut record) =
                find_grant_for_update(&mut transaction, grant_id, revoked_at).await?
            else {
                return Ok(AutomationGrantRevocationOutcome::NotFound);
            };
            if record.grant.grantor_id() != principal_id {
                return Ok(AutomationGrantRevocationOutcome::NotFound);
            }
            match record.grant.status() {
                AutomationGrantStatus::Revoked => {
                    return Ok(AutomationGrantRevocationOutcome::AlreadyRevoked(record));
                }
                AutomationGrantStatus::Exhausted | AutomationGrantStatus::Expired => {
                    return Ok(AutomationGrantRevocationOutcome::AlreadyInactive(record));
                }
                AutomationGrantStatus::Active => {}
            }
            let expected_version = record.grant.version();
            record
                .grant
                .revoke(revoked_at)
                .map_err(|error| map_domain_error(operation, &error))?;
            let updated = sqlx::query(
                r"UPDATE agent_room.automation_grant
                  SET state = 'revoked',
                      revoked_at = to_timestamp($3::double precision / 1000.0),
                      version = $4
                  WHERE id = $1 AND principal_id = $2 AND version = $5",
            )
            .bind(grant_id.as_uuid())
            .bind(principal_id.as_uuid())
            .bind(revoked_at.value())
            .bind(record.grant.version().value())
            .bind(expected_version.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if updated.rows_affected() != 1 {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Conflict,
                ));
            }
            let record = find_grant_for_update(&mut transaction, grant_id, revoked_at)
                .await?
                .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::Unavailable))?;
            Ok(AutomationGrantRevocationOutcome::Revoked(record))
        }
        .await;
        transaction::finish(transaction, result, operation).await
    }

    async fn consume_automation_grant(
        &self,
        request: &AutomationConsumptionRequest,
    ) -> RepositoryResult<AutomationConsumptionOutcome> {
        let operation = "automation_grant.consume";
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = consume_in_transaction(&mut transaction, request).await;
        transaction::finish(transaction, result, operation).await
    }
}

async fn consume_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    request: &AutomationConsumptionRequest,
) -> RepositoryResult<AutomationConsumptionOutcome> {
    const OPERATION: &str = "automation_grant.consume";
    if let Some(existing) = find_consumption(transaction, request.submission_id.as_uuid()).await? {
        if !consumption_matches(&existing, request) {
            return Err(RepositoryError::new(
                OPERATION,
                RepositoryErrorKind::Conflict,
            ));
        }
        let record = find_grant_in_transaction(transaction, request.grant_id, request.attempt.now)
            .await?
            .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::CorruptData))?;
        return Ok(AutomationConsumptionOutcome::Consumed {
            record,
            reused: true,
        });
    }

    expire_grant_in_transaction(transaction, request.grant_id, request.attempt.now).await?;
    let Some(record) =
        find_grant_for_update(transaction, request.grant_id, request.attempt.now).await?
    else {
        return Ok(AutomationConsumptionOutcome::NotFound);
    };
    // 相同提交可能在等待授权行锁期间已经被另一事务消费；拿锁后必须重查，
    // 否则会把安全重试错误地变成唯一键冲突。
    if let Some(existing) = find_consumption(transaction, request.submission_id.as_uuid()).await? {
        if !consumption_matches(&existing, request) {
            return Err(RepositoryError::new(
                OPERATION,
                RepositoryErrorKind::Conflict,
            ));
        }
        return Ok(AutomationConsumptionOutcome::Consumed {
            record,
            reused: true,
        });
    }
    if let AutomationGrantDecision::Denied(reason) =
        record.grant.evaluate(&request.attempt, record.usage)
    {
        insert_consumption_denial(transaction, request, &record, reason.as_str()).await?;
        return Ok(AutomationConsumptionOutcome::Denied(reason));
    }

    let minute_start = minute_window_start(request.attempt.now);
    sqlx::query(
        r"INSERT INTO agent_room.automation_consumption (
            submission_id, grant_id, agent_id, agent_instance_id, room_catalog_id,
            matrix_room_id, message_kind, contains_unknown_recipients,
            risk_scan_outcome, minute_window_start, consumed_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9,
            to_timestamp($10::double precision / 1000.0),
            to_timestamp($11::double precision / 1000.0)
        )",
    )
    .bind(request.submission_id.as_uuid())
    .bind(request.grant_id.as_uuid())
    .bind(request.attempt.agent_id.as_uuid())
    .bind(
        request
            .attempt
            .agent_instance_id
            .map(AgentInstanceId::as_uuid)
            .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Constraint))?,
    )
    .bind(request.attempt.room_catalog_id.as_uuid())
    .bind(request.matrix_room_id.as_str())
    .bind(request.attempt.message_kind.as_str())
    .bind(request.attempt.contains_unknown_recipients)
    .bind(request.attempt.risk_scan.as_str())
    .bind(minute_start.value())
    .bind(request.attempt.now.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;

    if record
        .grant
        .limits()
        .max_total_messages()
        .is_some_and(|maximum| record.usage.total_messages.saturating_add(1) >= maximum)
    {
        sqlx::query(
            r"UPDATE agent_room.automation_grant
              SET state = 'exhausted', version = version + 1
              WHERE id = $1 AND state = 'active'",
        )
        .bind(request.grant_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    }
    let record = find_grant_for_update(transaction, request.grant_id, request.attempt.now)
        .await?
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::CorruptData))?;
    Ok(AutomationConsumptionOutcome::Consumed {
        record,
        reused: false,
    })
}

async fn insert_consumption_denial(
    transaction: &mut Transaction<'_, Postgres>,
    request: &AutomationConsumptionRequest,
    record: &AutomationGrantRecord,
    decision_code: &'static str,
) -> RepositoryResult<()> {
    let Some(instance_id) = request.attempt.agent_instance_id else {
        return Err(RepositoryError::new(
            "automation_grant.consume.denial",
            RepositoryErrorKind::Constraint,
        ));
    };
    let decision = AutomationDecisionRecord {
        grant_id: request.grant_id,
        submission_id: request.submission_id,
        principal_id: record.grant.grantor_id(),
        agent_id: request.attempt.agent_id,
        agent_instance_id: instance_id,
        room_catalog_id: request.attempt.room_catalog_id,
        matrix_room_id: request.matrix_room_id.clone(),
        decision_code,
        decided_at: request.attempt.now,
    };
    insert_denial(transaction, &decision).await
}

async fn may_create_grant(
    pool: &PgPool,
    request: &AutomationScopeAuthorityRequest,
) -> RepositoryResult<bool> {
    let allowed: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
            SELECT 1
            FROM agent_room.principal AS principal
            JOIN agent_room.agent_ownership AS ownership
              ON ownership.principal_id = principal.id
             AND ownership.agent_id = $2
             AND ownership.role IN ('owner', 'operator')
             AND ownership.revoked_at IS NULL
            JOIN agent_room.agent AS agent
              ON agent.id = ownership.agent_id
             AND agent.lifecycle_state = 'active'
            JOIN agent_room.room_catalog_entry AS catalog
              ON catalog.id = $4 AND catalog.status = 'active'
            WHERE principal.id = $1 AND principal.status = 'active'
              AND (
                $3::uuid IS NULL OR EXISTS (
                    SELECT 1
                    FROM agent_room.agent_instance AS instance
                    JOIN agent_room.device AS device ON device.id = instance.device_id
                    WHERE instance.id = $3
                      AND instance.agent_id = agent.id
                      AND instance.status <> 'revoked'
                      AND instance.revoked_at IS NULL
                      AND device.principal_id = principal.id
                      AND device.trust_state = 'verified'
                      AND device.revoked_at IS NULL
                )
              )
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
                          AND (membership.permission_bits & 16) = 16
                    )
                )
                OR (
                    catalog.kind = 'direct'
                    AND EXISTS (
                        SELECT 1
                        FROM agent_room.direct_session AS direct
                        WHERE direct.catalog_entry_id = catalog.id
                          AND direct.target_agent_id = agent.id
                          AND direct.lifecycle_state = 'active'
                          AND NOT EXISTS (
                              SELECT 1
                              FROM agent_room.direct_contact_block AS block
                              WHERE block.principal_id = direct.principal_id
                                AND block.agent_id = direct.target_agent_id
                                AND block.revoked_at IS NULL
                          )
                    )
                )
              )
        )",
    )
    .bind(request.principal_id.as_uuid())
    .bind(request.agent_id.as_uuid())
    .bind(request.agent_instance_id.map(AgentInstanceId::as_uuid))
    .bind(request.room_catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(|error| map_sqlx_error("automation_authority.create", &error))?;
    Ok(allowed)
}

async fn inspect_send_authority(
    pool: &PgPool,
    request: &AutomationSendAuthorityRequest,
) -> RepositoryResult<Option<AutomationSendAuthority>> {
    let row = sqlx::query(
        r"SELECT agent.matrix_user_id, catalog.kind
          FROM agent_room.principal AS principal
          JOIN agent_room.device AS device
            ON device.id = $2
           AND device.principal_id = principal.id
           AND device.trust_state = 'verified'
           AND device.revoked_at IS NULL
          JOIN agent_room.agent_ownership AS ownership
            ON ownership.principal_id = principal.id
           AND ownership.agent_id = $3
           AND ownership.role IN ('owner', 'operator')
           AND ownership.revoked_at IS NULL
          JOIN agent_room.agent AS agent
            ON agent.id = ownership.agent_id
           AND agent.lifecycle_state = 'active'
          JOIN agent_room.agent_instance AS instance
            ON instance.id = $4
           AND instance.agent_id = agent.id
           AND instance.device_id = device.id
           AND instance.status = 'online'
           AND instance.revoked_at IS NULL
           AND instance.lease_expires_at > clock_timestamp()
          JOIN agent_room.room_catalog_entry AS catalog
            ON catalog.id = $5 AND catalog.status = 'active'
          JOIN agent_room.room_instance AS room
            ON room.catalog_entry_id = catalog.id
           AND room.matrix_room_id = $6
           AND room.state = 'active'
          WHERE principal.id = $1 AND principal.status = 'active'
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
                      AND (membership.permission_bits & 16) = 16
                )
              )
              OR (
                catalog.kind = 'direct'
                AND EXISTS (
                    SELECT 1
                    FROM agent_room.direct_session AS direct
                    WHERE direct.catalog_entry_id = catalog.id
                      AND direct.target_agent_id = agent.id
                      AND direct.lifecycle_state = 'active'
                      AND NOT EXISTS (
                          SELECT 1
                          FROM agent_room.direct_contact_block AS block
                          WHERE block.principal_id = direct.principal_id
                            AND block.agent_id = direct.target_agent_id
                            AND block.revoked_at IS NULL
                      )
                )
              )
            )",
    )
    .bind(request.principal_id.as_uuid())
    .bind(request.device_id.as_uuid())
    .bind(request.agent_id.as_uuid())
    .bind(request.agent_instance_id.as_uuid())
    .bind(request.room_catalog_id.as_uuid())
    .bind(request.matrix_room_id.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error("automation_authority.send", &error))?;
    row.map(|row| {
        let matrix_user_id: String =
            decode_column(&row, "matrix_user_id", "automation_authority.send")?;
        let kind: String = decode_column(&row, "kind", "automation_authority.send")?;
        Ok(AutomationSendAuthority {
            agent_matrix_user_id: MatrixUserId::new(matrix_user_id).map_err(|_| {
                RepositoryError::new(
                    "automation_authority.send",
                    RepositoryErrorKind::CorruptData,
                )
            })?,
            contains_unknown_recipients: kind != "private_room",
        })
    })
    .transpose()
}

async fn expire_grants(
    pool: &PgPool,
    principal_id: Option<PrincipalId>,
    grant_id: Option<AutomationGrantId>,
    now: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.automation_grant
          SET state = 'expired', version = version + 1
          WHERE state = 'active'
            AND expires_at <= to_timestamp($1::double precision / 1000.0)
            AND ($2::uuid IS NULL OR principal_id = $2)
            AND ($3::uuid IS NULL OR id = $3)",
    )
    .bind(now.value())
    .bind(principal_id.map(PrincipalId::as_uuid))
    .bind(grant_id.map(AutomationGrantId::as_uuid))
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error("automation_grant.expire", &error))?;
    Ok(())
}

async fn expire_grant_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    grant_id: AutomationGrantId,
    now: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.automation_grant
          SET state = 'expired', version = version + 1
          WHERE id = $1 AND state = 'active'
            AND expires_at <= to_timestamp($2::double precision / 1000.0)",
    )
    .bind(grant_id.as_uuid())
    .bind(now.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("automation_grant.expire", &error))?;
    Ok(())
}

async fn list_grants(
    pool: &PgPool,
    principal_id: PrincipalId,
    now: UtcMillis,
) -> RepositoryResult<Vec<AutomationGrantRecord>> {
    let query = format!(
        r"SELECT {GRANT_COLUMNS}
          FROM agent_room.automation_grant AS ag
          {GRANT_USAGE_JOIN}
          WHERE ag.principal_id = $1
          ORDER BY ag.created_at DESC, ag.id"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(principal_id.as_uuid())
        .bind(minute_window_start(now).value())
        .fetch_all(pool)
        .await
        .map_err(|error| map_sqlx_error("automation_grant.list", &error))?;
    rows.iter()
        .map(|row| decode_grant_record(row, "automation_grant.list"))
        .collect()
}

async fn find_grant(
    pool: &PgPool,
    grant_id: AutomationGrantId,
    now: UtcMillis,
) -> RepositoryResult<Option<AutomationGrantRecord>> {
    let query = grant_find_query();
    let row = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(grant_id.as_uuid())
        .bind(minute_window_start(now).value())
        .fetch_optional(pool)
        .await
        .map_err(|error| map_sqlx_error("automation_grant.find", &error))?;
    row.as_ref()
        .map(|row| decode_grant_record(row, "automation_grant.find"))
        .transpose()
}

async fn find_grant_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    grant_id: AutomationGrantId,
    now: UtcMillis,
) -> RepositoryResult<Option<AutomationGrantRecord>> {
    let query = grant_find_query();
    let row = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(grant_id.as_uuid())
        .bind(minute_window_start(now).value())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("automation_grant.find", &error))?;
    row.as_ref()
        .map(|row| decode_grant_record(row, "automation_grant.find"))
        .transpose()
}

async fn find_grant_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    grant_id: AutomationGrantId,
    now: UtcMillis,
) -> RepositoryResult<Option<AutomationGrantRecord>> {
    let locked_id: Option<uuid::Uuid> = sqlx::query_scalar(
        r"SELECT id
          FROM agent_room.automation_grant
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(grant_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("automation_grant.lock", &error))?;
    if locked_id.is_none() {
        return Ok(None);
    }
    // 锁成功后的新语句会取得新快照，因此能看到上一位持锁者刚提交的消费记录。
    find_grant_in_transaction(transaction, grant_id, now).await
}

fn grant_find_query() -> String {
    format!(
        r"SELECT {GRANT_COLUMNS}
          FROM agent_room.automation_grant AS ag
          {GRANT_USAGE_JOIN}
          WHERE ag.id = $1"
    )
}

fn decode_grant_record(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<AutomationGrantRecord> {
    let id = AutomationGrantId::from_uuid(decode_column(row, "id", operation)?);
    let grantor_id = PrincipalId::from_uuid(decode_column(row, "principal_id", operation)?);
    let agent_id = AgentId::from_uuid(decode_column(row, "agent_id", operation)?);
    let instance_id =
        decode_optional_uuid(row, "agent_instance_id", operation)?.map(AgentInstanceId::from_uuid);
    let room_catalog_id =
        RoomCatalogId::from_uuid(decode_column(row, "room_catalog_id", operation)?);
    let kinds = decode_column::<Vec<String>>(row, "allowed_message_kinds", operation)?
        .iter()
        .map(|kind| AutomationMessageKind::try_from(kind.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt_data(operation))?;
    let message_kinds = AutomationMessageKinds::new(kinds).map_err(|_| corrupt_data(operation))?;
    let allow_unknown: bool = decode_column(row, "allow_unknown_recipients", operation)?;
    let audience = if allow_unknown {
        AutomationAudience::AnyRoomMember
    } else {
        AutomationAudience::KnownRoomMembers
    };
    let requires_risk_scan = decode_column(row, "requires_risk_scan", operation)?;
    let scope = AutomationGrantScope::new(
        agent_id,
        instance_id,
        room_catalog_id,
        message_kinds,
        audience,
        requires_risk_scan,
    )
    .map_err(|_| corrupt_data(operation))?;
    let rate: i32 = decode_column(row, "max_messages_per_minute", operation)?;
    let total: Option<i32> = row
        .try_get("max_total_messages")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let limits = AutomationGrantLimits::new(
        u16::try_from(rate).map_err(|_| corrupt_data(operation))?,
        total
            .map(u32::try_from)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        decode_time(row, "starts_at_ms", operation)?,
        decode_time(row, "expires_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))?;
    let status: String = decode_column(row, "state", operation)?;
    let status =
        AutomationGrantStatus::try_from(status.as_str()).map_err(|_| corrupt_data(operation))?;
    let version = AggregateVersion::new(decode_column(row, "version", operation)?)
        .map_err(|_| corrupt_data(operation))?;
    let grant = AutomationGrant::restore(
        AutomationGrantFields {
            id,
            grantor_id,
            scope,
            limits,
            created_at: decode_time(row, "created_at_ms", operation)?,
        },
        status,
        decode_optional_time(row, "revoked_at_ms", operation)?,
        version,
    )
    .map_err(|_| corrupt_data(operation))?;
    let total_messages: i64 = decode_column(row, "total_messages", operation)?;
    let messages_in_current_minute: i64 =
        decode_column(row, "messages_in_current_minute", operation)?;
    Ok(AutomationGrantRecord {
        grant,
        usage: AutomationUsageSnapshot {
            total_messages: u32::try_from(total_messages).map_err(|_| corrupt_data(operation))?,
            messages_in_current_minute: u32::try_from(messages_in_current_minute)
                .map_err(|_| corrupt_data(operation))?,
        },
    })
}

struct ExistingConsumption {
    grant_id: AutomationGrantId,
    agent_id: AgentId,
    agent_instance_id: AgentInstanceId,
    room_catalog_id: RoomCatalogId,
    matrix_room_id: String,
    message_kind: AutomationMessageKind,
    contains_unknown_recipients: bool,
    risk_scan: AutomationRiskScanOutcome,
}

async fn find_consumption(
    transaction: &mut Transaction<'_, Postgres>,
    submission_id: uuid::Uuid,
) -> RepositoryResult<Option<ExistingConsumption>> {
    let operation = "automation_grant.consume.find";
    let row = sqlx::query(
        r"SELECT grant_id, agent_id, agent_instance_id, room_catalog_id,
                 matrix_room_id, message_kind, contains_unknown_recipients,
                 risk_scan_outcome
          FROM agent_room.automation_consumption
          WHERE submission_id = $1",
    )
    .bind(submission_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| {
        let kind: String = decode_column(&row, "message_kind", operation)?;
        let risk_scan: String = decode_column(&row, "risk_scan_outcome", operation)?;
        Ok(ExistingConsumption {
            grant_id: AutomationGrantId::from_uuid(decode_column(&row, "grant_id", operation)?),
            agent_id: AgentId::from_uuid(decode_column(&row, "agent_id", operation)?),
            agent_instance_id: AgentInstanceId::from_uuid(decode_column(
                &row,
                "agent_instance_id",
                operation,
            )?),
            room_catalog_id: RoomCatalogId::from_uuid(decode_column(
                &row,
                "room_catalog_id",
                operation,
            )?),
            matrix_room_id: decode_column(&row, "matrix_room_id", operation)?,
            message_kind: AutomationMessageKind::try_from(kind.as_str())
                .map_err(|_| corrupt_data(operation))?,
            contains_unknown_recipients: decode_column(
                &row,
                "contains_unknown_recipients",
                operation,
            )?,
            risk_scan: AutomationRiskScanOutcome::try_from(risk_scan.as_str())
                .map_err(|_| corrupt_data(operation))?,
        })
    })
    .transpose()
}

fn consumption_matches(
    existing: &ExistingConsumption,
    request: &AutomationConsumptionRequest,
) -> bool {
    existing.grant_id == request.grant_id
        && existing.agent_id == request.attempt.agent_id
        && Some(existing.agent_instance_id) == request.attempt.agent_instance_id
        && existing.room_catalog_id == request.attempt.room_catalog_id
        && existing.matrix_room_id == request.matrix_room_id.as_str()
        && existing.message_kind == request.attempt.message_kind
        && existing.contains_unknown_recipients == request.attempt.contains_unknown_recipients
        && existing.risk_scan == request.attempt.risk_scan
}

async fn record_denial(pool: &PgPool, record: &AutomationDecisionRecord) -> RepositoryResult<()> {
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.automation_denial (
            grant_id, submission_id, decision_code, principal_id, agent_id,
            agent_instance_id, room_catalog_id, matrix_room_id, decided_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            to_timestamp($9::double precision / 1000.0)
        ) ON CONFLICT (grant_id, submission_id, decision_code) DO NOTHING",
    )
    .bind(record.grant_id.as_uuid())
    .bind(record.submission_id.as_uuid())
    .bind(record.decision_code)
    .bind(record.principal_id.as_uuid())
    .bind(record.agent_id.as_uuid())
    .bind(record.agent_instance_id.as_uuid())
    .bind(record.room_catalog_id.as_uuid())
    .bind(record.matrix_room_id.as_str())
    .bind(record.decided_at.value())
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error("automation_denial.record", &error))?;
    if inserted.rows_affected() == 1 || denial_matches(pool, record).await? {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "automation_denial.record",
            RepositoryErrorKind::Conflict,
        ))
    }
}

async fn insert_denial(
    transaction: &mut Transaction<'_, Postgres>,
    record: &AutomationDecisionRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.automation_denial (
            grant_id, submission_id, decision_code, principal_id, agent_id,
            agent_instance_id, room_catalog_id, matrix_room_id, decided_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            to_timestamp($9::double precision / 1000.0)
        ) ON CONFLICT (grant_id, submission_id, decision_code) DO NOTHING",
    )
    .bind(record.grant_id.as_uuid())
    .bind(record.submission_id.as_uuid())
    .bind(record.decision_code)
    .bind(record.principal_id.as_uuid())
    .bind(record.agent_id.as_uuid())
    .bind(record.agent_instance_id.as_uuid())
    .bind(record.room_catalog_id.as_uuid())
    .bind(record.matrix_room_id.as_str())
    .bind(record.decided_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("automation_denial.record", &error))?;
    Ok(())
}

async fn denial_matches(
    pool: &PgPool,
    record: &AutomationDecisionRecord,
) -> RepositoryResult<bool> {
    let matches: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
            SELECT 1 FROM agent_room.automation_denial
            WHERE grant_id = $1 AND submission_id = $2 AND decision_code = $3
              AND principal_id = $4 AND agent_id = $5 AND agent_instance_id = $6
              AND room_catalog_id = $7 AND matrix_room_id = $8
              AND decided_at = to_timestamp($9::double precision / 1000.0)
        )",
    )
    .bind(record.grant_id.as_uuid())
    .bind(record.submission_id.as_uuid())
    .bind(record.decision_code)
    .bind(record.principal_id.as_uuid())
    .bind(record.agent_id.as_uuid())
    .bind(record.agent_instance_id.as_uuid())
    .bind(record.room_catalog_id.as_uuid())
    .bind(record.matrix_room_id.as_str())
    .bind(record.decided_at.value())
    .fetch_one(pool)
    .await
    .map_err(|error| map_sqlx_error("automation_denial.find", &error))?;
    Ok(matches)
}

fn same_creation(existing: &AutomationGrant, proposed: &AutomationGrant) -> bool {
    existing.id() == proposed.id()
        && existing.grantor_id() == proposed.grantor_id()
        && existing.scope() == proposed.scope()
        && existing.limits() == proposed.limits()
        && existing.created_at() == proposed.created_at()
}

fn minute_window_start(now: UtcMillis) -> UtcMillis {
    UtcMillis::new(now.value() - now.value().rem_euclid(60_000))
        .expect("非负 Unix 时间向下取整仍然非负")
}

fn decode_time(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<UtcMillis> {
    let value: i64 = decode_column(row, column, operation)?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}

fn decode_optional_time(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<UtcMillis>> {
    let value: Option<i64> = row
        .try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))?;
    value
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| corrupt_data(operation))
}

fn decode_optional_uuid(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<uuid::Uuid>> {
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn decode_column<T>(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<T>
where
    for<'decode> T: sqlx::Decode<'decode, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

const fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
