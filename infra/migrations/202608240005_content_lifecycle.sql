ALTER TABLE agent_room.content_object
    ADD COLUMN updated_at timestamptz;

UPDATE agent_room.content_object
SET updated_at = created_at;

ALTER TABLE agent_room.content_object
    ALTER COLUMN updated_at SET NOT NULL,
    ADD COLUMN version bigint NOT NULL DEFAULT 0,
    ADD CONSTRAINT content_object_positive_byte_length CHECK (byte_length > 0),
    ADD CONSTRAINT content_object_updated_order CHECK (updated_at >= created_at),
    ADD CONSTRAINT content_object_version_nonnegative CHECK (version >= 0),
    ADD CONSTRAINT content_object_owner_pair_unique UNIQUE (id, owner_principal_id);

ALTER TABLE agent_room.content_access_policy
    ADD COLUMN updated_at timestamptz;

UPDATE agent_room.content_access_policy
SET updated_at = created_at;

ALTER TABLE agent_room.content_access_policy
    ALTER COLUMN updated_at SET NOT NULL,
    ADD CONSTRAINT content_access_policy_updated_order CHECK (updated_at >= created_at),
    ADD CONSTRAINT content_access_policy_content_unique UNIQUE (content_id);

CREATE UNIQUE INDEX content_access_policy_event_unique
    ON agent_room.content_access_policy (matrix_room_id, matrix_event_id)
    WHERE matrix_event_id IS NOT NULL AND revoked_at IS NULL;

CREATE TABLE agent_room.content_upload_request (
    request_id uuid PRIMARY KEY,
    owner_principal_id uuid NOT NULL,
    content_id uuid NOT NULL UNIQUE,
    declaration_fingerprint bytea NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT content_upload_request_id_v7 CHECK (
        substring(request_id::text, 15, 1) = '7'
    ),
    CONSTRAINT content_upload_request_owner_fk
        FOREIGN KEY (owner_principal_id)
        REFERENCES agent_room.principal(id),
    CONSTRAINT content_upload_request_content_owner_fk
        FOREIGN KEY (content_id, owner_principal_id)
        REFERENCES agent_room.content_object(id, owner_principal_id),
    CONSTRAINT content_upload_request_fingerprint_length CHECK (
        octet_length(declaration_fingerprint) = 32
    ),
    CONSTRAINT content_upload_request_time_order CHECK (
        updated_at >= created_at
    )
);

CREATE INDEX content_upload_request_owner_created_idx
    ON agent_room.content_upload_request (owner_principal_id, created_at DESC);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.content_upload_request
    TO agent_room_runtime;
