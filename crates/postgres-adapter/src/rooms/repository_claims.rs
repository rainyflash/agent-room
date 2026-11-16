//! 仓库认领（`specs/redesign/project-rooms.md` 的 3P-2b，表 `repository_claim`）和项目房间的仓储入口。
//!
//! - 存认领时先锁住认领的人那一行，再数他在等的，两个请求同时来也不超过上限；
//! - 开始核对时在同一条语句里看上次核对够不够久、记下这一刻（`last_checked_at`），别的控制面就不再同时
//!   核对它；第一次核对时结果先记成 `checking`，读完换成真的；
//! - 认上以后的绑定、房主解绑在 `repository_bindings.rs`。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        BindRepository, BindRepositoryOutcome, BoundRepositoryRoom, DueClaimQuery, PortFuture,
        ProjectRoomCandidate, ProjectRoomStore, RepositoryBindingRecord, RepositoryCheckRecord,
        RepositoryClaimCheckStart, RepositoryClaimInsertOutcome, RepositoryClaimRecord,
        UnbindRepositoryOutcome,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RepositoryClaimId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository},
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_optional_time, decode_time},
    error::map_sqlx_error,
};

use super::{
    decode::corrupt_data,
    repository_bindings::{
        bind_room, binding_for_room, bound_room, decode_repository, project_room_candidate,
        refresh_binding, unbind_by_owner,
    },
};

const SHARE_CODE_INDEX: &str = "repository_claim_share_code_unique";

const CLAIM_COLUMNS: &str = r"
    id,
    principal_id,
    repository_owner,
    repository_name,
    share_code,
    target_catalog_entry_id,
    floor(extract(epoch FROM claimed_at) * 1000)::bigint AS claimed_at_ms,
    floor(extract(epoch FROM last_checked_at) * 1000)::bigint AS last_checked_at_ms,
    last_check_result,
    check_count";

impl ProjectRoomStore for PostgresRepositories {
    fn insert_claim<'a>(
        &'a self,
        claim: &'a RepositoryClaimRecord,
        max_pending: u32,
    ) -> PortFuture<'a, RepositoryResult<RepositoryClaimInsertOutcome>> {
        Box::pin(async move {
            let operation = "project_room.claim.insert";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let outcome = insert_claim(&mut transaction, claim, max_pending, operation).await?;
            if matches!(outcome, RepositoryClaimInsertOutcome::Inserted(_)) {
                transaction
                    .commit()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            Ok(outcome)
        })
    }

    fn claim(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryClaimRecord>>> {
        Box::pin(async move {
            let operation = "project_room.claim.find";
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {CLAIM_COLUMNS} FROM agent_room.repository_claim WHERE id = $1"
            )))
            .bind(claim_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.as_ref()
                .map(|row| decode_claim(row, operation))
                .transpose()
        })
    }

    fn claims_of(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimRecord>>> {
        Box::pin(async move {
            let operation = "project_room.claim.list";
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {CLAIM_COLUMNS} FROM agent_room.repository_claim
                  WHERE principal_id = $1 ORDER BY claimed_at, id"
            )))
            .bind(principal_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_claim(row, operation))
                .collect()
        })
    }

    fn delete_claim(
        &self,
        claim_id: RepositoryClaimId,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "project_room.claim.delete";
            let deleted = sqlx::query(
                "DELETE FROM agent_room.repository_claim WHERE id = $1 AND principal_id = $2",
            )
            .bind(claim_id.as_uuid())
            .bind(principal_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(deleted.rows_affected() > 0)
        })
    }

    fn start_check(
        &self,
        claim_id: RepositoryClaimId,
        now: UtcMillis,
        checked_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RepositoryClaimCheckStart>> {
        Box::pin(start_check(self, claim_id, now, checked_before))
    }

    fn finish_check(
        &self,
        claim_id: RepositoryClaimId,
        started_at: UtcMillis,
        result: ClaimCheckResult,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "project_room.claim.finish_check";
            sqlx::query(
                r"UPDATE agent_room.repository_claim
                     SET last_check_result = $3
                   WHERE id = $1
                     AND last_checked_at = to_timestamp($2::double precision / 1000.0)",
            )
            .bind(claim_id.as_uuid())
            .bind(started_at.value())
            .bind(result.as_str())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn due_claims(
        &self,
        query: DueClaimQuery,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimId>>> {
        Box::pin(async move {
            let operation = "project_room.claim.due";
            // 从没核对过的按认领的时间算：刚认领的人多半还没推上去，头一次也等 5 分钟。
            let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
                r"SELECT id
                    FROM agent_room.repository_claim
                   WHERE (claimed_at > to_timestamp($1::double precision / 1000.0)
                          AND COALESCE(last_checked_at, claimed_at)
                              <= to_timestamp($2::double precision / 1000.0))
                      OR (claimed_at <= to_timestamp($1::double precision / 1000.0)
                          AND COALESCE(last_checked_at, claimed_at)
                              <= to_timestamp($3::double precision / 1000.0))
                   ORDER BY COALESCE(last_checked_at, claimed_at), id
                   LIMIT $4",
            )
            .bind(query.young_since.value())
            .bind(query.young_checked_before.value())
            .bind(query.old_checked_before.value())
            .bind(i64::from(query.limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(ids.into_iter().map(RepositoryClaimId::from_uuid).collect())
        })
    }

    fn expire_claims(&self, claimed_before: UtcMillis) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(async move {
            let operation = "project_room.claim.expire";
            let deleted = sqlx::query(
                r"DELETE FROM agent_room.repository_claim
                   WHERE claimed_at <= to_timestamp($1::double precision / 1000.0)",
            )
            .bind(claimed_before.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(deleted.rows_affected())
        })
    }

    fn project_room_candidate(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ProjectRoomCandidate>>> {
        Box::pin(project_room_candidate(self.pool(), catalog_id))
    }

    fn binding_for_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryBindingRecord>>> {
        Box::pin(binding_for_room(self.pool(), catalog_id))
    }

    fn bound_room<'a>(
        &'a self,
        repository: &'a GithubRepository,
    ) -> PortFuture<'a, RepositoryResult<Option<BoundRepositoryRoom>>> {
        Box::pin(bound_room(self.pool(), repository))
    }

    fn bind_room<'a>(
        &'a self,
        request: &'a BindRepository,
    ) -> PortFuture<'a, RepositoryResult<BindRepositoryOutcome>> {
        Box::pin(bind_room(self.pool(), request))
    }

    fn unbind_by_owner(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<UnbindRepositoryOutcome>> {
        Box::pin(unbind_by_owner(self.pool(), catalog_id, owner))
    }

    fn refresh_binding<'a>(
        &'a self,
        binding: &'a RepositoryBindingRecord,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(refresh_binding(self.pool(), binding))
    }
}

