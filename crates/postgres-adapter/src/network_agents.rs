//! 只凭网络接入的 Agent：身份、封存的秘密和按用途的固定窗口计数（ADR 0010）。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        NetworkAgentActivation, NetworkAgentBeginOutcome, NetworkAgentLookup,
        NetworkAgentProvisioning, NetworkAgentRecord, NetworkAgentRoomRecord,
        NetworkAgentSecretKind, NetworkAgentStore, PortFuture, RateWindowDecision,
        RateWindowPolicy, SealedSecret, SecretDigest,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, DeviceId, NetworkAgentId, PrincipalId},
    network_agents::NetworkAgentStatus,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{corrupt_data, decode_column},
    error::{map_domain_error, map_sqlx_error},
};

/// 没停用的网络 Agent 不重名；撞上这个唯一索引时让用例换个带序号的名字再试。
const LIVE_NAME_INDEX: &str = "network_agent_live_name_unique";

const RECORD_COLUMNS: &str = r"
    id, principal_id, device_id, agent_id, agent_instance_id, display_name, status,
    floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms,
    floor(extract(epoch FROM last_active_at) * 1000)::bigint AS last_active_at_ms";

impl NetworkAgentStore for PostgresRepositories {
    fn begin(
        &self,
        provisioning: &NetworkAgentProvisioning,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentBeginOutcome>> {
        let provisioning = provisioning.clone();
        Box::pin(async move {
            let operation = "network_agent.begin";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            insert_principal(&mut transaction, &provisioning, operation).await?;
            insert_device(&mut transaction, &provisioning, operation).await?;
            let inserted = sqlx::query(
                r"INSERT INTO agent_room.network_agent (
                    id, principal_id, device_id, token_digest, display_name, status,
                    source_digest, created_at, last_active_at
                ) VALUES (
                    $1, $2, $3, $4, $5, 'provisioning', $6,
                    to_timestamp($7::double precision / 1000.0),
                    to_timestamp($7::double precision / 1000.0)
                )",
            )
            .bind(provisioning.id.as_uuid())
            .bind(provisioning.principal.principal.id().as_uuid())
            .bind(provisioning.device.id().as_uuid())
            .bind(provisioning.token_digest.as_bytes().as_slice())
            .bind(&provisioning.display_name)
            .bind(provisioning.source_digest.as_slice())
            .bind(provisioning.created_at.value())
            .execute(&mut *transaction)
            .await;
            if let Err(error) = inserted {
                // 名字被占了：整个事务回滚，主体和设备都不留下。
                if violates(&error, LIVE_NAME_INDEX) {
                    return Ok(NetworkAgentBeginOutcome::NameTaken);
                }
                return Err(map_sqlx_error(operation, &error));
            }
            for (kind, sealed) in &provisioning.secrets {
                upsert_secret(
                    &mut transaction,
                    provisioning.id,
                    *kind,
                    sealed,
                    provisioning.created_at,
                    operation,
                )
                .await?;
            }
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(NetworkAgentBeginOutcome::Created)
        })
    }

