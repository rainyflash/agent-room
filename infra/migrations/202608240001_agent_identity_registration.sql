ALTER TABLE agent_room.agent_instance
    DROP CONSTRAINT agent_instance_signing_key_length;

ALTER TABLE agent_room.agent_instance
    ADD CONSTRAINT agent_instance_signing_key_length CHECK (
        octet_length(public_signing_key) = 32
    );

CREATE UNIQUE INDEX adapter_binding_identity_unique
    ON agent_room.adapter_binding (agent_id, adapter_type, external_subject_hash)
    NULLS NOT DISTINCT;

CREATE UNIQUE INDEX agent_instance_active_signing_key_unique
    ON agent_room.agent_instance (public_signing_key)
    WHERE revoked_at IS NULL;

CREATE UNIQUE INDEX agent_instance_active_binding_device_unique
    ON agent_room.agent_instance (agent_id, device_id, adapter_binding_id)
    WHERE revoked_at IS NULL;

CREATE TABLE agent_room.agent_creation_request (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    agent_id uuid NOT NULL UNIQUE,
    request_fingerprint bytea NOT NULL,
    state text NOT NULL DEFAULT 'reserved',
    created_at timestamptz NOT NULL,
    completed_at timestamptz,
    CONSTRAINT agent_creation_request_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT agent_creation_request_agent_id_v7 CHECK (
        substring(agent_id::text, 15, 1) = '7'
    ),
    CONSTRAINT agent_creation_request_fingerprint_length CHECK (
        octet_length(request_fingerprint) = 32
    ),
    CONSTRAINT agent_creation_request_state CHECK (
        state IN ('reserved', 'completed')
    ),
    CONSTRAINT agent_creation_request_completion_consistency CHECK (
        (state = 'reserved' AND completed_at IS NULL)
        OR (state = 'completed' AND completed_at IS NOT NULL)
    ),
    CONSTRAINT agent_creation_request_completion_order CHECK (
        completed_at IS NULL OR completed_at >= created_at
    )
);

CREATE INDEX agent_creation_request_principal_idx
    ON agent_room.agent_creation_request (principal_id, created_at DESC);

CREATE TABLE agent_room.agent_instance_registration_request (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    adapter_binding_id uuid NOT NULL REFERENCES agent_room.adapter_binding(id),
    agent_instance_id uuid NOT NULL UNIQUE REFERENCES agent_room.agent_instance(id),
    request_fingerprint bytea NOT NULL,
    created_at timestamptz NOT NULL,
    CONSTRAINT agent_instance_registration_request_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT agent_instance_registration_request_fingerprint_length CHECK (
        octet_length(request_fingerprint) = 32
    )
);

CREATE INDEX agent_instance_registration_request_principal_idx
    ON agent_room.agent_instance_registration_request (principal_id, created_at DESC);

CREATE FUNCTION agent_room.enforce_completed_agent_creation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.state = 'completed' AND NOT EXISTS (
        SELECT 1
        FROM agent_room.agent
        WHERE id = NEW.agent_id
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23503',
            MESSAGE = '已完成的 Agent 创建请求必须引用存在的 Agent';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER completed_agent_creation_requires_agent
AFTER INSERT OR UPDATE ON agent_room.agent_creation_request
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION agent_room.enforce_completed_agent_creation();

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.agent_creation_request,
       agent_room.agent_instance_registration_request
    TO agent_room_runtime;

REVOKE ALL ON FUNCTION agent_room.enforce_completed_agent_creation() FROM PUBLIC;
