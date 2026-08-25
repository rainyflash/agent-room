ALTER TABLE agent_room.moderation_case
    ADD COLUMN room_catalog_id uuid REFERENCES agent_room.room_catalog_entry(id),
    ADD COLUMN matrix_event_id text,
    ADD COLUMN reporter_submitted_excerpt text,
    ADD COLUMN evidence_end_to_end_encrypted boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT moderation_case_event_reference_length CHECK (
        matrix_event_id IS NULL OR length(matrix_event_id) BETWEEN 1 AND 1024
    ),
    ADD CONSTRAINT moderation_case_excerpt_length CHECK (
        reporter_submitted_excerpt IS NULL
        OR length(reporter_submitted_excerpt) BETWEEN 1 AND 4096
    ),
    ADD CONSTRAINT moderation_case_excerpt_requires_event CHECK (
        reporter_submitted_excerpt IS NULL OR matrix_event_id IS NOT NULL
    );

CREATE INDEX moderation_case_reporter_time_idx
    ON agent_room.moderation_case (reporter_principal_id, created_at DESC);

CREATE TABLE agent_room.moderation_report_rate (
    principal_id uuid PRIMARY KEY REFERENCES agent_room.principal(id),
    window_started_at timestamptz NOT NULL,
    report_count integer NOT NULL,
    CONSTRAINT moderation_report_rate_count CHECK (report_count BETWEEN 0 AND 1000)
);

ALTER TABLE agent_room.moderation_action
    ADD COLUMN room_catalog_id uuid REFERENCES agent_room.room_catalog_entry(id),
    ADD COLUMN target_kind text,
    ADD COLUMN status text NOT NULL DEFAULT 'pending',
    ADD COLUMN failure_code text,
    ADD CONSTRAINT moderation_action_target_kind CHECK (
        target_kind IS NULL OR target_kind IN ('principal', 'event')
    ),
    ADD CONSTRAINT moderation_action_status CHECK (
        status IN ('pending', 'applied', 'failed', 'reversed')
    ),
    ADD CONSTRAINT moderation_action_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    ),
    ADD CONSTRAINT moderation_action_result_consistency CHECK (
        (status IN ('pending', 'applied') AND failure_code IS NULL AND reversed_at IS NULL)
        OR (status = 'failed' AND failure_code IS NOT NULL AND reversed_at IS NULL)
        OR (status = 'reversed' AND failure_code IS NULL AND reversed_at IS NOT NULL)
    ),
    ADD CONSTRAINT moderation_action_kind_target_consistency CHECK (
        target_kind IS NULL
        OR (action_type = 'hide' AND target_kind = 'event')
        OR (action_type IN ('mute', 'kick', 'ban') AND target_kind = 'principal')
    );

CREATE INDEX moderation_action_room_time_idx
    ON agent_room.moderation_action (room_catalog_id, starts_at DESC)
    WHERE room_catalog_id IS NOT NULL;

CREATE INDEX moderation_action_effective_idx
    ON agent_room.moderation_action (room_catalog_id, action_type, target_reference)
    WHERE status = 'applied';

CREATE TABLE agent_room.moderation_operator (
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    role text NOT NULL,
    granted_by uuid NOT NULL REFERENCES agent_room.principal(id),
    granted_at timestamptz NOT NULL,
    revoked_at timestamptz,
    PRIMARY KEY (principal_id, role),
    CONSTRAINT moderation_operator_role CHECK (role IN ('moderator', 'audit_reader')),
    CONSTRAINT moderation_operator_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= granted_at
    )
);

CREATE INDEX moderation_operator_active_idx
    ON agent_room.moderation_operator (principal_id, role)
    WHERE revoked_at IS NULL;

ALTER TABLE agent_room.audit_event
    ADD CONSTRAINT audit_event_moderation_metadata_whitelist CHECK (
        action NOT LIKE 'moderation.%'
        OR metadata - ARRAY['roomCatalogId'] = '{}'::jsonb
    );

GRANT SELECT, INSERT, UPDATE
    ON agent_room.moderation_report_rate
    TO agent_room_runtime;

GRANT SELECT
    ON agent_room.moderation_operator
    TO agent_room_runtime;

REVOKE DELETE ON agent_room.moderation_report_rate,
                 agent_room.moderation_operator
    FROM agent_room_runtime;

REVOKE INSERT, UPDATE ON agent_room.moderation_operator
    FROM agent_room_runtime;
