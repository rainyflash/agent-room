//! 子房间（`specs/redesign/sub-rooms.md`）：父子关系、谁在管、一间房里子房间的三项设置。
//!
//! 建子房间、给子房间改名换门时先锁父房间在目录里的那一行：同一间房下面同时建、同时改名的排着队
//! 来，重名和上限数得准；父房间这时候归档、转让也要等这一次存完。
//!
//! 父房间变了子房间怎么跟着变在 `sub_room_follow`。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        JoinRequestRecord, NewSubRoom, PortFuture, SubRoomAdmission, SubRoomArchiveRecord,
        SubRoomCreationBlock, SubRoomCreationCheck, SubRoomDoorSource, SubRoomEntry,
        SubRoomJoinRequestOutcome, SubRoomLink, SubRoomManagerChange, SubRoomParentFacts,
        SubRoomPolicyRecord, SubRoomRevision, SubRoomRevisionBlock, SubRoomStore, SubRoomSyncScope,
        SubRoomSyncTask,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, SubRoomCategoryId},
    private_rooms::{PrivateRoom, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    room_listing::RoomJoinMode,
    rooms::MatrixRoomReference,
    sub_room_archive::SubRoomArchiveReason,
    sub_rooms::{
        MAXIMUM_OPEN_SUB_ROOMS, MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR, SUB_ROOM_AUTO_ARCHIVE_DAYS,
        SubRoomCreators, SubRoomDoor, SubRoomPolicy,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
    transaction::finish,
};

use super::{
    decode::corrupt_data,
    join_requests::void_pending_of_room,
    private_rooms::{create_private_room, ensure_creatable, save_private_room},
    role_changes::insert_role_changes,
    sub_room_follow::{follow_parent_at_creation, follow_stale_sub_rooms},
    sub_room_sync,
};

const HOUR_MS: i64 = 3_600_000;

impl SubRoomDoorSource for PostgresRepositories {
    fn sub_room_door(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomLink>>> {
        SubRoomStore::sub_room_link(self, catalog_id)
    }

    fn agent_sub_rooms(
        &self,
        parent: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        Box::pin(async move {
            let operation = "sub_room.agent_sub_rooms";
            let rooms: Vec<uuid::Uuid> = sqlx::query_scalar(
                r"SELECT link.catalog_entry_id
                    FROM agent_room.private_room_parent AS link
                    JOIN agent_room.private_room_agent_member AS member
                      ON member.catalog_entry_id = link.catalog_entry_id
                     AND member.agent_id = $2
                     AND member.membership_status = 'joined'
                   WHERE link.parent_catalog_entry_id = $1
                   ORDER BY link.created_at ASC, link.catalog_entry_id ASC",
            )
            .bind(parent.as_uuid())
            .bind(agent_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(rooms.into_iter().map(RoomCatalogId::from_uuid).collect())
        })
    }
}

impl SubRoomStore for PostgresRepositories {
    fn sub_room_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomLink>>> {
        Box::pin(async move {
            let operation = "sub_room.link";
            let row = sqlx::query(
                r"SELECT link.catalog_entry_id,
                         link.parent_catalog_entry_id,
                         link.name_key,
                         link.access,
                         link.hidden,
                         floor(extract(epoch FROM link.door_opened_at) * 1000)::bigint
                             AS door_opened_at_ms,
                         catalog.end_to_end_encrypted,
                         link.created_by_principal_id,
                         link.created_by_agent_id,
                         floor(extract(epoch FROM link.created_at) * 1000)::bigint
                             AS created_at_ms,
                         floor(extract(epoch FROM link.archived_at) * 1000)::bigint
                             AS archived_at_ms,
                         link.archived_by_principal_id,
                         link.archived_reason
                    FROM agent_room.private_room_parent AS link
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = link.catalog_entry_id
                   WHERE link.catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.as_ref()
                .map(|row| decode_link(row, operation))
                .transpose()
        })
    }

    fn sub_room_parent_facts(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomParentFacts>>> {
        Box::pin(async move {
            let operation = "sub_room.parent_facts";
            let row = sqlx::query(
                r"SELECT catalog.end_to_end_encrypted,
                         policy.creators,
                         policy.agents_may_create,
                         policy.auto_archive_days
                    FROM agent_room.room_catalog_entry AS catalog
                    LEFT JOIN agent_room.private_room_sub_room_policy AS policy
                      ON policy.catalog_entry_id = catalog.id
                   WHERE catalog.id = $1
                     AND catalog.kind = 'private_room'",
            )
            .bind(parent.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let Some(row) = row else {
                return Ok(None);
            };
            let creators: Option<String> = decode_column(&row, "creators", operation)?;
            let policy = match creators {
                Some(creators) => decode_policy(&row, &creators, operation)?,
                None => SubRoomPolicy::DEFAULT,
            };
            Ok(Some(SubRoomParentFacts {
                end_to_end_encrypted: decode_column(&row, "end_to_end_encrypted", operation)?,
                policy,
            }))
        })
    }

    fn sub_room_creation_block<'a>(
        &'a self,
        check: &'a SubRoomCreationCheck,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>> {
        Box::pin(async move {
            let operation = "sub_room.creation_block";
            // 和存的时候查的是同一段；事务什么都不写，丢掉就放开父房间那一行。
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            block(&mut transaction, check, operation).await
        })
    }

    fn create_sub_room<'a>(
        &'a self,
        sub_room: &'a NewSubRoom,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>> {
        Box::pin(async move {
            let operation = "sub_room.create";
            ensure_creatable(&sub_room.snapshot, operation)?;
            if sub_room.snapshot.catalog().id() == sub_room.parent_catalog_id {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Constraint,
                ));
            }
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let blocked = block(&mut transaction, &sub_room.check(), operation).await?;
            if blocked.is_some() {
                // 什么都没写，事务丢掉就是回滚。
                return Ok(blocked);
            }
            create_private_room(
                &mut transaction,
                &sub_room.snapshot,
                sub_room.created_at,
                sub_room.end_to_end_encrypted,
                operation,
            )
            .await?;
            insert_link(&mut transaction, sub_room, operation).await?;
            if let Some(agent) = sub_room.creator_agent_member
                && !seat_creator_agent(&mut transaction, sub_room, agent, operation).await?
            {
                // 它这当口被移出了父房间：什么都不写，事务丢掉就是回滚。
                return Ok(Some(SubRoomCreationBlock::ParentUnavailable));
            }
            // 这间子房间是照建之前读到的父房间算的；那以后父房间又存过的话，照现在的再算一遍。
            follow_parent_at_creation(&mut transaction, sub_room, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(None)
        })
    }

    fn list_sub_rooms(
        &self,
        parent: RoomCatalogId,
        viewer: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomEntry>>> {
        Box::pin(async move {
            let operation = "sub_room.list";
            let rows = sqlx::query(LIST_SQL)
                .bind(parent.as_uuid())
                .bind(viewer.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_entry(row, operation))
                .collect()
        })
    }

    fn admit_to_sub_room<'a>(
        &'a self,
        admission: &'a SubRoomAdmission<'a>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "sub_room.admit";
            ensure_next_version(admission.room, admission.expected_version, operation)?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = admit(&mut transaction, admission, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn change_sub_room_manager<'a>(
        &'a self,
        change: &'a SubRoomManagerChange<'a>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "sub_room.manager.change";
            ensure_next_version(change.room, change.expected_version, operation)?;
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = change_manager(&mut transaction, change, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn revise_sub_room<'a>(
        &'a self,
        revision: &'a SubRoomRevision,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomRevisionBlock>>> {
        Box::pin(async move {
            let operation = "sub_room.revise";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = revise(&mut transaction, revision, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn request_sub_room_join<'a>(
        &'a self,
        parent_catalog_id: RoomCatalogId,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<SubRoomJoinRequestOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.join_request.create";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = request_join(&mut transaction, parent_catalog_id, record, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn sub_room_policy(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomPolicyRecord>>> {
        Box::pin(async move {
            let operation = "sub_room.policy";
            let row = sqlx::query(
                r"SELECT policy.creators,
                         policy.agents_may_create,
                         policy.auto_archive_days,
                         policy.updated_by,
                         floor(extract(epoch FROM policy.updated_at) * 1000)::bigint
                             AS updated_at_ms,
                         policy.version
                    FROM agent_room.private_room_sub_room_policy AS policy
                   WHERE policy.catalog_entry_id = $1",
            )
            .bind(parent.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let Some(row) = row else {
                return Ok(None);
            };
            let creators: String = decode_column(&row, "creators", operation)?;
            let updated_by: uuid::Uuid = decode_column(&row, "updated_by", operation)?;
            let updated_at: i64 = decode_column(&row, "updated_at_ms", operation)?;
            let version: i64 = decode_column(&row, "version", operation)?;
            Ok(Some(SubRoomPolicyRecord {
                catalog_id: parent,
                policy: decode_policy(&row, &creators, operation)?,
                updated_by: PrincipalId::from_uuid(updated_by),
                updated_at: UtcMillis::new(updated_at).map_err(|_| corrupt_data(operation))?,
                version: AggregateVersion::new(version).map_err(|_| corrupt_data(operation))?,
            }))
        })
    }

    fn save_sub_room_policy<'a>(
        &'a self,
        record: &'a SubRoomPolicyRecord,
        expected: Option<AggregateVersion>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "sub_room.policy.save";
            let auto_archive_days = record
                .policy
                .auto_archive
                .then_some(i16::from(SUB_ROOM_AUTO_ARCHIVE_DAYS));
            let statement = match expected {
                // 没有记录时新加一行；别人抢先加了就什么都不写。
                None => {
                    r"INSERT INTO agent_room.private_room_sub_room_policy (
                           catalog_entry_id, creators, agents_may_create, auto_archive_days,
                           updated_by, updated_at, version
                       ) VALUES (
                           $1, $2, $3, $4, $5, to_timestamp($6::double precision / 1000.0), $7
                       )
                       ON CONFLICT (catalog_entry_id) DO NOTHING"
                }
                Some(_) => {
                    r"UPDATE agent_room.private_room_sub_room_policy
                          SET creators = $2,
                              agents_may_create = $3,
                              auto_archive_days = $4,
                              updated_by = $5,
                              updated_at = to_timestamp($6::double precision / 1000.0),
                              version = $7
                        WHERE catalog_entry_id = $1
                          AND version = $8"
                }
            };
            let mut query = sqlx::query(statement)
                .bind(record.catalog_id.as_uuid())
                .bind(record.policy.creators.as_str())
                .bind(record.policy.agents_may_create)
                .bind(auto_archive_days)
                .bind(record.updated_by.as_uuid())
                .bind(record.updated_at.value())
                .bind(record.version.value());
            if let Some(expected) = expected {
                query = query.bind(expected.value());
            }
            let written = query
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
                .rows_affected();
            Ok(written == 1)
        })
    }

    fn follow_stale_sub_rooms(
        &self,
        limit: u16,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u32>> {
        Box::pin(follow_stale_sub_rooms(
            self.pool(),
            limit,
            at,
            "sub_room.follow_stale",
        ))
    }

    fn claim_sub_room_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomSyncTask>>> {
        Box::pin(sub_room_sync::claim(
            self.pool(),
            scope,
            limit,
            now,
            lease_until,
        ))
    }

    fn complete_sub_room_sync(
        &self,
        task: SubRoomSyncTask,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(sub_room_sync::complete(self.pool(), task))
    }

    fn retry_sub_room_sync(
        &self,
        task: SubRoomSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(sub_room_sync::retry(
            self.pool(),
            task,
            retry_at,
            failure_code,
        ))
    }
}

const LIST_SQL: &str = r"
    SELECT link.catalog_entry_id,
           instance.matrix_room_id,
           catalog.name,
           catalog.description,
           catalog.status,
           link.access,
           link.hidden,
           link.category_id,
           link.created_by_principal_id,
           floor(extract(epoch FROM link.created_at) * 1000)::bigint AS created_at_ms,
           floor(extract(epoch FROM link.last_message_at) * 1000)::bigint AS last_message_at_ms,
           floor(extract(epoch FROM link.archived_at) * 1000)::bigint AS archived_at_ms,
           link.archived_by_principal_id,
           link.archived_reason,
           COALESCE((
               SELECT array_agg(manager.principal_id
                                ORDER BY manager.granted_at, manager.principal_id)
                 FROM agent_room.private_room_sub_room_manager AS manager
                WHERE manager.catalog_entry_id = link.catalog_entry_id
           ), '{}'::uuid[]) AS managers,
           (SELECT viewer.membership_status
              FROM agent_room.private_room_membership AS viewer
             WHERE viewer.catalog_entry_id = link.catalog_entry_id
               AND viewer.principal_id = $2) AS viewer_status,
           (SELECT count(*)
              FROM agent_room.private_room_membership AS member
             WHERE member.catalog_entry_id = link.catalog_entry_id
               AND member.membership_status = 'joined')::bigint AS member_count,
           (SELECT count(*) FROM (
                SELECT reservation.agent_id
                  FROM agent_room.room_capacity_reservation AS reservation
                 WHERE reservation.catalog_entry_id = link.catalog_entry_id
                   AND reservation.state = 'committed'
                UNION
                SELECT network.agent_id
                  FROM agent_room.network_agent_room AS room
                  JOIN agent_room.network_agent AS network ON network.id = room.network_agent_id
                 WHERE room.catalog_entry_id = link.catalog_entry_id
           ) AS present)::bigint AS agent_count,
           (SELECT floor(extract(epoch FROM request.requested_at) * 1000)::bigint
              FROM agent_room.private_room_join_request AS request
             WHERE request.catalog_entry_id = link.catalog_entry_id
               AND request.requester_principal_id = $2
               AND request.status = 'pending') AS viewer_requested_at_ms,
           (SELECT count(*)
              FROM agent_room.private_room_join_request AS request
             WHERE request.catalog_entry_id = link.catalog_entry_id
               AND request.status = 'pending')::bigint AS pending_request_count
      FROM agent_room.private_room_parent AS link
      JOIN agent_room.room_catalog_entry AS catalog ON catalog.id = link.catalog_entry_id
      JOIN agent_room.private_room_state AS private_state
        ON private_state.catalog_entry_id = link.catalog_entry_id
      JOIN agent_room.room_instance AS instance ON instance.id = private_state.room_instance_id
     WHERE link.parent_catalog_entry_id = $1
     ORDER BY link.created_at, link.catalog_entry_id";

/// 建不了的原因；能建时为空。先锁父房间在目录里的那一行，后面数的才准。
async fn block(
    transaction: &mut Transaction<'_, Postgres>,
    check: &SubRoomCreationCheck,
    operation: &'static str,
) -> RepositoryResult<Option<SubRoomCreationBlock>> {
    let parent = check.parent_catalog_id.as_uuid();
    let usable: Option<bool> = sqlx::query_scalar(
        r"SELECT catalog.kind = 'private_room' AND catalog.status = 'active'
            FROM agent_room.room_catalog_entry AS catalog
           WHERE catalog.id = $1
             FOR NO KEY UPDATE",
    )
    .bind(parent)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if usable != Some(true) {
        return Ok(Some(SubRoomCreationBlock::ParentUnavailable));
    }
    // 只有一层：父房间自己不能是子房间。表上的触发器也卡着，这里先说清原因。
    let nested: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
              SELECT 1 FROM agent_room.private_room_parent WHERE catalog_entry_id = $1
          )",
    )
    .bind(parent)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if nested {
        return Ok(Some(SubRoomCreationBlock::ParentUnavailable));
    }
    // 不算要建的这一间自己：同一个请求重发时它已经在了，后面插入时撞上房间号，按重试处理。
    let (open, same_name): (i64, bool) = sqlx::query_as(
        r"SELECT count(*)::bigint,
                 COALESCE(bool_or(name_key = $2), false)
            FROM agent_room.private_room_parent
           WHERE parent_catalog_entry_id = $1
             AND archived_at IS NULL
             AND catalog_entry_id <> $3",
    )
    .bind(parent)
    .bind(&check.name_key)
    .bind(check.catalog_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if same_name {
        return Ok(Some(SubRoomCreationBlock::NameTaken));
    }
    if open >= i64::from(MAXIMUM_OPEN_SUB_ROOMS) {
        return Ok(Some(SubRoomCreationBlock::RoomLimitReached));
    }
    // 同一个人或 Agent 一小时最多建几个：人自己建的和他每个 Agent 建的分开数。
    let recent: i64 = sqlx::query_scalar(
        r"SELECT count(*)::bigint
            FROM agent_room.private_room_parent
           WHERE created_by_principal_id = $1
             AND created_by_agent_id IS NOT DISTINCT FROM $2
             AND created_at > to_timestamp($3::double precision / 1000.0)
             AND catalog_entry_id <> $4",
    )
    .bind(check.created_by.as_uuid())
    .bind(check.created_by_agent.map(AgentId::as_uuid))
    .bind(check.at.value().saturating_sub(HOUR_MS))
    .bind(check.catalog_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if recent >= i64::from(MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR) {
        return Ok(Some(SubRoomCreationBlock::CreatorLimitReached));
    }
    if let Some(category) = check.category_id
        && !category_in_parent(transaction, check.parent_catalog_id, category, operation).await?
    {
        return Ok(Some(SubRoomCreationBlock::CategoryMissing));
    }
    Ok(None)
}

/// 这个分类在不在这间房里。父房间那一行已经锁着，删分类的事务排在后面，这里看到的存的时候还在。
async fn category_in_parent(
    transaction: &mut Transaction<'_, Postgres>,
    parent: RoomCatalogId,
    category: SubRoomCategoryId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    sqlx::query_scalar(
        r"SELECT EXISTS (
              SELECT 1
                FROM agent_room.private_room_sub_room_category
               WHERE id = $1 AND parent_catalog_entry_id = $2
          )",
    )
    .bind(category.as_uuid())
    .bind(parent.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))
}

/// 把一个人记进子房间。他在父房间里的那条成员记录锁到这次存完：同时把他移出父房间的请求排在
/// 后面，它跟着清子房间时看得到这一条；已经被移出的，这里看得到，什么都不写。
async fn admit(
    transaction: &mut Transaction<'_, Postgres>,
    admission: &SubRoomAdmission<'_>,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let in_parent = lock_parent_membership(
        transaction,
        admission.room.catalog_id(),
        admission.parent_catalog_id,
        admission.principal,
        operation,
    )
    .await?;
    if !in_parent {
        return Ok(false);
    }
    save_private_room(
        transaction,
        admission.room,
        admission.expected_version,
        admission.at,
        operation,
    )
    .await?;
    insert_role_changes(
        transaction,
        admission.room.catalog_id(),
        admission.role_changes,
        admission.at,
        operation,
    )
    .await?;
    Ok(true)
}

/// 设、撤一位子房间管理员。和把人记进来一样先锁他在父房间里的那条成员记录：同时把他移出父房间的
/// 请求排在后面，它跟着清子房间时连管理员记录一起删；已经被移出的，这里什么都不写。身上的“管理”和
/// 名单在同一个事务里改：父房间里一变、子房间照着重算权限时，“管理”看的是名单。
async fn change_manager(
    transaction: &mut Transaction<'_, Postgres>,
    change: &SubRoomManagerChange<'_>,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let catalog_id = change.room.catalog_id();
    let in_parent = lock_parent_membership(
        transaction,
        catalog_id,
        change.parent_catalog_id,
        change.principal,
        operation,
    )
    .await?;
    if !in_parent {
        return Ok(false);
    }
    save_private_room(
        transaction,
        change.room,
        change.expected_version,
        change.at,
        operation,
    )
    .await?;
    let written = if change.grant {
        sqlx::query(
            r"INSERT INTO agent_room.private_room_sub_room_manager (
                   catalog_entry_id, principal_id, granted_by_principal_id, granted_at
               ) VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0))
               ON CONFLICT (catalog_entry_id, principal_id) DO NOTHING",
        )
        .bind(catalog_id.as_uuid())
        .bind(change.principal.as_uuid())
        .bind(change.changed_by.as_uuid())
        .bind(change.at.value())
        .execute(&mut **transaction)
        .await
    } else {
        sqlx::query(
            r"DELETE FROM agent_room.private_room_sub_room_manager
               WHERE catalog_entry_id = $1 AND principal_id = $2",
        )
        .bind(catalog_id.as_uuid())
        .bind(change.principal.as_uuid())
        .execute(&mut **transaction)
        .await
    };
    written.map_err(|error| map_sqlx_error(operation, &error))?;
    insert_role_changes(
        transaction,
        catalog_id,
        change.role_changes,
        change.at,
        operation,
    )
    .await?;
    Ok(true)
}

