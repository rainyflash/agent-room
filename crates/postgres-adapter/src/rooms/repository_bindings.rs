//! 房间绑着的仓库（`specs/redesign/project-rooms.md`，表 `room_repository_binding`，3P-2a 建的表）：
//!
//! - 绑着仓库的公开房间不占每个账户 1 间的名额（`listing.rs` 的 `owner_lists_another`）；
//! - 绑着仓库的房间不能换分享码：仓库里那一行写着它（`listing.rs` 的 `update_listing`）；
//! - 改回私密（`save_listing`）、归档或关掉（`private_rooms.rs` 的 `write_room_rows`）、删账户
//!   （`accounts.rs` 的 `archive_private_rooms`）时在同一个事务里解绑。平台冻结、从目录里拿掉不动它。
//!
//! 3P-2b 起认上以后绑上（`bind_room`、建项目房间时的 `write_binding`）、房主自己解绑（`unbind_by_owner`）
//! 也在这里，照转让的先后锁：先锁房间目录那一行（`FOR NO KEY UPDATE`），再锁房主
//! 那一行（`principal ... FOR UPDATE`），和 `save_transferred_room`、`save_listing` 抢的是同样的行；
//! 要看公开设置的先锁那一行，绑定那一行最后动（改回私密时也是先改设置、再删绑定）。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        BindRepository, BindRepositoryOutcome, BoundRepositoryRoom, ProjectRoomCandidate,
        RepositoryBindingRecord, UnbindRepositoryOutcome,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    project_rooms::{GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgRow};

use crate::{
    agents::{decode_column, decode_time},
    error::map_sqlx_error,
};

use super::{decode::corrupt_data, listing::UNBOUND_LISTED_ROOMS_OF_OWNER};

/// 解绑这间房，有就删。回答删没删。要和改动房间的那次存放在同一个事务里。
pub(super) async fn unbind_repository(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let removed =
        sqlx::query("DELETE FROM agent_room.room_repository_binding WHERE catalog_entry_id = $1")
            .bind(catalog_id.as_uuid())
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(removed.rows_affected() > 0)
}

/// 这间房绑着仓库没有。
pub(super) async fn repository_bound<'e, E>(
    executor: E,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<bool>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM agent_room.room_repository_binding WHERE catalog_entry_id = $1
         )",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))
}

const BINDING_COLUMNS: &str = r"
    binding.catalog_entry_id,
    binding.repository_owner,
    binding.repository_name,
    binding.verified_by,
    floor(extract(epoch FROM binding.verified_at) * 1000)::bigint AS verified_at_ms,
    binding.verified_file";

const BINDING_REPOSITORY_INDEX: &str = "room_repository_binding_repository_unique";
const BINDING_PRIMARY_KEY: &str = "room_repository_binding_pkey";

/// 一间房能不能绑仓库要看的几样（3P-2b）。不是私人房间的当没有。
pub(super) async fn project_room_candidate<'e, E>(
    executor: E,
    catalog_id: RoomCatalogId,
) -> RepositoryResult<Option<ProjectRoomCandidate>>
where
    E: sqlx::PgExecutor<'e>,
{
    let operation = "project_room.candidate";
    let row = sqlx::query(
        r"SELECT catalog.owner_principal_id,
                 catalog.status = 'active' AS active,
                 EXISTS (
                     SELECT 1 FROM agent_room.private_room_parent AS parent
                      WHERE parent.catalog_entry_id = catalog.id
                 ) AS sub_room,
                 COALESCE(listing.listed, false) AS listed,
                 listing.share_code,
                 EXISTS (
                     SELECT 1 FROM agent_room.public_room_platform_hold AS hold
                      WHERE hold.catalog_entry_id = catalog.id
                 ) AS held,
                 binding.repository_owner,
                 binding.repository_name
            FROM agent_room.room_catalog_entry AS catalog
            LEFT JOIN agent_room.private_room_listing AS listing
              ON listing.catalog_entry_id = catalog.id
            LEFT JOIN agent_room.room_repository_binding AS binding
              ON binding.catalog_entry_id = catalog.id
           WHERE catalog.id = $1 AND catalog.kind = 'private_room'",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let owner: Option<uuid::Uuid> = decode_column(&row, "owner_principal_id", operation)?;
    let share_code: Option<String> = decode_column(&row, "share_code", operation)?;
    let bound_owner: Option<String> = decode_column(&row, "repository_owner", operation)?;
    let bound_name: Option<String> = decode_column(&row, "repository_name", operation)?;
    let bound = match (bound_owner, bound_name) {
        (Some(owner), Some(name)) => {
            Some(GithubRepository::new(&owner, &name).map_err(|_| corrupt_data(operation))?)
        }
        _ => None,
    };
    Ok(Some(ProjectRoomCandidate {
        catalog_id,
        owner: owner.map(PrincipalId::from_uuid),
        active: decode_column(&row, "active", operation)?,
        sub_room: decode_column(&row, "sub_room", operation)?,
        listed: decode_column(&row, "listed", operation)?,
        share_code: share_code
            .map(|code| RoomShareCode::parse(&code))
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        held: decode_column(&row, "held", operation)?,
        bound,
    }))
}

pub(super) async fn binding_for_room(
    pool: &PgPool,
    catalog_id: RoomCatalogId,
) -> RepositoryResult<Option<RepositoryBindingRecord>> {
    let operation = "project_room.binding.find";
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {BINDING_COLUMNS} FROM agent_room.room_repository_binding AS binding
          WHERE binding.catalog_entry_id = $1"
    )))
    .bind(catalog_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_binding(row, operation))
        .transpose()
}

