use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};
use agent_room_application::{
    persistence::RepositoryResult,
    ports::{InboxHandoff, InboxRoom, PersonalInboxIndex, PersonalInboxRepository, PortFuture},
};
use agent_room_domain::ids::PrincipalId;

impl PersonalInboxRepository for PostgresRepositories {
    fn index(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<PersonalInboxIndex>> {
        Box::pin(async move {
            const OPERATION: &str = "inbox.index";
            // Public routing metadata is public. Private routing requires a joined viewing
            // membership; direct sessions belong only to their initiating human account.
            let rows = sqlx::query(r"
                SELECT catalog.id::text AS catalog_id, instance.matrix_room_id AS room_id,
                       catalog.name, catalog.kind = 'direct' AS direct
                FROM agent_room.room_catalog_entry AS catalog
                JOIN agent_room.room_instance AS instance ON instance.catalog_entry_id = catalog.id
                WHERE catalog.status = 'active' AND instance.state = 'active'
                  AND instance.matrix_room_id IS NOT NULL
                  AND ((catalog.kind = 'public_lobby' AND catalog.visibility = 'public')
                    OR EXISTS (SELECT 1 FROM agent_room.private_room_membership AS member
                        WHERE member.catalog_entry_id = catalog.id AND member.principal_id = $1
                          AND member.membership_status = 'joined' AND (member.permission_bits & 1) = 1)
                    OR EXISTS (SELECT 1 FROM agent_room.direct_session AS direct
                        WHERE direct.catalog_entry_id = catalog.id AND direct.principal_id = $1
                          AND direct.lifecycle_state = 'active'
                          AND NOT EXISTS(SELECT 1 FROM agent_room.direct_contact_block AS block WHERE block.principal_id=direct.principal_id AND block.agent_id=direct.target_agent_id AND block.revoked_at IS NULL)))
                ORDER BY catalog.id, instance.id LIMIT 1001")
                .bind(principal_id.as_uuid()).fetch_all(self.pool()).await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let mut limited = rows.len() > 1000;
            let rooms = rows
                .iter()
                .take(1000)
                .map(|row| {
                    Ok(InboxRoom {
                        catalog_id: decode_column(row, "catalog_id", OPERATION)?,
                        room_id: decode_column(row, "room_id", OPERATION)?,
                        name: decode_column(row, "name", OPERATION)?,
                        direct: decode_column(row, "direct", OPERATION)?,
                    })
                })
                .collect::<RepositoryResult<Vec<_>>>()?;
            let rows = sqlx::query(r"
                SELECT handoff.id::text AS handoff_id, handoff.source_matrix_room_id AS room_id,
                       handoff.source_message_id::text AS message_id, agent.display_name AS agent_name,
                       handoff.state AS status,
                       floor(extract(epoch FROM COALESCE(handoff.created_at, handoff.approved_at)) * 1000)::bigint AS created_at
                FROM agent_room.context_handoff AS handoff
                JOIN agent_room.agent_instance AS instance ON instance.id = handoff.target_agent_instance_id
                JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                WHERE handoff.principal_id = $1 AND handoff.state IN ('queued', 'delivered', 'failed')
                  AND handoff.source_message_id IS NOT NULL
                  AND handoff.expires_at > CURRENT_TIMESTAMP
                ORDER BY handoff.approved_at DESC, handoff.id DESC LIMIT 201")
                .bind(principal_id.as_uuid()).fetch_all(self.pool()).await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            limited |= rows.len() > 200;
            let handoffs = rows
                .iter()
                .take(200)
                .map(|row| {
                    Ok(InboxHandoff {
                        handoff_id: decode_column(row, "handoff_id", OPERATION)?,
                        room_id: decode_column(row, "room_id", OPERATION)?,
                        message_id: decode_column(row, "message_id", OPERATION)?,
                        agent_name: decode_column(row, "agent_name", OPERATION)?,
                        status: decode_column(row, "status", OPERATION)?,
                        created_at_unix_ms: decode_column(row, "created_at", OPERATION)?,
                    })
                })
                .collect::<RepositoryResult<Vec<_>>>()?;
            Ok(PersonalInboxIndex {
                rooms,
                handoffs,
                limited,
            })
        })
    }
}