/// 存子房间之前先看一眼要存的是读到的下一版。
fn ensure_next_version(
    room: &PrivateRoom,
    expected: AggregateVersion,
    operation: &'static str,
) -> RepositoryResult<()> {
    let next = expected
        .next()
        .map_err(|error| map_domain_error(operation, &error))?;
    if next == room.version() {
        Ok(())
    } else {
        Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Constraint,
        ))
    }
}

/// 锁住他在父房间里的那条成员记录到这次存完，回他此刻是不是父房间里已加入的人。
async fn lock_parent_membership(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    parent_catalog_id: RoomCatalogId,
    principal: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let in_parent: Option<bool> = sqlx::query_scalar(
        r"SELECT true
            FROM agent_room.private_room_membership AS member
            JOIN agent_room.private_room_parent AS link
              ON link.parent_catalog_entry_id = member.catalog_entry_id
           WHERE link.catalog_entry_id = $1
             AND member.catalog_entry_id = $2
             AND member.principal_id = $3
             AND member.membership_status = 'joined'
             FOR SHARE OF member",
    )
    .bind(sub_room.as_uuid())
    .bind(parent_catalog_id.as_uuid())
    .bind(principal.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(in_parent == Some(true))
}

/// 记下一个申请进子房间的人。先锁他在父房间里的那条成员记录（和把人记进子房间同一个先后），再锁
/// 这间子房间的父子关系那一行：把他移出父房间、给这一间换门的事务和这里排着队，它们作废在等的申请
/// 时看得到这一条，这里也看得到它们刚改完的样子。
async fn request_join(
    transaction: &mut Transaction<'_, Postgres>,
    parent_catalog_id: RoomCatalogId,
    record: &JoinRequestRecord,
    operation: &'static str,
) -> RepositoryResult<SubRoomJoinRequestOutcome> {
    let catalog_id = record.catalog_id.as_uuid();
    let in_parent: Option<bool> = sqlx::query_scalar(
        r"SELECT true
            FROM agent_room.private_room_membership AS member
           WHERE member.catalog_entry_id = $1
             AND member.principal_id = $2
             AND member.membership_status = 'joined'
             FOR SHARE",
    )
    .bind(parent_catalog_id.as_uuid())
    .bind(record.requester.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if in_parent != Some(true) {
        return Ok(SubRoomJoinRequestOutcome::NotInParent);
    }
    let takes_requests: Option<bool> = sqlx::query_scalar(
        r"SELECT link.access = 'request'
                 AND NOT link.hidden
                 AND link.archived_at IS NULL
                 AND catalog.status = 'active'
            FROM agent_room.private_room_parent AS link
            JOIN agent_room.room_catalog_entry AS catalog ON catalog.id = link.catalog_entry_id
           WHERE link.catalog_entry_id = $1
             AND link.parent_catalog_entry_id = $2
             FOR SHARE OF link",
    )
    .bind(catalog_id)
    .bind(parent_catalog_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if takes_requests != Some(true) {
        return Ok(SubRoomJoinRequestOutcome::Closed);
    }
    let inserted = sqlx::query(
        r"INSERT INTO agent_room.private_room_join_request
              (catalog_entry_id, requester_principal_id, message, status, requested_at)
          VALUES ($1, $2, $3, 'pending', to_timestamp($4::double precision / 1000.0))
          ON CONFLICT (catalog_entry_id, requester_principal_id) WHERE status = 'pending'
          DO NOTHING",
    )
    .bind(catalog_id)
    .bind(record.requester.as_uuid())
    .bind(record.message.as_str())
    .bind(record.requested_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if inserted.rows_affected() == 1 {
        return Ok(SubRoomJoinRequestOutcome::Created);
    }
    // 两个请求同时来：另一个先记上了，回那一个的时间。
    let requested_at: i64 = sqlx::query_scalar(
        r"SELECT floor(extract(epoch FROM requested_at) * 1000)::bigint
            FROM agent_room.private_room_join_request
           WHERE catalog_entry_id = $1
             AND requester_principal_id = $2
             AND status = 'pending'",
    )
    .bind(catalog_id)
    .bind(record.requester.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(SubRoomJoinRequestOutcome::AlreadyPending(
        UtcMillis::new(requested_at).map_err(|_| corrupt_data(operation))?,
    ))
}

/// 改子房间的名字、用途和门。先锁父房间在目录里的那一行，和建子房间排同一个队，重名看得准。
/// 从有门改成没有门的记下拆门的时间，只记第一次：这个时间和聊天服务器上那条“这以后进来的人不补发
/// 钥匙”是同一刻，往后挪等于把门里的话放给第一次拆门以后自己进来的人。
async fn revise(
    transaction: &mut Transaction<'_, Postgres>,
    revision: &SubRoomRevision,
    operation: &'static str,
) -> RepositoryResult<Option<SubRoomRevisionBlock>> {
    let catalog_id = revision.catalog_id.as_uuid();
    let parent = revision.parent_catalog_id.as_uuid();
    sqlx::query_scalar::<_, uuid::Uuid>(
        r"SELECT catalog.id
            FROM agent_room.room_catalog_entry AS catalog
           WHERE catalog.id = $1
             FOR NO KEY UPDATE",
    )
    .bind(parent)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let current: Option<(bool, String, bool)> = sqlx::query_as(
        r"SELECT link.archived_at IS NULL AND catalog.status = 'active',
                 link.access,
                 link.hidden
            FROM agent_room.private_room_parent AS link
            JOIN agent_room.room_catalog_entry AS catalog ON catalog.id = link.catalog_entry_id
           WHERE link.catalog_entry_id = $1
             AND link.parent_catalog_entry_id = $2
             FOR NO KEY UPDATE OF link",
    )
    .bind(catalog_id)
    .bind(parent)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some((true, access, hidden)) = current else {
        return Ok(Some(SubRoomRevisionBlock::Unavailable));
    };
    // 新门是照读到的旧门算的，聊天服务器上也照它先做过了。这一行锁住以后再看一眼：不是那样了就不写。
    let door_moved = revision
        .door
        .is_some_and(|door| access != door.from.access().as_str() || hidden != door.from.hidden());
    if door_moved {
        return Ok(Some(SubRoomRevisionBlock::DoorChanged));
    }
    if name_taken(transaction, revision, operation).await? {
        return Ok(Some(SubRoomRevisionBlock::NameTaken));
    }
    if let Some(Some(category)) = revision.category
        && !category_in_parent(transaction, revision.parent_catalog_id, category, operation).await?
    {
        return Ok(Some(SubRoomRevisionBlock::CategoryMissing));
    }
    // 没提分类时 `$3` 是假，那一列照旧。
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET name_key = $2,
                 category_id = CASE WHEN $3 THEN $4 ELSE category_id END
           WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id)
    .bind(&revision.name_key)
    .bind(revision.category.is_some())
    .bind(revision.category.flatten().map(SubRoomCategoryId::as_uuid))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if let Some(door) = revision.door {
        write_door(
            transaction,
            revision.catalog_id,
            door.to,
            revision.at,
            operation,
        )
        .await?;
        // 换完不收申请了（拆了门、改成只能请进来、藏起来了）：在等的一起作废。拆了门的自己进来就
        // 行；只能请进来的由管理员请；藏起来的外面的人找不到它。
        if !door.to.takes_requests() {
            void_pending_of_room(transaction, revision.catalog_id, revision.at, operation).await?;
        }
    }
    let updated = sqlx::query_scalar::<_, uuid::Uuid>(
        r"UPDATE agent_room.room_catalog_entry
             SET name = $2,
                 description = $3,
                 updated_at = greatest(updated_at, to_timestamp($4::double precision / 1000.0))
           WHERE id = $1 AND kind = 'private_room'
           RETURNING id",
    )
    .bind(catalog_id)
    .bind(&revision.name)
    .bind(&revision.description)
    .bind(revision.at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if updated.is_none() {
        return Err(corrupt_data(operation));
    }
    Ok(None)
}

/// 这间房里另一间没归档的子房间是不是已经叫这个名字。
async fn name_taken(
    transaction: &mut Transaction<'_, Postgres>,
    revision: &SubRoomRevision,
    operation: &'static str,
) -> RepositoryResult<bool> {
    sqlx::query_scalar(
        r"SELECT EXISTS (
              SELECT 1
                FROM agent_room.private_room_parent
               WHERE parent_catalog_entry_id = $1
                 AND archived_at IS NULL
                 AND name_key = $2
                 AND catalog_entry_id <> $3
          )",
    )
    .bind(revision.parent_catalog_id.as_uuid())
    .bind(&revision.name_key)
    .bind(revision.catalog_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))
}

/// 存换好的门。从有门改成没有门的记下拆门的时间，只记第一次。
async fn write_door(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    door: SubRoomDoor,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    // 右边读到的都是改之前的值：`access <> 'open'` 说的是原来有门。
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET door_opened_at = CASE
                     WHEN $2 = 'open' AND access <> 'open'
                         THEN COALESCE(
                             door_opened_at, to_timestamp($4::double precision / 1000.0)
                         )
                     ELSE door_opened_at
                 END,
                 access = $2,
                 hidden = $3
           WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .bind(door.access().as_str())
    .bind(door.hidden())
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn insert_link(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: &NewSubRoom,
    operation: &'static str,
) -> RepositoryResult<()> {
    let catalog_id = sub_room.snapshot.catalog().id().as_uuid();
    sqlx::query(
        r"INSERT INTO agent_room.private_room_parent (
               catalog_entry_id, parent_catalog_entry_id, name_key, access, hidden,
               created_by_principal_id, created_by_agent_id, parent_version_applied, created_at,
               category_id
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8, to_timestamp($9::double precision / 1000.0), $10
           )",
    )
    .bind(catalog_id)
    .bind(sub_room.parent_catalog_id.as_uuid())
    .bind(&sub_room.name_key)
    .bind(sub_room.access.as_str())
    .bind(sub_room.hidden)
    .bind(sub_room.created_by.as_uuid())
    .bind(sub_room.created_by_agent.map(AgentId::as_uuid))
    .bind(sub_room.parent_version.value())
    .bind(sub_room.created_at.value())
    .bind(sub_room.category_id.map(SubRoomCategoryId::as_uuid))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    for manager in &sub_room.managers {
        sqlx::query(
            r"INSERT INTO agent_room.private_room_sub_room_manager (
                   catalog_entry_id, principal_id, granted_by_principal_id, granted_at
               ) VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0))",
        )
        .bind(catalog_id)
        .bind(manager.as_uuid())
        .bind(sub_room.created_by.as_uuid())
        .bind(sub_room.created_at.value())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    Ok(())
}

/// 凭口令或敲门进了父房间的 Agent 成员（网络 Agent）自己建的子房间（2h-1b）：锁着它在父房间里的那一条，
/// 还是已加入的 Agent 成员才把它记成这一间已加入的 Agent 成员。父房间里正在移出它的，等那边提交以后
/// 再看。进来的方式记成 `knock`：它没拿口令进这一间，和没有门时直接放进来一样。
async fn seat_creator_agent(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: &NewSubRoom,
    agent: AgentId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let seated = sqlx::query(
        r"INSERT INTO agent_room.private_room_agent_member
              (catalog_entry_id, agent_id, membership_status, permission_bits,
               joined_via, created_at, status_changed_at)
          SELECT $1, $2, 'joined', $3, 'knock',
                 to_timestamp($4::double precision / 1000.0),
                 to_timestamp($4::double precision / 1000.0)
           WHERE EXISTS (
                     SELECT 1
                       FROM agent_room.private_room_agent_member AS parent_member
                      WHERE parent_member.catalog_entry_id = $5
                        AND parent_member.agent_id = $2
                        AND parent_member.membership_status = 'joined'
                        FOR SHARE OF parent_member
                 )",
    )
    .bind(sub_room.snapshot.catalog().id().as_uuid())
    .bind(agent.as_uuid())
    .bind(i16::from(PrivateRoomPermissions::AGENT_MEMBER.bits()))
    .bind(sub_room.created_at.value())
    .bind(sub_room.parent_catalog_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(seated.rows_affected() == 1)
}

fn decode_link(row: &PgRow, operation: &'static str) -> RepositoryResult<SubRoomLink> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let parent: uuid::Uuid = decode_column(row, "parent_catalog_entry_id", operation)?;
    let access: String = decode_column(row, "access", operation)?;
    let created_by: uuid::Uuid = decode_column(row, "created_by_principal_id", operation)?;
    let created_by_agent: Option<uuid::Uuid> =
        decode_column(row, "created_by_agent_id", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    let door_opened_at: Option<i64> = decode_column(row, "door_opened_at_ms", operation)?;
    Ok(SubRoomLink {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        parent_catalog_id: RoomCatalogId::from_uuid(parent),
        name_key: decode_column(row, "name_key", operation)?,
        access: RoomJoinMode::try_from(access.as_str()).map_err(|_| corrupt_data(operation))?,
        hidden: decode_column(row, "hidden", operation)?,
        door_opened_at: door_opened_at
            .map(|at| UtcMillis::new(at).map_err(|_| corrupt_data(operation)))
            .transpose()?,
        end_to_end_encrypted: decode_column(row, "end_to_end_encrypted", operation)?,
        created_by: PrincipalId::from_uuid(created_by),
        created_by_agent: created_by_agent.map(AgentId::from_uuid),
        created_at: UtcMillis::new(created_at).map_err(|_| corrupt_data(operation))?,
        archived: decode_archive_record(row, operation)?,
    })
}

/// 归档的记录：时间和原因要么都有、要么都没有（表上的约束卡着）。
fn decode_archive_record(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<Option<SubRoomArchiveRecord>> {
    let at: Option<i64> = decode_column(row, "archived_at_ms", operation)?;
    let by: Option<uuid::Uuid> = decode_column(row, "archived_by_principal_id", operation)?;
    let reason: Option<String> = decode_column(row, "archived_reason", operation)?;
    match (at, reason) {
        (None, None) => Ok(None),
        (Some(at), Some(reason)) => Ok(Some(SubRoomArchiveRecord {
            at: UtcMillis::new(at).map_err(|_| corrupt_data(operation))?,
            by: by.map(PrincipalId::from_uuid),
            reason: SubRoomArchiveReason::try_from(reason.as_str())
                .map_err(|_| corrupt_data(operation))?,
        })),
        _ => Err(corrupt_data(operation)),
    }
}

fn decode_entry(row: &PgRow, operation: &'static str) -> RepositoryResult<SubRoomEntry> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    let access: String = decode_column(row, "access", operation)?;
    let created_by: uuid::Uuid = decode_column(row, "created_by_principal_id", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    let last_message_at: Option<i64> = decode_column(row, "last_message_at_ms", operation)?;
    let managers: Vec<uuid::Uuid> = decode_column(row, "managers", operation)?;
    let viewer_status: Option<String> = decode_column(row, "viewer_status", operation)?;
    let member_count: i64 = decode_column(row, "member_count", operation)?;
    let agent_count: i64 = decode_column(row, "agent_count", operation)?;
    let viewer_requested_at: Option<i64> = decode_column(row, "viewer_requested_at_ms", operation)?;
    let pending_requests: i64 = decode_column(row, "pending_request_count", operation)?;
    let category_id: Option<uuid::Uuid> = decode_column(row, "category_id", operation)?;
    let count = |value: i64| u32::try_from(value).map_err(|_| corrupt_data(operation));
    let matrix_room: String = decode_column(row, "matrix_room_id", operation)?;
    Ok(SubRoomEntry {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        matrix_room: MatrixRoomReference::new(matrix_room).map_err(|_| corrupt_data(operation))?,
        name: decode_column(row, "name", operation)?,
        description: decode_column(row, "description", operation)?,
        access: RoomJoinMode::try_from(access.as_str()).map_err(|_| corrupt_data(operation))?,
        hidden: decode_column(row, "hidden", operation)?,
        category_id: category_id.map(SubRoomCategoryId::from_uuid),
        archived: match status.as_str() {
            "active" => false,
            "archived" => true,
            _ => return Err(corrupt_data(operation)),
        },
        archive_record: decode_archive_record(row, operation)?,
        created_by: PrincipalId::from_uuid(created_by),
        created_at: UtcMillis::new(created_at).map_err(|_| corrupt_data(operation))?,
        managers: managers.into_iter().map(PrincipalId::from_uuid).collect(),
        viewer_status: viewer_status
            .map(|status| {
                PrivateRoomMembershipStatus::try_from(status.as_str())
                    .map_err(|_| corrupt_data(operation))
            })
            .transpose()?,
        member_count: count(member_count)?,
        agent_count: count(agent_count)?,
        last_message_at: last_message_at
            .map(|at| UtcMillis::new(at).map_err(|_| corrupt_data(operation)))
            .transpose()?,
        viewer_requested_at: viewer_requested_at
            .map(|at| UtcMillis::new(at).map_err(|_| corrupt_data(operation)))
            .transpose()?,
        pending_requests: count(pending_requests)?,
    })
}

fn decode_policy(
    row: &PgRow,
    creators: &str,
    operation: &'static str,
) -> RepositoryResult<SubRoomPolicy> {
    let agents_may_create: Option<bool> = decode_column(row, "agents_may_create", operation)?;
    let auto_archive_days: Option<i16> = decode_column(row, "auto_archive_days", operation)?;
    Ok(SubRoomPolicy {
        creators: SubRoomCreators::try_from(creators).map_err(|_| corrupt_data(operation))?,
        agents_may_create: agents_may_create.ok_or_else(|| corrupt_data(operation))?,
        auto_archive: auto_archive_days.is_some(),
    })
}
