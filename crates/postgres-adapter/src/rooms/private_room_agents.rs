//! 私人房间的 Agent 口令、凭口令进来的 Agent 成员，以及猜口令的固定窗口计数。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        JoinCodeAttemptPolicy, MatrixUserId, PortFuture, PrivateRoomAgentAccessStore,
        PrivateRoomAgentMemberRecord, PrivateRoomJoinCodeRecord, SecretDigest,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    join_codes::PrivateRoomAgentMemberStatus,
    private_rooms::PrivateRoomPermissions,
    time::UtcMillis,
};
use sqlx::postgres::PgRow;

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::decode::corrupt_data;

const JOIN_CODE_COLUMNS: &str = r"
    catalog_entry_id,
    permission_bits,
    created_by_principal_id,
    floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms";

/// 名字取 Agent 当前的显示名；主人取仍然有效的第一位所有者。
const AGENT_MEMBER_QUERY: &str = r"
    SELECT member.catalog_entry_id,
           member.agent_id,
           agent.display_name,
           agent.matrix_user_id,
           owner.display_name AS owner_display_name,
           member.membership_status,
           member.permission_bits,
           floor(extract(epoch FROM member.created_at) * 1000)::bigint AS joined_at_ms,
           floor(extract(epoch FROM member.status_changed_at) * 1000)::bigint AS status_changed_at_ms
      FROM agent_room.private_room_agent_member member
      JOIN agent_room.agent agent ON agent.id = member.agent_id
      LEFT JOIN LATERAL (
          SELECT principal.display_name
            FROM agent_room.agent_ownership ownership
            JOIN agent_room.principal principal ON principal.id = ownership.principal_id
           WHERE ownership.agent_id = member.agent_id
             AND ownership.role = 'owner'
             AND ownership.revoked_at IS NULL
           ORDER BY ownership.created_at
           LIMIT 1
      ) owner ON true
     WHERE member.catalog_entry_id = $1";

impl PrivateRoomAgentAccessStore for PostgresRepositories {
    fn join_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>> {
        Box::pin(async move {
            let operation = "private_room.join_code.find";
            let query = format!(
                "SELECT {JOIN_CODE_COLUMNS} FROM agent_room.private_room_join_code
                  WHERE catalog_entry_id = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_join_code(&row, operation)).transpose()
        })
    }

