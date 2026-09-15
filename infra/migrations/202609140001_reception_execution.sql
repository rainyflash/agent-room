CREATE TABLE agent_room.reception_execution (
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    catalog_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    room_id text NOT NULL,
    session_key uuid NOT NULL,
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 128),
    instance_id uuid NOT NULL REFERENCES agent_room.agent_instance(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    run_id uuid NOT NULL,
    state text NOT NULL CHECK (state IN ('active', 'draining', 'idle')),
    next_device_id uuid REFERENCES agent_room.device(id),
    last_seen_unix_ms bigint NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CHECK (revision >= 0),
    progress jsonb NOT NULL CHECK (jsonb_typeof(progress) = 'object'),
    PRIMARY KEY (agent_id, catalog_id)
);
CREATE INDEX reception_execution_device ON agent_room.reception_execution(device_id, state);
