CREATE TABLE agent_room.account_deletion_job (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL UNIQUE REFERENCES agent_room.principal(id),
    matrix_user_id text NOT NULL,
    receipt_digest bytea NOT NULL UNIQUE,
    stage text NOT NULL DEFAULT 'queued',
    attempt_count integer NOT NULL DEFAULT 0,
    retry_at timestamptz,
    lease_expires_at timestamptz,
    failure_code text,
    requested_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    completed_at timestamptz,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT account_deletion_job_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT account_deletion_matrix_user_id_format CHECK (
        length(matrix_user_id) BETWEEN 4 AND 512 AND matrix_user_id LIKE '@%:%'
    ),
    CONSTRAINT account_deletion_receipt_digest_length CHECK (
        octet_length(receipt_digest) = 32
    ),
    CONSTRAINT account_deletion_stage CHECK (
        stage IN (
            'queued',
            'federated_deactivation',
            'local_erasure',
            'retry_scheduled',
            'completed'
        )
    ),
    CONSTRAINT account_deletion_attempt_count CHECK (
        attempt_count BETWEEN 0 AND 65535
    ),
    CONSTRAINT account_deletion_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    ),
    CONSTRAINT account_deletion_version_nonnegative CHECK (version >= 0),
    CONSTRAINT account_deletion_timestamp_order CHECK (
        updated_at >= requested_at
        AND (retry_at IS NULL OR retry_at >= requested_at)
        AND (lease_expires_at IS NULL OR lease_expires_at > updated_at)
        AND (completed_at IS NULL OR completed_at >= requested_at)
    ),
    CONSTRAINT account_deletion_stage_consistency CHECK (
        (
            stage = 'queued'
            AND attempt_count = 0
            AND retry_at IS NULL
            AND lease_expires_at IS NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            stage = 'federated_deactivation'
            AND attempt_count > 0
            AND retry_at IS NULL
            AND lease_expires_at IS NOT NULL
            AND completed_at IS NULL
        )
        OR (
            stage = 'local_erasure'
            AND attempt_count > 0
            AND retry_at IS NULL
            AND completed_at IS NULL
        )
        OR (
            stage = 'retry_scheduled'
            AND attempt_count > 0
            AND retry_at IS NOT NULL
            AND lease_expires_at IS NULL
            AND failure_code IS NOT NULL
            AND completed_at IS NULL
        )
        OR (
            stage = 'completed'
            AND retry_at IS NULL
            AND lease_expires_at IS NULL
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
    )
);

CREATE INDEX account_deletion_job_due_idx
    ON agent_room.account_deletion_job (retry_at, requested_at)
    WHERE stage IN ('queued', 'retry_scheduled', 'local_erasure');

CREATE INDEX account_deletion_job_expired_lease_idx
    ON agent_room.account_deletion_job (lease_expires_at)
    WHERE stage = 'federated_deactivation';

GRANT SELECT, INSERT, UPDATE
    ON agent_room.account_deletion_job
    TO agent_room_runtime;

REVOKE DELETE
    ON agent_room.account_deletion_job
    FROM agent_room_runtime;