    fn activate(
        &self,
        activation: &NetworkAgentActivation,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let activation = activation.clone();
        Box::pin(async move {
            let operation = "network_agent.activate";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            // 重试时同一个 Agent 与实例再生效一次也算成功；已停用的不能复活。
            let updated = sqlx::query(
                r"UPDATE agent_room.network_agent
                     SET agent_id = $2,
                         agent_instance_id = $3,
                         status = 'active',
                         last_active_at = greatest(last_active_at,
                             to_timestamp($4::double precision / 1000.0))
                   WHERE id = $1
                     AND (status = 'provisioning'
                          OR (status = 'active' AND agent_id = $2 AND agent_instance_id = $3))",
            )
            .bind(activation.id.as_uuid())
            .bind(activation.agent_id.as_uuid())
            .bind(activation.agent_instance_id.as_uuid())
            .bind(activation.activated_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if updated.rows_affected() != 1 {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Conflict,
                ));
            }
            upsert_secret(
                &mut transaction,
                activation.id,
                NetworkAgentSecretKind::MatrixAccessToken,
                &activation.matrix_access_token,
                activation.activated_at,
                operation,
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn find_by_token<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentRecord>>> {
        Box::pin(async move {
            let operation = "network_agent.find_by_token";
            let query = format!(
                "SELECT {RECORD_COLUMNS} FROM agent_room.network_agent WHERE token_digest = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(digest.as_bytes().as_slice())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_record(&row, operation)).transpose()
        })
    }

    fn find_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedSecret>>> {
        Box::pin(async move {
            let operation = "network_agent.find_secret";
            let row = sqlx::query_as::<_, (i16, Vec<u8>)>(
                r"SELECT key_version, sealed
                    FROM agent_room.network_agent_secret
                   WHERE network_agent_id = $1 AND kind = $2",
            )
            .bind(id.as_uuid())
            .bind(kind.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|(version, bytes)| {
                Ok(SealedSecret {
                    key_version: u16::try_from(version).map_err(|_| corrupt_data(operation))?,
                    bytes,
                })
            })
            .transpose()
        })
    }

    fn count_live(&self) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(async move {
            let operation = "network_agent.count_live";
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agent_room.network_agent WHERE status <> 'disabled'",
            )
            .fetch_one(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            u64::try_from(count).map_err(|_| corrupt_data(operation))
        })
    }

    fn record_activity(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "network_agent.record_activity";
            sqlx::query(
                r"UPDATE agent_room.network_agent
                     SET last_active_at = greatest(last_active_at,
                         to_timestamp($2::double precision / 1000.0))
                   WHERE id = $1",
            )
            .bind(id.as_uuid())
            .bind(at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn disable(&self, id: NetworkAgentId, at: UtcMillis) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "network_agent.disable";
            sqlx::query(
                r"UPDATE agent_room.network_agent
                     SET status = 'disabled',
                         disabled_at = greatest(created_at, to_timestamp($2::double precision / 1000.0))
                   WHERE id = $1 AND status <> 'disabled'",
            )
            .bind(id.as_uuid())
            .bind(at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn take<'a>(
        &'a self,
        bucket: &'a str,
        now: UtcMillis,
        policy: RateWindowPolicy,
    ) -> PortFuture<'a, RepositoryResult<RateWindowDecision>> {
        Box::pin(async move {
            let operation = "network_agent.rate_window";
            let window =
                i64::try_from(policy.window.value()).map_err(|_| corrupt_data(operation))?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            // 先确保有这一行，再锁住它判断：并发的请求按顺序各记一次。
            sqlx::query(
                r"INSERT INTO agent_room.network_agent_rate_window
                      (bucket, window_started_at, event_count)
                  VALUES ($1, to_timestamp($2::double precision / 1000.0), 0)
                  ON CONFLICT (bucket) DO NOTHING",
            )
            .bind(bucket)
            .bind(now.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let (started, count) = sqlx::query_as::<_, (i64, i32)>(
                r"SELECT floor(extract(epoch FROM window_started_at) * 1000)::bigint, event_count
                    FROM agent_room.network_agent_rate_window
                   WHERE bucket = $1
                   FOR UPDATE",
            )
            .bind(bucket)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let ends = started.saturating_add(window);
            let (next_started, next_count, decision) = if now.value() >= ends {
                (now.value(), 1, RateWindowDecision::Allowed)
            } else if u32::try_from(count).is_ok_and(|count| count < policy.limit) {
                (
                    started,
                    count.saturating_add(1),
                    RateWindowDecision::Allowed,
                )
            } else {
                let retry_at =
                    UtcMillis::new(ends).map_err(|error| map_domain_error(operation, &error))?;
                (started, count, RateWindowDecision::Limited { retry_at })
            };
            sqlx::query(
                r"UPDATE agent_room.network_agent_rate_window
                     SET window_started_at = to_timestamp($2::double precision / 1000.0),
                         event_count = $3
                   WHERE bucket = $1",
            )
            .bind(bucket)
            .bind(next_started)
            .bind(next_count)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(decision)
        })
    }

    fn record_room<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(self.record_network_agent_room(id, room))
    }

    fn rooms(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentRoomRecord>>> {
        Box::pin(self.network_agent_rooms(id))
    }
}

impl NetworkAgentLookup for PostgresRepositories {
    fn network_agent_ids<'a>(
        &'a self,
        candidates: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<AgentId>>> {
        Box::pin(async move {
            if candidates.is_empty() {
                return Ok(Vec::new());
            }
            let operation = "network_agent.lookup";
            let candidates: Vec<uuid::Uuid> =
                candidates.iter().copied().map(AgentId::as_uuid).collect();
            // `network_agent.agent_id` 有唯一索引；停用的网络 Agent 也算。
            let found: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT agent_id FROM agent_room.network_agent \
                 WHERE agent_id = ANY($1) ORDER BY agent_id",
            )
            .bind(&candidates)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(found.into_iter().map(AgentId::from_uuid).collect())
        })
    }
}

async fn insert_principal(
    transaction: &mut Transaction<'_, Postgres>,
    provisioning: &NetworkAgentProvisioning,
    operation: &'static str,
) -> RepositoryResult<()> {
    let registration = &provisioning.principal;
    sqlx::query(
        r"INSERT INTO agent_room.principal (
            id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
            avatar_content_id, locale, status, created_at, updated_at, version
        ) VALUES (
            $1, $2, $3, $4, $5, NULL, $6, $7,
            to_timestamp($8::double precision / 1000.0),
            to_timestamp($8::double precision / 1000.0), $9
        )",
    )
    .bind(registration.principal.id().as_uuid())
    .bind(&registration.oidc_issuer)
    .bind(&registration.oidc_subject)
    .bind(&registration.matrix_user_id)
    .bind(&registration.display_name)
    .bind(&registration.locale)
    .bind(registration.principal.status().as_str())
    .bind(registration.registered_at.value())
    .bind(registration.principal.version().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 网络设备一建好就是已验证的：它的密钥由服务器生成，没有要证明的持有者。
async fn insert_device(
    transaction: &mut Transaction<'_, Postgres>,
    provisioning: &NetworkAgentProvisioning,
    operation: &'static str,
) -> RepositoryResult<()> {
    let device = &provisioning.device;
    sqlx::query(
        r"INSERT INTO agent_room.device (
            id, principal_id, label, platform, public_signing_key, signing_algorithm,
            trust_state, verified_at, created_at, version
        ) VALUES (
            $1, $2, $3, $4, $5, 'ed25519', 'verified',
            to_timestamp($6::double precision / 1000.0),
            to_timestamp($6::double precision / 1000.0), 0
        )",
    )
    .bind(device.id().as_uuid())
    .bind(provisioning.principal.principal.id().as_uuid())
    .bind(&provisioning.device_label)
    .bind(device.platform().as_str())
    .bind(device.public_signing_key().as_bytes().as_slice())
    .bind(device.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn upsert_secret(
    transaction: &mut Transaction<'_, Postgres>,
    id: NetworkAgentId,
    kind: NetworkAgentSecretKind,
    sealed: &SealedSecret,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let version = i16::try_from(sealed.key_version).map_err(|_| corrupt_data(operation))?;
    sqlx::query(
        r"INSERT INTO agent_room.network_agent_secret
              (network_agent_id, kind, sealed, key_version, updated_at)
          VALUES ($1, $2, $3, $4, to_timestamp($5::double precision / 1000.0))
          ON CONFLICT (network_agent_id, kind) DO UPDATE
             SET sealed = EXCLUDED.sealed,
                 key_version = EXCLUDED.key_version,
                 updated_at = EXCLUDED.updated_at",
    )
    .bind(id.as_uuid())
    .bind(kind.as_str())
    .bind(sealed.bytes.as_slice())
    .bind(version)
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn violates(error: &sqlx::Error, constraint: &str) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.constraint() == Some(constraint))
}

fn decode_record(row: &PgRow, operation: &'static str) -> RepositoryResult<NetworkAgentRecord> {
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let principal_id: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let device_id: uuid::Uuid = decode_column(row, "device_id", operation)?;
    let agent_id: Option<uuid::Uuid> = decode_column(row, "agent_id", operation)?;
    let instance_id: Option<uuid::Uuid> = decode_column(row, "agent_instance_id", operation)?;
    let display_name: String = decode_column(row, "display_name", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    let last_active_at: i64 = decode_column(row, "last_active_at_ms", operation)?;
    Ok(NetworkAgentRecord {
        id: NetworkAgentId::from_uuid(id),
        principal_id: PrincipalId::from_uuid(principal_id),
        device_id: DeviceId::from_uuid(device_id),
        agent_id: agent_id.map(AgentId::from_uuid),
        agent_instance_id: instance_id.map(AgentInstanceId::from_uuid),
        display_name,
        status: NetworkAgentStatus::parse(&status)
            .map_err(|error| map_domain_error(operation, &error))?,
        created_at: UtcMillis::new(created_at)
            .map_err(|error| map_domain_error(operation, &error))?,
        last_active_at: UtcMillis::new(last_active_at)
            .map_err(|error| map_domain_error(operation, &error))?,
    })
}
