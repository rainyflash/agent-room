ALTER TABLE agent_room.agent_instance
    ADD CONSTRAINT agent_instance_id_agent_unique UNIQUE (id, agent_id);

ALTER TABLE agent_room.automation_grant
    DROP CONSTRAINT automation_grant_rate,
    DROP CONSTRAINT automation_grant_total,
    DROP CONSTRAINT automation_grant_time_order;

ALTER TABLE agent_room.automation_grant
    ADD COLUMN requires_risk_scan boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT automation_grant_instance_agent_fk
        FOREIGN KEY (agent_instance_id, agent_id)
        REFERENCES agent_room.agent_instance(id, agent_id),
    ADD CONSTRAINT automation_grant_message_kinds_known CHECK (
        allowed_message_kinds <@ ARRAY['room_message', 'reply']::text[]
        AND cardinality(allowed_message_kinds) <= 2
    ),
    ADD CONSTRAINT automation_grant_rate CHECK (
        max_messages_per_minute BETWEEN 1 AND 60
    ),
    ADD CONSTRAINT automation_grant_total CHECK (
        max_total_messages IS NULL OR max_total_messages BETWEEN 1 AND 10000
    ),
    ADD CONSTRAINT automation_grant_time_order CHECK (
        created_at <= starts_at
        AND expires_at > starts_at
        AND expires_at <= starts_at + interval '30 days'
    ),
    ADD CONSTRAINT automation_grant_unknown_recipient_scan CHECK (
        NOT allow_unknown_recipients OR requires_risk_scan
    );

CREATE TABLE agent_room.automation_consumption (
    submission_id uuid PRIMARY KEY,
    grant_id uuid NOT NULL REFERENCES agent_room.automation_grant(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    agent_instance_id uuid NOT NULL,
    room_catalog_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    matrix_room_id text NOT NULL,
    message_kind text NOT NULL,
    contains_unknown_recipients boolean NOT NULL,
    risk_scan_outcome text NOT NULL,
    minute_window_start timestamptz NOT NULL,
    consumed_at timestamptz NOT NULL,
    CONSTRAINT automation_consumption_submission_id_v7 CHECK (
        substring(submission_id::text, 15, 1) = '7'
    ),
    CONSTRAINT automation_consumption_instance_agent_fk
        FOREIGN KEY (agent_instance_id, agent_id)
        REFERENCES agent_room.agent_instance(id, agent_id),
    CONSTRAINT automation_consumption_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    ),
    CONSTRAINT automation_consumption_message_kind CHECK (
        message_kind IN ('room_message', 'reply')
    ),
    CONSTRAINT automation_consumption_risk_scan CHECK (
        risk_scan_outcome IN ('passed', 'rejected', 'unavailable', 'not_requested')
    ),
    CONSTRAINT automation_consumption_window_order CHECK (
        consumed_at >= minute_window_start
        AND consumed_at < minute_window_start + interval '1 minute'
    )
);

CREATE INDEX automation_consumption_total_idx
    ON agent_room.automation_consumption (grant_id, consumed_at);

CREATE INDEX automation_consumption_minute_idx
    ON agent_room.automation_consumption (grant_id, minute_window_start);

CREATE TABLE agent_room.automation_denial (
    grant_id uuid NOT NULL REFERENCES agent_room.automation_grant(id),
    submission_id uuid NOT NULL,
    decision_code text NOT NULL,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    agent_instance_id uuid NOT NULL,
    room_catalog_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    matrix_room_id text NOT NULL,
    decided_at timestamptz NOT NULL,
    PRIMARY KEY (grant_id, submission_id, decision_code),
    CONSTRAINT automation_denial_submission_id_v7 CHECK (
        substring(submission_id::text, 15, 1) = '7'
    ),
    CONSTRAINT automation_denial_instance_agent_fk
        FOREIGN KEY (agent_instance_id, agent_id)
        REFERENCES agent_room.agent_instance(id, agent_id),
    CONSTRAINT automation_denial_decision_code_length CHECK (
        length(decision_code) BETWEEN 1 AND 128
    ),
    CONSTRAINT automation_denial_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    )
);

CREATE INDEX automation_denial_principal_time_idx
    ON agent_room.automation_denial (principal_id, decided_at DESC);

CREATE TRIGGER automation_denial_is_append_only
BEFORE UPDATE OR DELETE ON agent_room.automation_denial
FOR EACH ROW EXECUTE FUNCTION agent_room.reject_audit_mutation();

GRANT SELECT, INSERT
    ON agent_room.automation_consumption,
       agent_room.automation_denial
    TO agent_room_runtime;

REVOKE UPDATE, DELETE
    ON agent_room.automation_consumption,
       agent_room.automation_denial
    FROM agent_room_runtime;
