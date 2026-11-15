//! 子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，第 2e-1 步）。
//!
//! 建、改、删都先锁父房间在目录里的那一行（和建子房间、改子房间同一把）：同一间房的分类排着队改，
//! 上限、重名数得准，先后改完从 0 起连着。建和删在同一个事务里写审计，只记分类号和房间号，名字是
//! 用户写的字，不进审计。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        NewSubRoomCategory, PortFuture, SubRoomCategory, SubRoomCategoryOutcome,
        SubRoomCategoryRemoval, SubRoomCategoryRevision, SubRoomCategoryStore,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, SubRoomCategoryId},
    sub_room_categories::MAXIMUM_SUB_ROOM_CATEGORIES,
    time::UtcMillis,
};
use serde_json::json;
use sqlx::{Postgres, Transaction, postgres::PgRow};
use uuid::Uuid;

use crate::{
    PostgresRepositories, agents::decode_column, error::map_sqlx_error, transaction::finish,
};

use super::decode::corrupt_data;

impl SubRoomCategoryStore for PostgresRepositories {
    fn sub_room_categories(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomCategory>>> {
        Box::pin(async move {
            let operation = "sub_room.categories";
            let rows = sqlx::query(CATEGORIES)
                .bind(parent.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_category(row, operation))
                .collect()
        })
    }

    fn create_sub_room_category<'a>(
        &'a self,
        category: &'a NewSubRoomCategory,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.category.create";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = create(&mut transaction, category, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn revise_sub_room_category<'a>(
        &'a self,
        revision: &'a SubRoomCategoryRevision,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.category.revise";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = revise(&mut transaction, revision, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn delete_sub_room_category<'a>(
        &'a self,
        removal: &'a SubRoomCategoryRemoval,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.category.delete";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = delete(&mut transaction, removal, operation).await;
            finish(transaction, result, operation).await
        })
    }
}

const CATEGORIES: &str = r"SELECT id, name, position
                             FROM agent_room.private_room_sub_room_category
                            WHERE parent_catalog_entry_id = $1
                            ORDER BY position, created_at, id";

async fn create(
    transaction: &mut Transaction<'_, Postgres>,
    category: &NewSubRoomCategory,
    operation: &'static str,
) -> RepositoryResult<SubRoomCategoryOutcome> {
    let parent = category.parent_catalog_id;
    if !lock_parent(transaction, parent, operation).await? {
        return Ok(SubRoomCategoryOutcome::ParentUnavailable);
    }
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        r"SELECT parent_catalog_entry_id, name_key
            FROM agent_room.private_room_sub_room_category
           WHERE id = $1",
    )
    .bind(category.id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if let Some((existing_parent, existing_key)) = existing {
        // 同一个请求重发：什么都不写，回现在的样子。
        return if existing_parent == parent.as_uuid() && existing_key == category.name.name_key {
            saved(transaction, parent, operation).await
        } else {
            Ok(SubRoomCategoryOutcome::IdTaken)
        };
    }
    let (count, same_name): (i64, bool) = sqlx::query_as(
        r"SELECT count(*)::bigint, COALESCE(bool_or(name_key = $2), false)
            FROM agent_room.private_room_sub_room_category
           WHERE parent_catalog_entry_id = $1",
    )
    .bind(parent.as_uuid())
    .bind(&category.name.name_key)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if same_name {
        return Ok(SubRoomCategoryOutcome::NameTaken);
    }
    if count >= i64::from(MAXIMUM_SUB_ROOM_CATEGORIES) {
        return Ok(SubRoomCategoryOutcome::LimitReached);
    }
    sqlx::query(
        r"INSERT INTO agent_room.private_room_sub_room_category (
               id, parent_catalog_entry_id, name, name_key, position,
               created_by_principal_id, created_at, updated_at
           ) VALUES (
               $1, $2, $3, $4, $5, $6,
               to_timestamp($7::double precision / 1000.0),
               to_timestamp($7::double precision / 1000.0)
           )",
    )
    .bind(category.id.as_uuid())
    .bind(parent.as_uuid())
    .bind(&category.name.name)
    .bind(&category.name.name_key)
    .bind(i32::try_from(count).map_err(|_| constraint(operation))?)
    .bind(category.created_by.as_uuid())
    .bind(category.created_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    audit(
        transaction,
        &CategoryAudit {
            action: "sub_room.category.created",
            actor: category.created_by,
            category: category.id,
            parent,
            at: category.created_at,
        },
        operation,
    )
    .await?;
    saved(transaction, parent, operation).await
}