/// 锁住认领的人，看他同一个仓库是不是已经在等、一共等着几个，再存。
async fn insert_claim(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RepositoryClaimRecord,
    max_pending: u32,
    operation: &'static str,
) -> RepositoryResult<RepositoryClaimInsertOutcome> {
    sqlx::query("SELECT 1 FROM agent_room.principal WHERE id = $1 FOR UPDATE")
        .bind(claim.principal_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let existing = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {CLAIM_COLUMNS} FROM agent_room.repository_claim
          WHERE principal_id = $1 AND host = 'github'
            AND repository_owner_lower = lower($2) AND repository_name_lower = lower($3)"
    )))
    .bind(claim.principal_id.as_uuid())
    .bind(claim.repository.owner())
    .bind(claim.repository.name())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if let Some(row) = existing {
        let existing = decode_claim(&row, operation)?;
        if existing.target == claim.target {
            return Ok(RepositoryClaimInsertOutcome::Existing(existing));
        }
        // 同一个仓库换了要绑的房间：换成新的，不多占名额。
        delete_claim_row(transaction, existing.id, operation).await?;
    }
    if let Some(target) = claim.target {
        // 同一间房他只留最新的一个认领：码就是房间的分享码，两个认领不能同码。
        sqlx::query(
            r"DELETE FROM agent_room.repository_claim
               WHERE principal_id = $1 AND target_catalog_entry_id = $2",
        )
        .bind(claim.principal_id.as_uuid())
        .bind(target.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    } else {
        let listed_code: bool = sqlx::query_scalar(
            r"SELECT EXISTS (
                  SELECT 1 FROM agent_room.private_room_listing WHERE share_code = $1
              )",
        )
        .bind(claim.share_code.as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        if listed_code {
            return Ok(RepositoryClaimInsertOutcome::ShareCodeTaken);
        }
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.repository_claim WHERE principal_id = $1",
    )
    .bind(claim.principal_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if pending >= i64::from(max_pending) {
        return Ok(RepositoryClaimInsertOutcome::TooMany);
    }
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.repository_claim
              (id, principal_id, host, repository_owner, repository_name, share_code,
               target_catalog_entry_id, claimed_at)
          VALUES ($1, $2, 'github', $3, $4, $5, $6, to_timestamp($7::double precision / 1000.0))",
    )
    .bind(claim.id.as_uuid())
    .bind(claim.principal_id.as_uuid())
    .bind(claim.repository.owner())
    .bind(claim.repository.name())
    .bind(claim.share_code.as_str())
    .bind(claim.target.map(RoomCatalogId::as_uuid))
    .bind(claim.claimed_at.value())
    .execute(&mut **transaction)
    .await;
    match inserted {
        Ok(_) => Ok(RepositoryClaimInsertOutcome::Inserted(
            RepositoryClaimRecord {
                last_check: None,
                check_count: 0,
                ..claim.clone()
            },
        )),
        Err(sqlx::Error::Database(database)) if database.constraint() == Some(SHARE_CODE_INDEX) => {
            Ok(RepositoryClaimInsertOutcome::ShareCodeTaken)
        }
        Err(error) => Err(map_sqlx_error(operation, &error)),
    }
}

