ALTER TABLE agent_room.context_handoff
    ADD COLUMN source_message_id uuid,
    ADD COLUMN request_fingerprint bytea,
    ADD COLUMN permissions text[] NOT NULL DEFAULT ARRAY['read_text']::text[],
    ADD COLUMN created_at timestamptz,
    ADD COLUMN queued_at timestamptz,
    ADD COLUMN resolved_at timestamptz;

ALTER TABLE agent_room.context_handoff
    DROP CONSTRAINT context_handoff_state;

ALTER TABLE agent_room.context_handoff
    ADD CONSTRAINT context_handoff_state CHECK (
        state IN (
            'proposed', 'approved', 'queued', 'delivered', 'consumed',
            'declined', 'revoked', 'expired', 'failed'
        )
    ),
    ADD CONSTRAINT context_handoff_source_message_v7 CHECK (
        source_message_id IS NULL OR substring(source_message_id::text, 15, 1) = '7'
    ),
    ADD CONSTRAINT context_handoff_request_fingerprint_length CHECK (
        request_fingerprint IS NULL OR octet_length(request_fingerprint) = 32
    ),
    ADD CONSTRAINT context_handoff_permissions CHECK (
        cardinality(permissions) BETWEEN 1 AND 3
        AND permissions <@ ARRAY['read_text', 'read_attachments', 'include_metadata']::text[]
    ),
    ADD CONSTRAINT context_handoff_cloud_timestamp_order CHECK (
        (created_at IS NULL OR created_at < expires_at)
        AND (queued_at IS NULL OR (created_at IS NOT NULL AND queued_at >= created_at))
        AND (resolved_at IS NULL OR resolved_at >= COALESCE(delivered_at, queued_at, approved_at))
        AND (
            source_message_id IS NULL
            OR resolved_at IS NULL
            OR state = 'expired'
            OR resolved_at < expires_at
        )
    );

DROP INDEX agent_room.context_handoff_active_unique;

CREATE UNIQUE INDEX context_handoff_active_unique
    ON agent_room.context_handoff (
        principal_id,
        target_agent_instance_id,
        source_matrix_event_id,
        content_id,
        allowed_purpose
    )
    WHERE state IN ('proposed', 'approved', 'queued', 'delivered');

CREATE INDEX context_handoff_principal_history_idx
    ON agent_room.context_handoff (principal_id, created_at DESC, id)
    WHERE source_message_id IS NOT NULL;

-- 只为旧版内置 Bridge 补齐能力；第三方适配器必须在重新登记时自行声明。
UPDATE agent_room.adapter_binding
SET configuration = CASE
        WHEN jsonb_typeof(configuration -> 'capabilities') = 'array'
            THEN jsonb_set(
                configuration,
                '{capabilities}',
                (configuration -> 'capabilities') || '["targeted_handoff_v1"]'::jsonb,
                true
            )
        ELSE jsonb_set(
            configuration,
            '{capabilities}',
            '["targeted_handoff_v1"]'::jsonb,
            true
        )
    END,
    updated_at = GREATEST(updated_at, now())
WHERE state = 'active'
  AND adapter_type = 'codex-desktop'
  AND NOT (configuration @> '{"capabilities":["targeted_handoff_v1"]}'::jsonb);
