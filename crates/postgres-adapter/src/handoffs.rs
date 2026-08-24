use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        HandoffAccessRepository, HandoffAuthorizationSnapshot, HandoffInstanceAccessRecord,
        PortFuture,
    },
};
use agent_room_domain::{
    agents::AgentRole,
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId},
};
use sqlx::postgres::PgRow;

use crate::{
    PostgresRepositories,
    agents::{corrupt_data, decode_column},
    error::map_sqlx_error,
};

impl HandoffAccessRepository for PostgresRepositories {
    fn inspect_authorization(
        &self,
        principal_id: PrincipalId,
        requester_instance_id: AgentInstanceId,
        target_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffAuthorizationSnapshot>>> {
        Box::pin(async move {
            const OPERATION: &str = "handoff.authorization.inspect";
            let row = sqlx::query(
                r"WITH instance_access AS (
                    SELECT instance.id AS instance_id,
                           instance.agent_id,
                           instance.device_id,
                           agent.matrix_user_id,
                           instance.matrix_device_id,
                           ownership.role,
                           instance.revoked_at IS NULL
                               AND instance.status <> 'revoked'
                               AND device.revoked_at IS NULL
                               AND device.trust_state = 'verified'
                               AND principal.status = 'active'
                               AND agent.lifecycle_state = 'active'
                               AND binding.state = 'active' AS active
                      FROM agent_room.agent_instance AS instance
                      JOIN agent_room.device AS device ON device.id = instance.device_id
                      JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                      JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                      JOIN agent_room.adapter_binding AS binding
                        ON binding.id = instance.adapter_binding_id
                      LEFT JOIN agent_room.agent_ownership AS ownership
                        ON ownership.agent_id = instance.agent_id
                       AND ownership.principal_id = $1
                       AND ownership.revoked_at IS NULL
                     WHERE instance.id = $2 OR instance.id = $3
                )
                SELECT requester.instance_id AS requester_instance_id,
                       requester.agent_id AS requester_agent_id,
                       requester.device_id AS requester_device_id,
                       requester.matrix_user_id AS requester_matrix_user_id,
                       requester.matrix_device_id AS requester_matrix_device_id,
                       requester.role AS requester_role,
                       requester.active AS requester_active,
                       target.instance_id AS target_instance_id,
                       target.agent_id AS target_agent_id,
                       target.device_id AS target_device_id,
                       target.matrix_user_id AS target_matrix_user_id,
                       target.matrix_device_id AS target_matrix_device_id,
                       target.role AS target_role,
                       target.active AS target_active
                  FROM instance_access AS requester
                  CROSS JOIN instance_access AS target
                 WHERE requester.instance_id = $2 AND target.instance_id = $3",
            )
            .bind(principal_id.as_uuid())
            .bind(requester_instance_id.as_uuid())
            .bind(target_instance_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            row.map(|row| {
                Ok(HandoffAuthorizationSnapshot {
                    requester: decode_access_record(&row, "requester", OPERATION)?,
                    target: decode_access_record(&row, "target", OPERATION)?,
                })
            })
            .transpose()
        })
    }

    fn find_instance_access(
        &self,
        principal_id: PrincipalId,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffInstanceAccessRecord>>> {
        Box::pin(async move {
            const OPERATION: &str = "handoff.instance_access.find";
            let row = sqlx::query(
                r"SELECT instance.id AS instance_id,
                          instance.agent_id,
                          instance.device_id,
                          agent.matrix_user_id,
                          instance.matrix_device_id,
                          ownership.role,
                          instance.revoked_at IS NULL
                              AND instance.status <> 'revoked'
                              AND device.revoked_at IS NULL
                              AND device.trust_state = 'verified'
                              AND principal.status = 'active'
                              AND agent.lifecycle_state = 'active'
                              AND binding.state = 'active' AS active
                     FROM agent_room.agent_instance AS instance
                     JOIN agent_room.device AS device ON device.id = instance.device_id
                     JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                     JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                     JOIN agent_room.adapter_binding AS binding
                       ON binding.id = instance.adapter_binding_id
                     LEFT JOIN agent_room.agent_ownership AS ownership
                       ON ownership.agent_id = instance.agent_id
                      AND ownership.principal_id = $1
                      AND ownership.revoked_at IS NULL
                    WHERE instance.id = $2",
            )
            .bind(principal_id.as_uuid())
            .bind(instance_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            row.map(|row| decode_access_record(&row, "", OPERATION))
                .transpose()
        })
    }
}

fn decode_access_record(
    row: &PgRow,
    prefix: &str,
    operation: &'static str,
) -> RepositoryResult<HandoffInstanceAccessRecord> {
    let column = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}_{name}")
        }
    };
    let instance_id: uuid::Uuid = decode_column(row, &column("instance_id"), operation)?;
    let agent_id: uuid::Uuid = decode_column(row, &column("agent_id"), operation)?;
    let device_id: uuid::Uuid = decode_column(row, &column("device_id"), operation)?;
    let role: Option<String> = decode_column(row, &column("role"), operation)?;
    let role = role
        .map(|value| AgentRole::try_from(value.as_str()).map_err(|_| corrupt_data(operation)))
        .transpose()?;
    Ok(HandoffInstanceAccessRecord {
        instance_id: AgentInstanceId::from_uuid(instance_id),
        agent_id: AgentId::from_uuid(agent_id),
        device_id: DeviceId::from_uuid(device_id),
        matrix_user_id: decode_column(row, &column("matrix_user_id"), operation)?,
        matrix_device_id: decode_column(row, &column("matrix_device_id"), operation)?,
        role,
        active: decode_column(row, &column("active"), operation)?,
    })
}