/// 这个仓库（不论大小写）绑着的那间房、它现在的分享码、平台拿掉没有。
pub(super) async fn bound_room(
    pool: &PgPool,
    repository: &GithubRepository,
) -> RepositoryResult<Option<BoundRepositoryRoom>> {
    let operation = "project_room.binding.by_repository";
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {BINDING_COLUMNS},
                listing.share_code,
                EXISTS (
                    SELECT 1 FROM agent_room.public_room_platform_hold AS hold
                     WHERE hold.catalog_entry_id = binding.catalog_entry_id
                ) AS held
           FROM agent_room.room_repository_binding AS binding
           LEFT JOIN agent_room.private_room_listing AS listing
             ON listing.catalog_entry_id = binding.catalog_entry_id
          WHERE binding.host = 'github'
            AND binding.repository_owner_lower = lower($1)
            AND binding.repository_name_lower = lower($2)"
    )))
    .bind(repository.owner())
    .bind(repository.name())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let share_code: Option<String> = decode_column(&row, "share_code", operation)?;
    Ok(Some(BoundRepositoryRoom {
        binding: decode_binding(&row, operation)?,
        share_code: share_code
            .map(|code| RoomShareCode::parse(&code))
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        held: decode_column(&row, "held", operation)?,
    }))
}

/// 认上以后绑到已有的房间上（3P-2b）。照文件开头的先后锁：先锁房间目录那一行，再看房间还能不能绑，
/// 挪走原来的绑定、写下新的、删掉认领，一个事务。
pub(super) async fn bind_room(
    pool: &PgPool,
    request: &BindRepository,
) -> RepositoryResult<BindRepositoryOutcome> {
    let operation = "project_room.binding.bind";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    sqlx::query("SELECT 1 FROM agent_room.room_catalog_entry WHERE id = $1 FOR NO KEY UPDATE")
        .bind(request.catalog_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let claim_present: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM agent_room.repository_claim WHERE id = $1)",
    )
    .bind(request.claim_id.as_uuid())
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if !claim_present {
        return Ok(BindRepositoryOutcome::ClaimGone);
    }
    let candidate = project_room_candidate(&mut *transaction, request.catalog_id).await?;
    let eligible = candidate.as_ref().is_some_and(|candidate| {
        candidate.bindable_by(request.verified_by)
            && candidate.bound.is_none()
            && candidate.share_code.as_ref() == Some(&request.share_code)
    });
    if !eligible {
        return Ok(BindRepositoryOutcome::RoomNotEligible);
    }
    let outcome = write_binding(&mut transaction, request, operation).await?;
    if outcome == BindRepositoryOutcome::Bound {
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    Ok(outcome)
}

/// 挪走原来的绑定、写下新的、删掉认领。仓库刚被别的房间绑上、这间房刚绑上别的仓库、认领刚没了，
/// 都回答没绑上，由调用方放弃这个事务。
pub(super) async fn write_binding(
    transaction: &mut Transaction<'_, Postgres>,
    request: &BindRepository,
    operation: &'static str,
) -> RepositoryResult<BindRepositoryOutcome> {
    if let Some(replacing) = request.replacing {
        sqlx::query(
            r"DELETE FROM agent_room.room_repository_binding
               WHERE catalog_entry_id = $1 AND host = 'github'
                 AND repository_owner_lower = lower($2) AND repository_name_lower = lower($3)",
        )
        .bind(replacing.as_uuid())
        .bind(request.repository.owner())
        .bind(request.repository.name())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.room_repository_binding
              (catalog_entry_id, host, repository_owner, repository_name, verified_by,
               verified_at, verified_file)
          VALUES ($1, 'github', $2, $3, $4, to_timestamp($5::double precision / 1000.0), $6)",
    )
    .bind(request.catalog_id.as_uuid())
    .bind(request.repository.owner())
    .bind(request.repository.name())
    .bind(request.verified_by.as_uuid())
    .bind(request.verified_at.value())
    .bind(request.verified_file.as_str())
    .execute(&mut **transaction)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(database))
            if database.constraint() == Some(BINDING_REPOSITORY_INDEX) =>
        {
            return Ok(BindRepositoryOutcome::RepositoryTaken);
        }
        Err(sqlx::Error::Database(database))
            if database.constraint() == Some(BINDING_PRIMARY_KEY) =>
        {
            return Ok(BindRepositoryOutcome::RoomNotEligible);
        }
        Err(error) => return Err(map_sqlx_error(operation, &error)),
    }
    let deleted = sqlx::query("DELETE FROM agent_room.repository_claim WHERE id = $1")
        .bind(request.claim_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    if deleted.rows_affected() == 0 {
        return Ok(BindRepositoryOutcome::ClaimGone);
    }
    Ok(BindRepositoryOutcome::Bound)
}

