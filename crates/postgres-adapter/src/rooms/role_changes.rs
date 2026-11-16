//! 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）。写在保存房间的同一个事务里，只加不改。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        PortFuture, RoomRoleChange, RoomRoleChangeLog, RoomRoleChangeRecord, RoomRoleChangeSubRoom,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    owner_handover::AdminTenure,
    private_rooms::{PrivateRoomMembershipStatus, PrivateRoomPermissions},
    room_role_changes::RoomStanding,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::decode::corrupt_data;

/// 记下这一次保存动了谁的身份。和房间的成员在同一个事务里：房间没存成，记录也没有。
pub(super) async fn insert_role_changes(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    changes: &[RoomRoleChange],
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    for entry in changes {
        let previous = entry.change.previous;
        let next = entry.change.next;
        sqlx::query(
            r"INSERT INTO agent_room.private_room_role_change (
                   catalog_entry_id, actor_principal_id, target_principal_id,
                   previous_status, previous_owner, previous_permission_bits,
                   next_status, next_owner, next_permission_bits, changed_at
               ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9,
                   to_timestamp($10::double precision / 1000.0)
               )",
        )
        .bind(catalog_id.as_uuid())
        .bind(entry.actor.map(PrincipalId::as_uuid))
        .bind(entry.change.target.as_uuid())
        .bind(previous.map(|standing| standing.status.as_str()))
        .bind(previous.map(|standing| standing.owner))
        .bind(previous.map(|standing| i16::from(standing.permissions.bits())))
        .bind(next.status.as_str())
        .bind(next.owner)
        .bind(i16::from(next.permissions.bits()))
        .bind(changed_at.value())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    Ok(())
}

impl RoomRoleChangeLog for PostgresRepositories {
    fn recent_role_changes(
        &self,
        catalog_id: RoomCatalogId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomRoleChangeRecord>>> {
        Box::pin(async move {
            let operation = "private_room.role_changes.recent";
            // 这间房自己的，加上它下面每间子房间里有人动手的（R5）。一间一间走索引
            // （`private_room_role_change_by_room`），子房间最多一百间。
            let rows = sqlx::query(
                r"WITH room AS (
                      SELECT $1::uuid AS catalog_entry_id, false AS sub_room
                      UNION ALL
                      SELECT link.catalog_entry_id, true
                        FROM agent_room.private_room_parent AS link
                       WHERE link.parent_catalog_entry_id = $1
                  )
                  SELECT change.id,
                         change.actor_principal_id,
                         change.target_principal_id,
                         change.previous_status,
                         change.previous_owner,
                         change.previous_permission_bits,
                         change.next_status,
                         change.next_owner,
                         change.next_permission_bits,
                         floor(extract(epoch FROM change.changed_at) * 1000)::bigint
                             AS changed_at_ms,
                         catalog.id AS sub_room_id,
                         catalog.name AS sub_room_name
                    FROM room
                    JOIN agent_room.private_room_role_change AS change
                      ON change.catalog_entry_id = room.catalog_entry_id
                    LEFT JOIN agent_room.room_catalog_entry AS catalog
                      ON room.sub_room AND catalog.id = room.catalog_entry_id
                   WHERE NOT room.sub_room OR change.actor_principal_id IS NOT NULL
                   ORDER BY change.id DESC
                   LIMIT $2",
            )
            .bind(catalog_id.as_uuid())
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_change(row, operation))
                .collect()
        })
    }

    fn admin_tenure(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AdminTenure>>> {
        Box::pin(async move {
            let operation = "private_room.role_changes.admin_tenure";
            // “拿到管理”：之后在房间里、有“管理”，之前不在房间里或者没有“管理”。房主转让以后自己留着
            // “管理”的不算拿到（他一直有），没有记录就算当得最久。
            let rows = sqlx::query(
                r"SELECT membership.principal_id,
                         floor(extract(epoch FROM membership.created_at) * 1000)::bigint
                             AS member_since_ms,
                         (SELECT floor(extract(epoch FROM max(change.changed_at)) * 1000)::bigint
                            FROM agent_room.private_room_role_change AS change
                           WHERE change.catalog_entry_id = membership.catalog_entry_id
                             AND change.target_principal_id = membership.principal_id
                             AND change.next_status IN ('invited', 'joined')
                             AND (change.next_permission_bits & 8) <> 0
                             AND NOT COALESCE(
                                     change.previous_status IN ('invited', 'joined')
                                     AND (change.previous_permission_bits & 8) <> 0,
                                     false
                                 )
                         ) AS admin_since_ms
                    FROM agent_room.private_room_membership AS membership
                   WHERE membership.catalog_entry_id = $1
                     AND membership.membership_status = 'joined'
                     AND (membership.permission_bits & 8) <> 0
                   ORDER BY membership.principal_id",
            )
            .bind(catalog_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_tenure(row, operation))
                .collect()
        })
    }
}