async fn revise(
    transaction: &mut Transaction<'_, Postgres>,
    revision: &SubRoomCategoryRevision,
    operation: &'static str,
) -> RepositoryResult<SubRoomCategoryOutcome> {
    let parent = revision.parent_catalog_id;
    if !lock_parent(transaction, parent, operation).await? {
        return Ok(SubRoomCategoryOutcome::ParentUnavailable);
    }
    let order = order(transaction, parent, operation).await?;
    if !order.contains(&revision.id) {
        return Ok(SubRoomCategoryOutcome::Missing);
    }
    if let Some(name) = &revision.name {
        let taken: bool = sqlx::query_scalar(
            r"SELECT EXISTS (
                  SELECT 1 FROM agent_room.private_room_sub_room_category
                   WHERE parent_catalog_entry_id = $1 AND name_key = $2 AND id <> $3
              )",
        )
        .bind(parent.as_uuid())
        .bind(&name.name_key)
        .bind(revision.id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        if taken {
            return Ok(SubRoomCategoryOutcome::NameTaken);
        }
        sqlx::query(
            r"UPDATE agent_room.private_room_sub_room_category
                 SET name = $2,
                     name_key = $3,
                     updated_at = GREATEST(updated_at, to_timestamp($4::double precision / 1000.0))
               WHERE id = $1",
        )
        .bind(revision.id.as_uuid())
        .bind(&name.name)
        .bind(&name.name_key)
        .bind(revision.at.value())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    if let Some(to) = revision.position {
        let mut order = order;
        order.retain(|id| *id != revision.id);
        order.insert(usize::from(to).min(order.len()), revision.id);
        renumber(transaction, &order, revision.at, operation).await?;
    }
    saved(transaction, parent, operation).await
}

async fn delete(
    transaction: &mut Transaction<'_, Postgres>,
    removal: &SubRoomCategoryRemoval,
    operation: &'static str,
) -> RepositoryResult<SubRoomCategoryOutcome> {
    let parent = removal.parent_catalog_id;
    if !lock_parent(transaction, parent, operation).await? {
        return Ok(SubRoomCategoryOutcome::ParentUnavailable);
    }
    // 子房间上的分类由外键置空，回到“未分类”。
    let deleted = sqlx::query(
        r"DELETE FROM agent_room.private_room_sub_room_category
           WHERE id = $1 AND parent_catalog_entry_id = $2",
    )
    .bind(removal.id.as_uuid())
    .bind(parent.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .rows_affected();
    if deleted > 0 {
        let order = order(transaction, parent, operation).await?;
        renumber(transaction, &order, removal.at, operation).await?;
        audit(
            transaction,
            &CategoryAudit {
                action: "sub_room.category.deleted",
                actor: removal.removed_by,
                category: removal.id,
                parent,
                at: removal.at,
            },
            operation,
        )
        .await?;
    }
    saved(transaction, parent, operation).await
}

/// 锁父房间在目录里的那一行，看它还能不能管分类：是一间还在用的私人房间，自己不是子房间。
async fn lock_parent(
    transaction: &mut Transaction<'_, Postgres>,
    parent: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let usable: Option<bool> = sqlx::query_scalar(
        r"SELECT catalog.kind = 'private_room'
                 AND catalog.status = 'active'
                 AND NOT EXISTS (
                     SELECT 1 FROM agent_room.private_room_parent AS link
                      WHERE link.catalog_entry_id = catalog.id
                 )
            FROM agent_room.room_catalog_entry AS catalog
           WHERE catalog.id = $1
             FOR NO KEY UPDATE OF catalog",
    )
    .bind(parent.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(usable == Some(true))
}

/// 这间房的分类照现在的先后。
async fn order(
    transaction: &mut Transaction<'_, Postgres>,
    parent: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Vec<SubRoomCategoryId>> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        r"SELECT id
            FROM agent_room.private_room_sub_room_category
           WHERE parent_catalog_entry_id = $1
           ORDER BY position, created_at, id",
    )
    .bind(parent.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(ids.into_iter().map(SubRoomCategoryId::from_uuid).collect())
}

/// 照给的先后从 0 起重新编号，位置变了的才写。
async fn renumber(
    transaction: &mut Transaction<'_, Postgres>,
    order: &[SubRoomCategoryId],
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let ids: Vec<Uuid> = order.iter().map(|id| id.as_uuid()).collect();
    sqlx::query(
        r"UPDATE agent_room.private_room_sub_room_category AS category
             SET position = ordered.position,
                 updated_at = GREATEST(
                     category.updated_at,
                     to_timestamp($2::double precision / 1000.0)
                 )
            FROM (
                SELECT given.id, (given.ordinality - 1)::integer AS position
                  FROM unnest($1::uuid[]) WITH ORDINALITY AS given(id, ordinality)
            ) AS ordered
           WHERE category.id = ordered.id
             AND category.position <> ordered.position",
    )
    .bind(&ids)
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn saved(
    transaction: &mut Transaction<'_, Postgres>,
    parent: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<SubRoomCategoryOutcome> {
    let rows = sqlx::query(CATEGORIES)
        .bind(parent.as_uuid())
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let categories = rows
        .iter()
        .map(|row| decode_category(row, operation))
        .collect::<RepositoryResult<_>>()?;
    Ok(SubRoomCategoryOutcome::Saved(categories))
}

struct CategoryAudit {
    action: &'static str,
    actor: PrincipalId,
    category: SubRoomCategoryId,
    parent: RoomCatalogId,
    at: UtcMillis,
}

async fn audit(
    transaction: &mut Transaction<'_, Postgres>,
    audit: &CategoryAudit,
    operation: &'static str,
) -> RepositoryResult<()> {
    let id = Uuid::now_v7();
    sqlx::query(
        r"INSERT INTO agent_room.audit_event (
               id, occurred_at, actor_kind, actor_reference, action,
               target_kind, target_reference, outcome, reason_code,
               correlation_id, metadata
           ) VALUES (
               $1, to_timestamp($2::double precision / 1000.0),
               'principal', $3, $4, 'sub_room_category', $5, 'allowed', NULL, $1, $6
           )",
    )
    .bind(id)
    .bind(audit.at.value())
    .bind(audit.actor.to_string())
    .bind(audit.action)
    .bind(audit.category.to_string())
    .bind(json!({ "roomCatalogId": audit.parent.to_string() }))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_category(row: &PgRow, operation: &'static str) -> RepositoryResult<SubRoomCategory> {
    let position: i32 = decode_column(row, "position", operation)?;
    Ok(SubRoomCategory {
        id: SubRoomCategoryId::from_uuid(decode_column(row, "id", operation)?),
        name: decode_column(row, "name", operation)?,
        position: u8::try_from(position).map_err(|_| corrupt_data(operation))?,
    })
}

const fn constraint(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Constraint)
}