/// 房主自己解绑（3P-2b）。照文件开头的先后锁：先房间目录那一行，再房主那一行；这间房公开着、名下已经有
/// 一间别的不绑仓库的公开房间就不解（和转让多出来的一样，先改回私密一间）。
pub(super) async fn unbind_by_owner(
    pool: &PgPool,
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
) -> RepositoryResult<UnbindRepositoryOutcome> {
    let operation = "project_room.binding.unbind";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let current: Option<(Option<uuid::Uuid>, bool)> = sqlx::query_as(
        r"SELECT owner_principal_id, status = 'active'
            FROM agent_room.room_catalog_entry
           WHERE id = $1 AND kind = 'private_room'
             FOR NO KEY UPDATE",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some((current_owner, active)) = current else {
        return Ok(UnbindRepositoryOutcome::NotOwner);
    };
    if current_owner != Some(owner.as_uuid()) {
        return Ok(UnbindRepositoryOutcome::NotOwner);
    }
    sqlx::query("SELECT 1 FROM agent_room.principal WHERE id = $1 FOR UPDATE")
        .bind(owner.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    if !repository_bound(&mut *transaction, catalog_id, operation).await? {
        return Ok(UnbindRepositoryOutcome::NotBound);
    }
    let listed: Option<bool> = sqlx::query_scalar(
        "SELECT listed FROM agent_room.private_room_listing WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if active && listed == Some(true) {
        let others: Vec<uuid::Uuid> = sqlx::query_scalar(UNBOUND_LISTED_ROOMS_OF_OWNER)
            .bind(owner.as_uuid())
            .fetch_all(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        if others.iter().any(|room| *room != catalog_id.as_uuid()) {
            return Ok(UnbindRepositoryOutcome::LimitReached);
        }
    }
    unbind_repository(&mut transaction, catalog_id, operation).await?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(UnbindRepositoryOutcome::Unbound)
}

/// 房主再核对一次、找到了：记下谁、什么时候、在哪个文件里。仓库换了（被别人要走了）就不动。
pub(super) async fn refresh_binding(
    pool: &PgPool,
    binding: &RepositoryBindingRecord,
) -> RepositoryResult<bool> {
    let operation = "project_room.binding.refresh";
    let updated = sqlx::query(
        r"UPDATE agent_room.room_repository_binding
             SET verified_by = $2,
                 verified_at = to_timestamp($3::double precision / 1000.0),
                 verified_file = $4
           WHERE catalog_entry_id = $1 AND host = 'github'
             AND repository_owner_lower = lower($5) AND repository_name_lower = lower($6)",
    )
    .bind(binding.catalog_id.as_uuid())
    .bind(binding.verified_by.as_uuid())
    .bind(binding.verified_at.value())
    .bind(binding.verified_file.as_str())
    .bind(binding.repository.owner())
    .bind(binding.repository.name())
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(updated.rows_affected() > 0)
}

/// 读 `repository_owner`、`repository_name` 两列。
pub(super) fn decode_repository(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<GithubRepository> {
    let owner: String = decode_column(row, "repository_owner", operation)?;
    let name: String = decode_column(row, "repository_name", operation)?;
    GithubRepository::new(&owner, &name).map_err(|_| corrupt_data(operation))
}

fn decode_binding(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RepositoryBindingRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let verified_by: uuid::Uuid = decode_column(row, "verified_by", operation)?;
    let verified_file: String = decode_column(row, "verified_file", operation)?;
    Ok(RepositoryBindingRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        repository: decode_repository(row, operation)?,
        verified_by: PrincipalId::from_uuid(verified_by),
        verified_at: decode_time(row, "verified_at_ms", operation)?,
        verified_file: RepositoryFile::parse(&verified_file)
            .map_err(|_| corrupt_data(operation))?,
    })
}