fn decode_tenure(row: &PgRow, operation: &'static str) -> RepositoryResult<AdminTenure> {
    let principal: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let member_since: i64 = decode_column(row, "member_since_ms", operation)?;
    let admin_since: Option<i64> = decode_column(row, "admin_since_ms", operation)?;
    let time =
        |value: i64| UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error));
    Ok(AdminTenure {
        principal_id: PrincipalId::from_uuid(principal),
        admin_since: admin_since.map(time).transpose()?,
        member_since: time(member_since)?,
    })
}

fn decode_change(row: &PgRow, operation: &'static str) -> RepositoryResult<RoomRoleChangeRecord> {
    let id: i64 = decode_column(row, "id", operation)?;
    let actor: Option<uuid::Uuid> = decode_column(row, "actor_principal_id", operation)?;
    let target: uuid::Uuid = decode_column(row, "target_principal_id", operation)?;
    let previous_status: Option<String> = decode_column(row, "previous_status", operation)?;
    let previous_owner: Option<bool> = decode_column(row, "previous_owner", operation)?;
    let previous_bits: Option<i16> = decode_column(row, "previous_permission_bits", operation)?;
    let next_status: String = decode_column(row, "next_status", operation)?;
    let next_owner: bool = decode_column(row, "next_owner", operation)?;
    let next_bits: i16 = decode_column(row, "next_permission_bits", operation)?;
    let changed_at: i64 = decode_column(row, "changed_at_ms", operation)?;
    let sub_room_id: Option<uuid::Uuid> = decode_column(row, "sub_room_id", operation)?;
    let sub_room_name: Option<String> = decode_column(row, "sub_room_name", operation)?;
    let sub_room = match (sub_room_id, sub_room_name) {
        (Some(id), Some(name)) => Some(RoomRoleChangeSubRoom {
            catalog_id: RoomCatalogId::from_uuid(id),
            name,
        }),
        (None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    let previous = match (previous_status, previous_owner, previous_bits) {
        (Some(status), Some(owner), Some(bits)) => Some(standing(&status, owner, bits, operation)?),
        (None, None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    Ok(RoomRoleChangeRecord {
        id,
        actor: actor.map(PrincipalId::from_uuid),
        target: PrincipalId::from_uuid(target),
        previous,
        next: standing(&next_status, next_owner, next_bits, operation)?,
        changed_at: UtcMillis::new(changed_at)
            .map_err(|error| map_domain_error(operation, &error))?,
        sub_room,
    })
}

fn standing(
    status: &str,
    owner: bool,
    permission_bits: i16,
    operation: &'static str,
) -> RepositoryResult<RoomStanding> {
    let bits = u8::try_from(permission_bits).map_err(|_| corrupt_data(operation))?;
    Ok(RoomStanding {
        status: PrivateRoomMembershipStatus::try_from(status)
            .map_err(|_| corrupt_data(operation))?,
        owner,
        permissions: PrivateRoomPermissions::from_bits(bits)
            .map_err(|_| corrupt_data(operation))?,
    })
}
