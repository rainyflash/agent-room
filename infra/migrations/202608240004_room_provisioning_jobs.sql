CREATE TABLE agent_room.room_provisioning_job (
    id uuid PRIMARY KEY,
    catalog_entry_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    target_kind text NOT NULL,
    room_instance_id uuid,
    region_hint text,
    room_alias_localpart text NOT NULL,
    matrix_room_id text,
    state text NOT NULL DEFAULT 'pending',
    lease_id uuid,
    lease_expires_at timestamptz,
    failure_code text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    completed_at timestamptz,
    CONSTRAINT room_provisioning_job_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT room_provisioning_job_target_kind CHECK (
        target_kind IN ('space', 'instance')
    ),
    CONSTRAINT room_provisioning_job_target_shape CHECK (
        (target_kind = 'space' AND room_instance_id IS NULL AND region_hint IS NULL)
        OR (target_kind = 'instance' AND room_instance_id IS NOT NULL)
    ),
    CONSTRAINT room_provisioning_job_instance_id_v7 CHECK (
        room_instance_id IS NULL OR substring(room_instance_id::text, 15, 1) = '7'
    ),
    CONSTRAINT room_provisioning_job_region_length CHECK (
        region_hint IS NULL OR length(region_hint) BETWEEN 1 AND 64
    ),
    CONSTRAINT room_provisioning_job_alias_format CHECK (
        room_alias_localpart ~ '^[a-z0-9][a-z0-9._=-]{0,254}$'
    ),
    CONSTRAINT room_provisioning_job_matrix_room_id_format CHECK (
        matrix_room_id IS NULL
        OR (
            length(matrix_room_id) BETWEEN 4 AND 512
            AND matrix_room_id LIKE '!%:%'
        )
    ),
    CONSTRAINT room_provisioning_job_state CHECK (
        state IN ('pending', 'completed')
    ),
    CONSTRAINT room_provisioning_job_lease_pair CHECK (
        (lease_id IS NULL AND lease_expires_at IS NULL)
        OR (lease_id IS NOT NULL AND lease_expires_at IS NOT NULL)
    ),
    CONSTRAINT room_provisioning_job_lease_id_v7 CHECK (
        lease_id IS NULL OR substring(lease_id::text, 15, 1) = '7'
    ),
    CONSTRAINT room_provisioning_job_failure_code CHECK (
        failure_code IS NULL
        OR failure_code IN ('matrix_create', 'matrix_resolve', 'space_attach')
    ),
    CONSTRAINT room_provisioning_job_completion_consistency CHECK (
        (
            state = 'pending'
            AND completed_at IS NULL
        )
        OR (
            state = 'completed'
            AND matrix_room_id IS NOT NULL
            AND completed_at IS NOT NULL
            AND lease_id IS NULL
            AND lease_expires_at IS NULL
            AND failure_code IS NULL
        )
    ),
    CONSTRAINT room_provisioning_job_time_order CHECK (
        updated_at >= created_at
        AND (lease_expires_at IS NULL OR lease_expires_at > updated_at)
        AND (completed_at IS NULL OR completed_at >= created_at)
    ),
    CONSTRAINT room_provisioning_job_alias_unique UNIQUE (room_alias_localpart),
    CONSTRAINT room_provisioning_job_instance_unique UNIQUE (room_instance_id)
);

-- 同一目录只允许一个待完成 Space；同一地区只允许一个待完成房间任务。
CREATE UNIQUE INDEX room_provisioning_space_pending_unique
    ON agent_room.room_provisioning_job (catalog_entry_id)
    WHERE target_kind = 'space' AND state = 'pending';

CREATE UNIQUE INDEX room_provisioning_instance_pending_unique
    ON agent_room.room_provisioning_job (
        catalog_entry_id,
        COALESCE(region_hint, '')
    )
    WHERE target_kind = 'instance' AND state = 'pending';

CREATE INDEX room_provisioning_lease_expiry_idx
    ON agent_room.room_provisioning_job (lease_expires_at, id)
    WHERE state = 'pending' AND lease_id IS NOT NULL;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.room_provisioning_job
    TO agent_room_runtime;
