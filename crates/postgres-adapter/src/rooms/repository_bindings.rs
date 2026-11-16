//! 房间绑着的仓库（`specs/redesign/project-rooms.md` 的 3P-2a，表 `room_repository_binding`）。这一步还
//! 没有接口往里写，只有读和解绑：
//!
//! - 绑着仓库的公开房间不占每个账户 1 间的名额（`listing.rs` 的 `owner_lists_another`）；
//! - 绑着仓库的房间不能换分享码：仓库里那一行写着它（`listing.rs` 的 `update_listing`）；
//! - 改回私密（`save_listing`）、归档或关掉（`private_rooms.rs` 的 `write_room_rows`）、删账户
//!   （`accounts.rs` 的 `archive_private_rooms`）时在同一个事务里解绑。平台冻结、从目录里拿掉不动它。
//!
//! 以后绑上、房主自己解绑（3P-2b）照转让的先后锁：先锁房间目录那一行（`FOR NO KEY UPDATE`），再锁房主
//! 那一行（`principal ... FOR UPDATE`），和 `save_transferred_room`、`save_listing` 抢的是同样的行；
//! 要看公开设置的先锁那一行，绑定那一行最后动（改回私密时也是先改设置、再删绑定）。

use agent_room_application::persistence::RepositoryResult;
use agent_room_domain::ids::RoomCatalogId;
use sqlx::{Postgres, Transaction};

use crate::error::map_sqlx_error;

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
