CREATE TABLE agent_room.device_authorization_receipt (
    authorization_digest bytea PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    consumed_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    CONSTRAINT device_authorization_receipt_digest_length CHECK (
        octet_length(authorization_digest) = 32
    ),
    CONSTRAINT device_authorization_receipt_expiry_order CHECK (
        expires_at > consumed_at
    )
);

CREATE INDEX device_authorization_receipt_expiry_idx
    ON agent_room.device_authorization_receipt (expires_at);

GRANT SELECT, INSERT, DELETE
    ON agent_room.device_authorization_receipt
    TO agent_room_runtime;
