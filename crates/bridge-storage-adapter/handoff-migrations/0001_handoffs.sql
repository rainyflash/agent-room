-- Handoff 正文只以应用层密文存在；本库不与通用 Bridge 元数据库混用。
CREATE TABLE context_handoff (
    handoff_id TEXT PRIMARY KEY NOT NULL,
    requester_agent_id TEXT NOT NULL,
    requester_instance_id TEXT NOT NULL,
    source_room_id TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    source_message_id TEXT NOT NULL,
    source_agent_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_provenance TEXT NOT NULL
        CHECK (source_provenance IN ('human', 'human_confirmed_agent', 'autonomous_agent')),
    target_agent_id TEXT NOT NULL,
    target_instance_id TEXT NOT NULL,
    content_id TEXT NOT NULL,
    content_digest BLOB NOT NULL CHECK (length(content_digest) = 32),
    content_byte_length INTEGER NOT NULL
        CHECK (content_byte_length BETWEEN 1 AND 26214400),
    content_media_type TEXT NOT NULL,
    permissions_json TEXT NOT NULL
        CHECK (json_valid(permissions_json) AND json_array_length(permissions_json) > 0),
    purpose TEXT NOT NULL CHECK (purpose IN ('inspect', 'summarize', 'reply_draft')),
    risk_flags_json TEXT NOT NULL CHECK (json_valid(risk_flags_json)),
    proposed_at_unix_ms INTEGER NOT NULL CHECK (proposed_at_unix_ms >= 0),
    expires_at_unix_ms INTEGER NOT NULL CHECK (expires_at_unix_ms > proposed_at_unix_ms),
    status TEXT NOT NULL
        CHECK (status IN ('proposed', 'approved', 'delivered', 'consumed', 'declined', 'revoked', 'expired', 'failed')),
    approved_by_principal_id TEXT,
    approved_at_unix_ms INTEGER,
    delivered_at_unix_ms INTEGER,
    consumed_at_unix_ms INTEGER,
    resolved_at_unix_ms INTEGER,
    failure_code TEXT,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    CHECK ((approved_by_principal_id IS NULL) = (approved_at_unix_ms IS NULL)),
    CHECK (
        (status = 'proposed'
            AND approved_at_unix_ms IS NULL AND delivered_at_unix_ms IS NULL
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NULL
            AND failure_code IS NULL)
        OR (status = 'approved'
            AND approved_at_unix_ms IS NOT NULL AND delivered_at_unix_ms IS NULL
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NULL
            AND failure_code IS NULL)
        OR (status = 'delivered'
            AND approved_at_unix_ms IS NOT NULL AND delivered_at_unix_ms IS NOT NULL
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NULL
            AND failure_code IS NULL)
        OR (status = 'consumed'
            AND approved_at_unix_ms IS NOT NULL AND delivered_at_unix_ms IS NOT NULL
            AND consumed_at_unix_ms IS NOT NULL
            AND resolved_at_unix_ms = consumed_at_unix_ms AND failure_code IS NULL)
        OR (status = 'declined'
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NOT NULL
            AND failure_code IS NULL)
        OR (status = 'revoked'
            AND approved_at_unix_ms IS NOT NULL AND consumed_at_unix_ms IS NULL
            AND resolved_at_unix_ms IS NOT NULL AND failure_code IS NULL)
        OR (status = 'expired'
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NOT NULL
            AND failure_code IS NULL)
        OR (status = 'failed'
            AND approved_at_unix_ms IS NOT NULL AND delivered_at_unix_ms IS NULL
            AND consumed_at_unix_ms IS NULL AND resolved_at_unix_ms IS NOT NULL
            AND failure_code IS NOT NULL)
    )
);

CREATE INDEX context_handoff_target_status_idx
    ON context_handoff (target_instance_id, status);

CREATE INDEX context_handoff_expiry_idx
    ON context_handoff (status, expires_at_unix_ms);

CREATE TABLE context_handoff_package (
    handoff_id TEXT PRIMARY KEY NOT NULL
        REFERENCES context_handoff(handoff_id) ON DELETE CASCADE,
    key_version INTEGER NOT NULL CHECK (key_version = 1),
    nonce BLOB NOT NULL CHECK (length(nonce) = 24),
    ciphertext BLOB NOT NULL CHECK (length(ciphertext) >= 16)
);
