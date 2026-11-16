//! 每小时那一轮要看谁、他在哪几间房（`specs/redesign/reminders.md` 的“怎么算”）。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        MatrixRoomId, MatrixUserId, PortFuture, ReminderCandidate, ReminderRecipientRepository,
        ReminderRoom, ReminderRoomKind,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    network_agents::NETWORK_AGENT_ISSUER,
    time::UtcMillis,
};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use crate::{PostgresRepositories, agents::corrupt_data, error::map_sqlx_error};

/// 一个人最多看这么多间房；再多的这一轮不看（按目录 ID 排，每轮看的是同一批）。
const MAX_ROOMS: i64 = 500;

impl ReminderRecipientRepository for PostgresRepositories {
    fn reminder_candidates(
        &self,
        after: Option<PrincipalId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderCandidate>>> {
        Box::pin(async move {
            const OPERATION: &str = "reminders.candidates";
            // 还在用、不是网络 Agent、没关邮件提醒（没有那一行算开着）。被停用的不提醒。
            let rows = sqlx::query(
                r"SELECT principal.id, principal.matrix_user_id,
                         floor(extract(epoch FROM reminder.last_email_at) * 1000)::bigint
                             AS last_email_at,
                         floor(extract(epoch FROM reminder.covered_until) * 1000)::bigint
                             AS covered_until
                    FROM agent_room.principal AS principal
                    LEFT JOIN agent_room.account_reminder AS reminder
                      ON reminder.principal_id = principal.id
                   WHERE principal.status = 'active'
                     AND principal.oidc_issuer <> $3
                     AND COALESCE(reminder.email_enabled, true)
                     AND ($1::uuid IS NULL OR principal.id > $1)
                   ORDER BY principal.id
                   LIMIT $2",
            )
            .bind(after.map(PrincipalId::as_uuid))
            .bind(i64::from(limit))
            .bind(NETWORK_AGENT_ISSUER)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            rows.iter().map(|row| candidate(row, OPERATION)).collect()
        })
    }

    fn reminder_rooms(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderRoom>>> {
        Box::pin(async move {
            const OPERATION: &str = "reminders.rooms";
            // 和收件箱认的一样：已加入、能看的私人房间和子房间，他自己开的、没屏蔽的私聊。不算公共
            // 大厅，不算归档了的（子房间归档记在父子关系那一行上）。
            let rows = sqlx::query(
                r"SELECT catalog.id AS catalog_id, instance.matrix_room_id,
                         catalog.kind = 'direct' AS direct,
                         parent.catalog_entry_id IS NOT NULL AS sub_room,
                         catalog.end_to_end_encrypted,
                         floor(extract(epoch FROM
                             COALESCE(member.status_changed_at, direct.created_at)) * 1000
                         )::bigint AS joined_at
                    FROM agent_room.room_catalog_entry AS catalog
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = catalog.id
                    LEFT JOIN agent_room.private_room_membership AS member
                      ON member.catalog_entry_id = catalog.id
                     AND member.principal_id = $1
                     AND member.membership_status = 'joined'
                     AND (member.permission_bits & 1) = 1
                    LEFT JOIN agent_room.direct_session AS direct
                      ON direct.catalog_entry_id = catalog.id
                     AND direct.principal_id = $1
                     AND direct.lifecycle_state = 'active'
                     AND NOT EXISTS (
                         SELECT 1 FROM agent_room.direct_contact_block AS block
                          WHERE block.principal_id = direct.principal_id
                            AND block.agent_id = direct.target_agent_id
                            AND block.revoked_at IS NULL)
                    LEFT JOIN agent_room.private_room_parent AS parent
                      ON parent.catalog_entry_id = catalog.id
                   WHERE catalog.kind IN ('private_room', 'direct')
                     AND catalog.status = 'active'
                     AND instance.state = 'active'
                     AND instance.matrix_room_id IS NOT NULL
                     AND (member.principal_id IS NOT NULL OR direct.catalog_entry_id IS NOT NULL)
                     AND parent.archived_at IS NULL
                   ORDER BY catalog.id, instance.id
                   LIMIT $2",
            )
            .bind(principal_id.as_uuid())
            .bind(MAX_ROOMS)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            rows.iter().map(|row| room(row, OPERATION)).collect()
        })
    }
}

fn candidate(row: &PgRow, operation: &'static str) -> RepositoryResult<ReminderCandidate> {
    let corrupt = |_| corrupt_data(operation);
    let principal_id: Uuid = row.try_get("id").map_err(corrupt)?;
    let matrix_user_id: String = row.try_get("matrix_user_id").map_err(corrupt)?;
    Ok(ReminderCandidate {
        principal_id: PrincipalId::from_uuid(principal_id),
        matrix_user_id: MatrixUserId::new(matrix_user_id).map_err(|_| corrupt_data(operation))?,
        last_email_at: optional_millis(row, "last_email_at", operation)?,
        covered_until: optional_millis(row, "covered_until", operation)?,
    })
}

fn room(row: &PgRow, operation: &'static str) -> RepositoryResult<ReminderRoom> {
    let corrupt = |_| corrupt_data(operation);
    let catalog_id: Uuid = row.try_get("catalog_id").map_err(corrupt)?;
    let matrix_room_id: String = row.try_get("matrix_room_id").map_err(corrupt)?;
    let direct: bool = row.try_get("direct").map_err(corrupt)?;
    let sub_room: bool = row.try_get("sub_room").map_err(corrupt)?;
    let joined_at: i64 = row.try_get("joined_at").map_err(corrupt)?;
    Ok(ReminderRoom {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        matrix_room_id: MatrixRoomId::new(matrix_room_id).map_err(|_| corrupt_data(operation))?,
        kind: if direct {
            ReminderRoomKind::Direct
        } else if sub_room {
            ReminderRoomKind::SubRoom
        } else {
            ReminderRoomKind::Room
        },
        end_to_end_encrypted: row.try_get("end_to_end_encrypted").map_err(corrupt)?,
        joined_at: UtcMillis::new(joined_at).map_err(|_| corrupt_data(operation))?,
    })
}

fn optional_millis(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<UtcMillis>> {
    let value: Option<i64> = row.try_get(column).map_err(|_| corrupt_data(operation))?;
    value
        .map(|value| UtcMillis::new(value).map_err(|_| corrupt_data(operation)))
        .transpose()
}
