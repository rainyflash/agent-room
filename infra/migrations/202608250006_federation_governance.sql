CREATE TABLE agent_room.federation_peer (
    server_name text PRIMARY KEY,
    reputation_score smallint NOT NULL DEFAULT 0,
    disposition text NOT NULL DEFAULT 'allow',
    observed_protocol_majors smallint[] NOT NULL DEFAULT '{}',
    last_observed_at timestamptz,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT federation_peer_server_name_length CHECK (
        length(server_name) BETWEEN 1 AND 255
    ),
    CONSTRAINT federation_peer_reputation_range CHECK (
        reputation_score BETWEEN -100 AND 100
    ),
    CONSTRAINT federation_peer_disposition CHECK (
        disposition IN ('allow', 'throttle', 'quarantine', 'block')
    ),
    CONSTRAINT federation_peer_protocol_majors CHECK (
        cardinality(observed_protocol_majors) <= 8
        AND 0 < ALL(observed_protocol_majors)
    ),
    CONSTRAINT federation_peer_version_nonnegative CHECK (version >= 0)
);

CREATE TABLE agent_room.federation_governance_rule (
    id uuid PRIMARY KEY,
    scope_kind text NOT NULL,
    server_name text NOT NULL,
    subject_reference text,
    disposition text NOT NULL,
    reason text NOT NULL,
    created_by uuid NOT NULL REFERENCES agent_room.principal(id),
    created_at timestamptz NOT NULL,
    expires_at timestamptz,
    revoked_by uuid REFERENCES agent_room.principal(id),
    revoked_at timestamptz,
    revocation_reason text,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT federation_governance_rule_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT federation_governance_rule_scope CHECK (
        scope_kind IN ('server', 'room', 'user')
    ),
    CONSTRAINT federation_governance_rule_server_name_length CHECK (
        length(server_name) BETWEEN 1 AND 255
    ),
    CONSTRAINT federation_governance_rule_subject_consistency CHECK (
        (scope_kind = 'server' AND subject_reference IS NULL)
        OR (
            scope_kind IN ('room', 'user')
            AND length(subject_reference) BETWEEN 2 AND 1024
        )
    ),
    CONSTRAINT federation_governance_rule_disposition CHECK (
        disposition IN ('allow', 'throttle', 'quarantine', 'block')
    ),
    CONSTRAINT federation_governance_rule_reason_length CHECK (
        length(reason) BETWEEN 1 AND 500
    ),
    CONSTRAINT federation_governance_rule_expiry_order CHECK (
        expires_at IS NULL OR expires_at > created_at
    ),
    CONSTRAINT federation_governance_rule_revocation_consistency CHECK (
        (
            revoked_at IS NULL
            AND revoked_by IS NULL
            AND revocation_reason IS NULL
        )
        OR (
            revoked_at >= created_at
            AND revoked_by IS NOT NULL
            AND length(revocation_reason) BETWEEN 1 AND 500
        )
    ),
    CONSTRAINT federation_governance_rule_version_nonnegative CHECK (version >= 0)
);

CREATE UNIQUE INDEX federation_governance_rule_active_target_idx
    ON agent_room.federation_governance_rule (
        scope_kind,
        server_name,
        COALESCE(subject_reference, ''),
        disposition
    )
    WHERE revoked_at IS NULL;

CREATE INDEX federation_governance_rule_active_peer_idx
    ON agent_room.federation_governance_rule (server_name, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE agent_room.federation_governance_audit (
    id uuid PRIMARY KEY,
    rule_id uuid REFERENCES agent_room.federation_governance_rule(id),
    action text NOT NULL,
    scope_kind text NOT NULL,
    server_name text NOT NULL,
    subject_reference text,
    disposition text NOT NULL,
    actor_principal_id uuid REFERENCES agent_room.principal(id),
    reason_code text NOT NULL,
    occurred_at timestamptz NOT NULL,
    correlation_id uuid NOT NULL,
    CONSTRAINT federation_governance_audit_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT federation_governance_audit_action CHECK (
        action IN (
            'rule_created',
            'rule_revoked',
            'auto_throttled',
            'auto_quarantined',
            'event_rejected'
        )
    ),
    CONSTRAINT federation_governance_audit_scope CHECK (
        scope_kind IN ('server', 'room', 'user')
    ),
    CONSTRAINT federation_governance_audit_disposition CHECK (
        disposition IN ('allow', 'throttle', 'quarantine', 'block')
    ),
    CONSTRAINT federation_governance_audit_reason_code CHECK (
        reason_code ~ '^[a-z][a-z0-9_.]{0,127}$'
    ),
    CONSTRAINT federation_governance_audit_correlation_id_v7 CHECK (
        substring(correlation_id::text, 15, 1) = '7'
    )
);

CREATE INDEX federation_governance_audit_peer_time_idx
    ON agent_room.federation_governance_audit (server_name, occurred_at DESC);

GRANT SELECT, INSERT, UPDATE
    ON agent_room.federation_peer,
       agent_room.federation_governance_rule
    TO agent_room_runtime;

GRANT SELECT, INSERT
    ON agent_room.federation_governance_audit
    TO agent_room_runtime;

REVOKE DELETE
    ON agent_room.federation_peer,
       agent_room.federation_governance_rule,
       agent_room.federation_governance_audit
    FROM agent_room_runtime;

REVOKE UPDATE
    ON agent_room.federation_governance_audit
    FROM agent_room_runtime;