    fn replace_join_code<'a>(
        &'a self,
        record: &'a PrivateRoomJoinCodeRecord,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.join_code.replace";
            sqlx::query(
                r"INSERT INTO agent_room.private_room_join_code
                      (catalog_entry_id, code_digest, permission_bits,
                       created_by_principal_id, created_at)
                  VALUES ($1, $2, $3, $4, to_timestamp($5::double precision / 1000.0))
                  ON CONFLICT (catalog_entry_id) DO UPDATE
                     SET code_digest = EXCLUDED.code_digest,
                         permission_bits = EXCLUDED.permission_bits,
                         created_by_principal_id = EXCLUDED.created_by_principal_id,
                         created_at = EXCLUDED.created_at",
            )
            .bind(record.catalog_id.as_uuid())
            .bind(digest.as_bytes().as_slice())
            .bind(i16::from(record.permissions.bits()))
            .bind(record.created_by.as_uuid())
            .bind(record.created_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn clear_join_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "private_room.join_code.clear";
            let result = sqlx::query(
                "DELETE FROM agent_room.private_room_join_code WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn find_join_code<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>> {
        Box::pin(async move {
            let operation = "private_room.join_code.find_by_digest";
            let query = format!(
                "SELECT {JOIN_CODE_COLUMNS} FROM agent_room.private_room_join_code
                  WHERE code_digest = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(digest.as_bytes().as_slice())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_join_code(&row, operation)).transpose()
        })
    }

    fn agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>> {
        Box::pin(async move {
            let operation = "private_room.agent_member.find";
            let query = format!("{AGENT_MEMBER_QUERY} AND member.agent_id = $2");
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .bind(agent_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_agent_member(&row, operation))
                .transpose()
        })
    }

    fn agent_members(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentMemberRecord>>> {
        Box::pin(async move {
            let operation = "private_room.agent_member.list";
            let query = format!("{AGENT_MEMBER_QUERY} ORDER BY member.created_at, member.agent_id");
            let rows = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_agent_member(row, operation))
                .collect()
        })
    }

    fn admit_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        permissions: PrivateRoomPermissions,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.agent_member.admit";
            // 已加入的保持原来的状态时间；以前被移出的重新加入时从现在算起。
            sqlx::query(
                r"INSERT INTO agent_room.private_room_agent_member
                      (catalog_entry_id, agent_id, membership_status, permission_bits,
                       joined_via, created_at, status_changed_at)
                  VALUES ($1, $2, 'joined', $3, 'code',
                          to_timestamp($4::double precision / 1000.0),
                          to_timestamp($4::double precision / 1000.0))
                  ON CONFLICT (catalog_entry_id, agent_id) DO UPDATE
                     SET permission_bits = EXCLUDED.permission_bits,
                         status_changed_at = CASE
                             WHEN private_room_agent_member.membership_status = 'joined'
                             THEN private_room_agent_member.status_changed_at
                             ELSE greatest(EXCLUDED.status_changed_at,
                                           private_room_agent_member.status_changed_at)
                         END,
                         membership_status = 'joined'",
            )
            .bind(catalog_id.as_uuid())
            .bind(agent_id.as_uuid())
            .bind(i16::from(permissions.bits()))
            .bind(now.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn remove_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "private_room.agent_member.remove";
            let result = sqlx::query(
                r"UPDATE agent_room.private_room_agent_member
                     SET membership_status = 'removed',
                         permission_bits = 0,
                         status_changed_at = greatest(
                             status_changed_at,
                             to_timestamp($3::double precision / 1000.0)
                         )
                   WHERE catalog_entry_id = $1
                     AND agent_id = $2
                     AND membership_status = 'joined'",
            )
            .bind(catalog_id.as_uuid())
            .bind(agent_id.as_uuid())
            .bind(now.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn join_code_retry_at<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<Option<UtcMillis>>> {
        Box::pin(async move {
            let operation = "private_room.join_code.attempts";
            let row = sqlx::query_as::<_, (i64, i32)>(
                r"SELECT floor(extract(epoch FROM window_started_at) * 1000)::bigint,
                         failure_count
                    FROM agent_room.join_code_attempt_window
                   WHERE caller_key = $1",
            )
            .bind(caller)
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let Some((started, failures)) = row else {
                return Ok(None);
            };
            let window =
                i64::try_from(policy.window.value()).map_err(|_| corrupt_data(operation))?;
            let ends = started.saturating_add(window);
            let exhausted = u32::try_from(failures).is_ok_and(|count| count >= policy.max_failures);
            if exhausted && now.value() < ends {
                UtcMillis::new(ends)
                    .map(Some)
                    .map_err(|error| map_domain_error(operation, &error))
            } else {
                Ok(None)
            }
        })
    }

    fn record_join_code_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.join_code.record_failure";
            let window =
                i64::try_from(policy.window.value()).map_err(|_| corrupt_data(operation))?;
            // 窗口已过就从这一次重新计数。
            sqlx::query(
                r"INSERT INTO agent_room.join_code_attempt_window
                      (caller_key, window_started_at, failure_count)
                  VALUES ($1, to_timestamp($2::double precision / 1000.0), 1)
                  ON CONFLICT (caller_key) DO UPDATE
                     SET failure_count = CASE
                             WHEN join_code_attempt_window.window_started_at
                                  + make_interval(secs => $3::double precision / 1000.0)
                                  <= EXCLUDED.window_started_at
                             THEN 1
                             ELSE join_code_attempt_window.failure_count + 1
                         END,
                         window_started_at = CASE
                             WHEN join_code_attempt_window.window_started_at
                                  + make_interval(secs => $3::double precision / 1000.0)
                                  <= EXCLUDED.window_started_at
                             THEN EXCLUDED.window_started_at
                             ELSE join_code_attempt_window.window_started_at
                         END",
            )
            .bind(caller)
            .bind(now.value())
            .bind(window)
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }
}

fn decode_join_code(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomJoinCodeRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let bits: i16 = decode_column(row, "permission_bits", operation)?;
    let created_by: uuid::Uuid = decode_column(row, "created_by_principal_id", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    Ok(PrivateRoomJoinCodeRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        permissions: permissions(bits, operation)?,
        created_by: PrincipalId::from_uuid(created_by),
        created_at: UtcMillis::new(created_at)
            .map_err(|error| map_domain_error(operation, &error))?,
    })
}

fn decode_agent_member(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomAgentMemberRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let status: String = decode_column(row, "membership_status", operation)?;
    let bits: i16 = decode_column(row, "permission_bits", operation)?;
    let matrix_user_id: String = decode_column(row, "matrix_user_id", operation)?;
    let joined_at: i64 = decode_column(row, "joined_at_ms", operation)?;
    let status_changed_at: i64 = decode_column(row, "status_changed_at_ms", operation)?;
    let status =
        PrivateRoomAgentMemberStatus::parse(&status).map_err(|_| corrupt_data(operation))?;
    let permissions = if status == PrivateRoomAgentMemberStatus::Joined {
        permissions(bits, operation)?
    } else {
        PrivateRoomPermissions::NONE
    };
    Ok(PrivateRoomAgentMemberRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        agent_id: AgentId::from_uuid(agent_id),
        display_name: decode_column(row, "display_name", operation)?,
        matrix_user_id: MatrixUserId::new(matrix_user_id).map_err(|_| corrupt_data(operation))?,
        owner_display_name: decode_column(row, "owner_display_name", operation)?,
        status,
        permissions,
        joined_at: UtcMillis::new(joined_at)
            .map_err(|error| map_domain_error(operation, &error))?,
        status_changed_at: UtcMillis::new(status_changed_at)
            .map_err(|error| map_domain_error(operation, &error))?,
    })
}

fn permissions(bits: i16, operation: &'static str) -> RepositoryResult<PrivateRoomPermissions> {
    let bits = u8::try_from(bits).map_err(|_| corrupt_data(operation))?;
    PrivateRoomPermissions::from_bits(bits).map_err(|_| corrupt_data(operation))
}
