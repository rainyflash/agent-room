UPDATE agent_room.agent_instance
SET lease_expires_at = NULL
WHERE status = 'revoked' AND lease_expires_at IS NOT NULL;

ALTER TABLE agent_room.agent_instance
    ADD COLUMN matrix_device_revoked_at timestamptz,
    ADD CONSTRAINT agent_instance_matrix_revocation_requires_local_revocation CHECK (
        matrix_device_revoked_at IS NULL OR revoked_at IS NOT NULL
    ),
    ADD CONSTRAINT agent_instance_matrix_revocation_order CHECK (
        matrix_device_revoked_at IS NULL OR matrix_device_revoked_at >= revoked_at
    );

CREATE INDEX agent_instance_pending_matrix_revocation_idx
    ON agent_room.agent_instance (revoked_at, id)
    WHERE revoked_at IS NOT NULL AND matrix_device_revoked_at IS NULL;
