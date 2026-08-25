ALTER TABLE agent_room.room_catalog_entry
    ADD CONSTRAINT room_catalog_entry_direct_owner_scope CHECK (
        kind <> 'direct' OR owner_principal_id IS NOT NULL
    );

CREATE TABLE agent_room.direct_session (
    catalog_entry_id uuid PRIMARY KEY REFERENCES agent_room.room_catalog_entry(id) ON DELETE CASCADE,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    target_agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    lifecycle_state text NOT NULL DEFAULT 'provisioning',
    version bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT direct_session_participants_unique UNIQUE (principal_id, target_agent_id),
    CONSTRAINT direct_session_lifecycle CHECK (
        lifecycle_state IN ('provisioning', 'active', 'failed')
    ),
    CONSTRAINT direct_session_lifecycle_version CHECK (
        (lifecycle_state = 'provisioning' AND version = 0)
        OR (lifecycle_state IN ('active', 'failed') AND version > 0)
    ),
    CONSTRAINT direct_session_version_nonnegative CHECK (version >= 0),
    CONSTRAINT direct_session_timestamp_order CHECK (updated_at >= created_at)
);

CREATE INDEX direct_session_principal_activity_idx
    ON agent_room.direct_session (principal_id, updated_at DESC)
    WHERE lifecycle_state = 'active';

CREATE TABLE agent_room.direct_contact_block (
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    blocker_kind text NOT NULL,
    blocked_at timestamptz NOT NULL,
    revoked_at timestamptz,
    PRIMARY KEY (principal_id, agent_id, blocker_kind),
    CONSTRAINT direct_contact_blocker_kind CHECK (blocker_kind IN ('principal', 'agent')),
    CONSTRAINT direct_contact_block_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= blocked_at
    )
);

CREATE INDEX direct_contact_block_active_idx
    ON agent_room.direct_contact_block (principal_id, agent_id)
    WHERE revoked_at IS NULL;
