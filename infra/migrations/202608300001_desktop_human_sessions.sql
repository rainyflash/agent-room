ALTER TABLE agent_room.oidc_login_attempt
    ADD COLUMN delivery_kind text NOT NULL DEFAULT 'web',
    ADD COLUMN desktop_client_state text,
    ADD COLUMN desktop_pkce_challenge text,
    ADD CONSTRAINT oidc_login_attempt_delivery_kind
        CHECK (delivery_kind IN ('web', 'desktop')),
    ADD CONSTRAINT oidc_login_attempt_desktop_delivery
        CHECK (
            (
                delivery_kind = 'web'
                AND desktop_client_state IS NULL
                AND desktop_pkce_challenge IS NULL
            )
            OR
            (
                delivery_kind = 'desktop'
                AND length(desktop_client_state) BETWEEN 32 AND 128
                AND desktop_client_state ~ '^[A-Za-z0-9_-]+$'
                AND length(desktop_pkce_challenge) = 43
                AND desktop_pkce_challenge ~ '^[A-Za-z0-9_-]+$'
            )
        );

CREATE TABLE agent_room.desktop_authorization_code (
    code_digest bytea PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    pkce_challenge text NOT NULL,
    authenticated_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    CONSTRAINT desktop_authorization_code_digest_length CHECK (
        octet_length(code_digest) = 32
    ),
    CONSTRAINT desktop_authorization_code_pkce CHECK (
        length(pkce_challenge) = 43
        AND pkce_challenge ~ '^[A-Za-z0-9_-]+$'
    ),
    CONSTRAINT desktop_authorization_code_expiry_order CHECK (expires_at > created_at),
    CONSTRAINT desktop_authorization_code_authentication_order CHECK (
        authenticated_at <= created_at + interval '5 minutes'
    ),
    CONSTRAINT desktop_authorization_code_consumed_order CHECK (
        consumed_at IS NULL OR consumed_at >= created_at
    )
);

CREATE INDEX desktop_authorization_code_expiry_idx
    ON agent_room.desktop_authorization_code (expires_at)
    WHERE consumed_at IS NULL;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.desktop_authorization_code
    TO agent_room_runtime;
