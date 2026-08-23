CREATE TABLE agent_room.oidc_login_attempt (
    id uuid PRIMARY KEY,
    browser_secret_digest bytea NOT NULL UNIQUE,
    state_digest bytea NOT NULL UNIQUE,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    return_path text NOT NULL,
    import_display_name boolean NOT NULL DEFAULT false,
    import_locale boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    CONSTRAINT oidc_login_attempt_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT oidc_login_attempt_browser_digest_length CHECK (
        octet_length(browser_secret_digest) = 32
    ),
    CONSTRAINT oidc_login_attempt_state_digest_length CHECK (
        octet_length(state_digest) = 32
    ),
    CONSTRAINT oidc_login_attempt_nonce_length CHECK (length(nonce) BETWEEN 16 AND 4096),
    CONSTRAINT oidc_login_attempt_pkce_length CHECK (length(pkce_verifier) BETWEEN 43 AND 128),
    CONSTRAINT oidc_login_attempt_return_path CHECK (
        length(return_path) BETWEEN 1 AND 2048
        AND return_path LIKE '/%'
        AND return_path NOT LIKE '//%'
        AND position(E'\\' IN return_path) = 0
        AND return_path !~ '[[:cntrl:]]'
    ),
    CONSTRAINT oidc_login_attempt_expiry_order CHECK (expires_at > created_at),
    CONSTRAINT oidc_login_attempt_consumed_order CHECK (
        consumed_at IS NULL OR consumed_at >= created_at
    )
);

CREATE INDEX oidc_login_attempt_expiry_idx
    ON agent_room.oidc_login_attempt (expires_at)
    WHERE consumed_at IS NULL;

CREATE TABLE agent_room.web_session (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    secret_digest bytea NOT NULL UNIQUE,
    authenticated_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CONSTRAINT web_session_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT web_session_secret_digest_length CHECK (octet_length(secret_digest) = 32),
    CONSTRAINT web_session_expiry_order CHECK (expires_at > created_at),
    CONSTRAINT web_session_authentication_order CHECK (
        authenticated_at <= created_at + interval '5 minutes'
    ),
    CONSTRAINT web_session_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE INDEX web_session_principal_active_idx
    ON agent_room.web_session (principal_id, expires_at)
    WHERE revoked_at IS NULL;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.oidc_login_attempt, agent_room.web_session
    TO agent_room_runtime;