async fn delete_claim_row(
    transaction: &mut Transaction<'_, Postgres>,
    claim_id: RepositoryClaimId,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query("DELETE FROM agent_room.repository_claim WHERE id = $1")
        .bind(claim_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 上次核对早于 `checked_before`（或者从没核对过）才记下这一刻；没记下时回什么时候能再来。
async fn start_check(
    repositories: &PostgresRepositories,
    claim_id: RepositoryClaimId,
    now: UtcMillis,
    checked_before: UtcMillis,
) -> RepositoryResult<RepositoryClaimCheckStart> {
    let operation = "project_room.claim.start_check";
    let started = sqlx::query(sqlx::AssertSqlSafe(format!(
        r"UPDATE agent_room.repository_claim
             SET last_checked_at = to_timestamp($2::double precision / 1000.0),
                 last_check_result = COALESCE(last_check_result, 'checking'),
                 check_count = check_count + 1
           WHERE id = $1
             AND (last_checked_at IS NULL
                  OR last_checked_at <= to_timestamp($3::double precision / 1000.0))
       RETURNING {CLAIM_COLUMNS}"
    )))
    .bind(claim_id.as_uuid())
    .bind(now.value())
    .bind(checked_before.value())
    .fetch_optional(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if let Some(row) = started {
        return Ok(RepositoryClaimCheckStart::Started(decode_claim(
            &row, operation,
        )?));
    }
    let last: Option<Option<i64>> = sqlx::query_scalar(
        r"SELECT floor(extract(epoch FROM last_checked_at) * 1000)::bigint
            FROM agent_room.repository_claim WHERE id = $1",
    )
    .bind(claim_id.as_uuid())
    .fetch_optional(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some(last) = last.flatten() else {
        return Ok(RepositoryClaimCheckStart::Gone);
    };
    let interval = now.value().saturating_sub(checked_before.value());
    let retry_at = UtcMillis::new(last.saturating_add(interval).max(now.value()))
        .map_err(|_| corrupt_data(operation))?;
    Ok(RepositoryClaimCheckStart::TooSoon { retry_at })
}

fn decode_claim(row: &PgRow, operation: &'static str) -> RepositoryResult<RepositoryClaimRecord> {
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let principal_id: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let share_code: String = decode_column(row, "share_code", operation)?;
    let target: Option<uuid::Uuid> = decode_column(row, "target_catalog_entry_id", operation)?;
    let last_checked_at = decode_optional_time(row, "last_checked_at_ms", operation)?;
    let last_check_result: Option<String> = decode_column(row, "last_check_result", operation)?;
    let last_check = match (last_checked_at, last_check_result) {
        (Some(at), Some(result)) => Some(RepositoryCheckRecord {
            at,
            result: ClaimCheckResult::parse(&result).map_err(|_| corrupt_data(operation))?,
        }),
        (None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    let check_count: i32 = decode_column(row, "check_count", operation)?;
    Ok(RepositoryClaimRecord {
        id: RepositoryClaimId::from_uuid(id),
        principal_id: PrincipalId::from_uuid(principal_id),
        repository: decode_repository(row, operation)?,
        share_code: RoomShareCode::parse(&share_code).map_err(|_| corrupt_data(operation))?,
        target: target.map(RoomCatalogId::from_uuid),
        claimed_at: decode_time(row, "claimed_at_ms", operation)?,
        last_check,
        check_count: u32::try_from(check_count).map_err(|_| corrupt_data(operation))?,
    })
}
