ALTER TABLE agent_room.device
    ADD COLUMN signing_algorithm text NOT NULL DEFAULT 'ed25519',
    ADD COLUMN verified_at timestamptz,
    ADD COLUMN version bigint NOT NULL DEFAULT 0;

UPDATE agent_room.device
SET verified_at = created_at
WHERE trust_state = 'verified';

ALTER TABLE agent_room.device
    DROP CONSTRAINT device_revocation_consistency,
    ADD CONSTRAINT device_signing_algorithm CHECK (signing_algorithm = 'ed25519'),
    ADD CONSTRAINT device_version_nonnegative CHECK (version >= 0),
    ADD CONSTRAINT device_timestamp_order CHECK (
        (last_seen_at IS NULL OR last_seen_at >= created_at)
        AND (verified_at IS NULL OR verified_at >= created_at)
        AND (revoked_at IS NULL OR revoked_at >= created_at)
    ),
    ADD CONSTRAINT device_trust_timestamp_consistency CHECK (
        (trust_state = 'pending' AND verified_at IS NULL AND revoked_at IS NULL)
        OR (trust_state = 'verified' AND verified_at IS NOT NULL AND revoked_at IS NULL)
        OR (
            trust_state = 'revoked'
            AND revoked_at IS NOT NULL
            AND (verified_at IS NULL OR revoked_at >= verified_at)
        )
    );

CREATE TABLE agent_room.device_token_family (
    id uuid PRIMARY KEY,
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    state text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    compromise_detected_at timestamptz,
    CONSTRAINT device_token_family_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT device_token_family_state CHECK (
        state IN ('active', 'revoked', 'compromised')
    ),
    CONSTRAINT device_token_family_expiry_order CHECK (expires_at > created_at),
    CONSTRAINT device_token_family_state_consistency CHECK (
        (state = 'active' AND revoked_at IS NULL AND compromise_detected_at IS NULL)
        OR (state = 'revoked' AND revoked_at IS NOT NULL AND compromise_detected_at IS NULL)
        OR (
            state = 'compromised'
            AND revoked_at IS NOT NULL
            AND compromise_detected_at IS NOT NULL
        )
    ),
    CONSTRAINT device_token_family_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    ),
    CONSTRAINT device_token_family_compromise_order CHECK (
        compromise_detected_at IS NULL
        OR (
            compromise_detected_at >= created_at
            AND revoked_at <= compromise_detected_at
        )
    )
);

CREATE UNIQUE INDEX device_token_family_one_active_per_device
    ON agent_room.device_token_family (device_id)
    WHERE state = 'active';

CREATE INDEX device_token_family_device_history_idx
    ON agent_room.device_token_family (device_id, created_at DESC);

CREATE TABLE agent_room.device_access_token (
    id uuid PRIMARY KEY,
    family_id uuid NOT NULL REFERENCES agent_room.device_token_family(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    secret_digest bytea NOT NULL UNIQUE,
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CONSTRAINT device_access_token_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT device_access_token_digest_length CHECK (octet_length(secret_digest) = 32),
    CONSTRAINT device_access_token_expiry_order CHECK (expires_at > issued_at),
    CONSTRAINT device_access_token_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= issued_at
    )
);

CREATE INDEX device_access_token_family_idx
    ON agent_room.device_access_token (family_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE INDEX device_access_token_device_idx
    ON agent_room.device_access_token (device_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE agent_room.device_refresh_token (
    id uuid PRIMARY KEY,
    family_id uuid NOT NULL REFERENCES agent_room.device_token_family(id),
    secret_digest bytea NOT NULL UNIQUE,
    sequence bigint NOT NULL,
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    replaced_by_id uuid REFERENCES agent_room.device_refresh_token(id),
    revoked_at timestamptz,
    CONSTRAINT device_refresh_token_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT device_refresh_token_digest_length CHECK (octet_length(secret_digest) = 32),
    CONSTRAINT device_refresh_token_sequence_nonnegative CHECK (sequence >= 0),
    CONSTRAINT device_refresh_token_expiry_order CHECK (expires_at > issued_at),
    CONSTRAINT device_refresh_token_consumed_order CHECK (
        consumed_at IS NULL OR consumed_at >= issued_at
    ),
    CONSTRAINT device_refresh_token_replacement_consistency CHECK (
        (consumed_at IS NULL AND replaced_by_id IS NULL)
        OR (consumed_at IS NOT NULL AND replaced_by_id IS NOT NULL)
    ),
    CONSTRAINT device_refresh_token_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= issued_at
    ),
    CONSTRAINT device_refresh_token_not_self_replaced CHECK (replaced_by_id <> id),
    CONSTRAINT device_refresh_token_family_sequence_unique UNIQUE (family_id, sequence)
);

CREATE INDEX device_refresh_token_family_idx
    ON agent_room.device_refresh_token (family_id, sequence DESC);

CREATE TABLE agent_room.device_proof_nonce (
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    nonce_digest bytea NOT NULL,
    consumed_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (device_id, nonce_digest),
    CONSTRAINT device_proof_nonce_digest_length CHECK (octet_length(nonce_digest) = 32),
    CONSTRAINT device_proof_nonce_expiry_order CHECK (expires_at > consumed_at)
);

CREATE INDEX device_proof_nonce_expiry_idx
    ON agent_room.device_proof_nonce (expires_at);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.device_token_family,
       agent_room.device_access_token,
       agent_room.device_refresh_token,
       agent_room.device_proof_nonce
    TO agent_room_runtime;
