CREATE SCHEMA agent_room;

REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON SCHEMA agent_room FROM PUBLIC;

CREATE TABLE agent_room.principal (
    id uuid PRIMARY KEY,
    oidc_issuer text NOT NULL,
    oidc_subject text NOT NULL,
    matrix_user_id text NOT NULL,
    display_name text NOT NULL,
    avatar_content_id uuid,
    locale text NOT NULL DEFAULT 'en',
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT principal_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT principal_oidc_issuer_length CHECK (length(oidc_issuer) BETWEEN 1 AND 2048),
    CONSTRAINT principal_oidc_subject_length CHECK (length(oidc_subject) BETWEEN 1 AND 512),
    CONSTRAINT principal_matrix_user_id_format CHECK (
        length(matrix_user_id) BETWEEN 4 AND 512 AND matrix_user_id LIKE '@%:%'
    ),
    CONSTRAINT principal_display_name_length CHECK (length(display_name) BETWEEN 1 AND 128),
    CONSTRAINT principal_locale_format CHECK (
        locale ~ '^[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*$'
    ),
    CONSTRAINT principal_status CHECK (status IN ('active', 'suspended', 'deleting', 'deleted')),
    CONSTRAINT principal_timestamp_order CHECK (updated_at >= created_at),
    CONSTRAINT principal_version_nonnegative CHECK (version >= 0),
    CONSTRAINT principal_oidc_identity_unique UNIQUE (oidc_issuer, oidc_subject),
    CONSTRAINT principal_matrix_user_id_unique UNIQUE (matrix_user_id)
);

CREATE TABLE agent_room.device (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    label text NOT NULL,
    platform text NOT NULL,
    public_signing_key bytea NOT NULL,
    matrix_device_id text,
    trust_state text NOT NULL DEFAULT 'pending',
    last_seen_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL,
    CONSTRAINT device_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT device_label_length CHECK (length(label) BETWEEN 1 AND 128),
    CONSTRAINT device_platform CHECK (platform IN ('windows', 'macos', 'linux', 'web')),
    CONSTRAINT device_signing_key_length CHECK (octet_length(public_signing_key) BETWEEN 32 AND 128),
    CONSTRAINT device_matrix_device_id_length CHECK (
        matrix_device_id IS NULL OR length(matrix_device_id) BETWEEN 1 AND 255
    ),
    CONSTRAINT device_trust_state CHECK (trust_state IN ('pending', 'verified', 'revoked')),
    CONSTRAINT device_revocation_consistency CHECK (
        (trust_state = 'revoked' AND revoked_at IS NOT NULL)
        OR (trust_state <> 'revoked' AND revoked_at IS NULL)
    )
);

CREATE UNIQUE INDEX device_active_signing_key_unique
    ON agent_room.device (public_signing_key)
    WHERE revoked_at IS NULL;

CREATE UNIQUE INDEX device_principal_matrix_device_unique
    ON agent_room.device (principal_id, matrix_device_id)
    WHERE matrix_device_id IS NOT NULL;

CREATE TABLE agent_room.agent (
    id uuid PRIMARY KEY,
    matrix_user_id text NOT NULL,
    slug text NOT NULL,
    display_name text NOT NULL,
    description text NOT NULL DEFAULT '',
    avatar_content_id uuid,
    visibility text NOT NULL DEFAULT 'private',
    lifecycle_state text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT agent_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT agent_matrix_user_id_format CHECK (
        length(matrix_user_id) BETWEEN 4 AND 512 AND matrix_user_id LIKE '@%:%'
    ),
    CONSTRAINT agent_slug_format CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    CONSTRAINT agent_display_name_length CHECK (length(display_name) BETWEEN 1 AND 128),
    CONSTRAINT agent_description_length CHECK (length(description) <= 2048),
    CONSTRAINT agent_visibility CHECK (visibility IN ('public', 'unlisted', 'private')),
    CONSTRAINT agent_lifecycle_state CHECK (lifecycle_state IN ('active', 'suspended', 'retired')),
    CONSTRAINT agent_timestamp_order CHECK (updated_at >= created_at),
    CONSTRAINT agent_version_nonnegative CHECK (version >= 0),
    CONSTRAINT agent_matrix_user_id_unique UNIQUE (matrix_user_id),
    CONSTRAINT agent_slug_unique UNIQUE (slug)
);

CREATE TABLE agent_room.agent_ownership (
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    role text NOT NULL,
    granted_by uuid NOT NULL REFERENCES agent_room.principal(id),
    created_at timestamptz NOT NULL,
    revoked_at timestamptz,
    PRIMARY KEY (principal_id, agent_id),
    CONSTRAINT agent_ownership_role CHECK (role IN ('owner', 'operator', 'viewer')),
    CONSTRAINT agent_ownership_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE INDEX agent_ownership_active_principal_idx
    ON agent_room.agent_ownership (principal_id)
    WHERE revoked_at IS NULL;

CREATE TABLE agent_room.adapter_binding (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    adapter_type text NOT NULL,
    external_subject_hash bytea,
    capability_version text NOT NULL,
    configuration jsonb NOT NULL DEFAULT '{}'::jsonb,
    state text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT adapter_binding_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT adapter_binding_type_length CHECK (length(adapter_type) BETWEEN 1 AND 64),
    CONSTRAINT adapter_binding_subject_hash_length CHECK (
        external_subject_hash IS NULL OR octet_length(external_subject_hash) = 32
    ),
    CONSTRAINT adapter_binding_capability_version_length CHECK (
        length(capability_version) BETWEEN 1 AND 64
    ),
    CONSTRAINT adapter_binding_configuration_object CHECK (jsonb_typeof(configuration) = 'object'),
    CONSTRAINT adapter_binding_configuration_has_no_credentials CHECK (
        NOT configuration ?| ARRAY['password', 'secret', 'token', 'apiKey', 'api_key']
    ),
    CONSTRAINT adapter_binding_state CHECK (state IN ('active', 'disabled', 'incompatible')),
    CONSTRAINT adapter_binding_timestamp_order CHECK (updated_at >= created_at)
);

CREATE TABLE agent_room.agent_instance (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    adapter_binding_id uuid NOT NULL REFERENCES agent_room.adapter_binding(id),
    public_signing_key bytea NOT NULL,
    matrix_device_id text NOT NULL,
    status text NOT NULL DEFAULT 'connecting',
    lease_expires_at timestamptz,
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CONSTRAINT agent_instance_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT agent_instance_signing_key_length CHECK (
        octet_length(public_signing_key) BETWEEN 32 AND 128
    ),
    CONSTRAINT agent_instance_matrix_device_id_length CHECK (
        length(matrix_device_id) BETWEEN 1 AND 255
    ),
    CONSTRAINT agent_instance_status CHECK (
        status IN ('connecting', 'online', 'degraded', 'offline', 'revoked')
    ),
    CONSTRAINT agent_instance_online_lease CHECK (
        status <> 'online' OR lease_expires_at IS NOT NULL
    ),
    CONSTRAINT agent_instance_revocation_consistency CHECK (
        (status = 'revoked' AND revoked_at IS NOT NULL)
        OR (status <> 'revoked' AND revoked_at IS NULL)
    )
);

CREATE UNIQUE INDEX agent_instance_active_matrix_device_unique
    ON agent_room.agent_instance (agent_id, matrix_device_id)
    WHERE revoked_at IS NULL;

CREATE INDEX agent_instance_status_idx
    ON agent_room.agent_instance (agent_id, status, last_seen_at DESC);

CREATE TABLE agent_room.agent_card_snapshot (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    source_url text NOT NULL,
    canonical_digest bytea NOT NULL,
    normalized_card jsonb NOT NULL,
    verification_state text NOT NULL,
    fetched_at timestamptz NOT NULL,
    expires_at timestamptz,
    CONSTRAINT agent_card_snapshot_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT agent_card_snapshot_https CHECK (
        length(source_url) BETWEEN 9 AND 2048 AND source_url LIKE 'https://%'
    ),
    CONSTRAINT agent_card_snapshot_digest_length CHECK (octet_length(canonical_digest) = 32),
    CONSTRAINT agent_card_snapshot_card_object CHECK (jsonb_typeof(normalized_card) = 'object'),
    CONSTRAINT agent_card_snapshot_verification_state CHECK (
        verification_state IN ('verified', 'unverified', 'invalid', 'expired')
    ),
    CONSTRAINT agent_card_snapshot_expiry_order CHECK (
        expires_at IS NULL OR expires_at > fetched_at
    )
);

CREATE INDEX agent_card_snapshot_history_idx
    ON agent_room.agent_card_snapshot (agent_id, fetched_at DESC);

CREATE TABLE agent_room.room_catalog_entry (
    id uuid PRIMARY KEY,
    kind text NOT NULL,
    slug text,
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    language text,
    matrix_space_id text,
    owner_principal_id uuid REFERENCES agent_room.principal(id),
    visibility text NOT NULL,
    retention_days integer,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT room_catalog_entry_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT room_catalog_entry_kind CHECK (kind IN ('public_lobby', 'private_room', 'direct')),
    CONSTRAINT room_catalog_entry_slug_format CHECK (
        slug IS NULL OR slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'
    ),
    CONSTRAINT room_catalog_entry_name_length CHECK (length(name) BETWEEN 1 AND 128),
    CONSTRAINT room_catalog_entry_description_length CHECK (length(description) <= 2048),
    CONSTRAINT room_catalog_entry_language_format CHECK (
        language IS NULL OR language ~ '^[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*$'
    ),
    CONSTRAINT room_catalog_entry_visibility CHECK (visibility IN ('public', 'unlisted', 'private')),
    CONSTRAINT room_catalog_entry_retention CHECK (
        retention_days IS NULL OR retention_days BETWEEN 1 AND 3650
    ),
    CONSTRAINT room_catalog_entry_status CHECK (status IN ('active', 'frozen', 'archived')),
    CONSTRAINT room_catalog_entry_owner_scope CHECK (
        (kind = 'private_room' AND owner_principal_id IS NOT NULL)
        OR kind <> 'private_room'
    ),
    CONSTRAINT room_catalog_entry_timestamp_order CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX room_catalog_entry_slug_unique
    ON agent_room.room_catalog_entry (slug)
    WHERE slug IS NOT NULL;

CREATE INDEX room_catalog_entry_directory_idx
    ON agent_room.room_catalog_entry (visibility, status, language);

CREATE TABLE agent_room.room_instance (
    id uuid PRIMARY KEY,
    catalog_entry_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    matrix_room_id text NOT NULL,
    region_hint text,
    soft_capacity integer NOT NULL DEFAULT 180,
    hard_capacity integer NOT NULL DEFAULT 250,
    member_count_projection integer NOT NULL DEFAULT 0,
    activity_score numeric(12, 4) NOT NULL DEFAULT 0,
    state text NOT NULL DEFAULT 'provisioning',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT room_instance_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT room_instance_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    ),
    CONSTRAINT room_instance_region_length CHECK (
        region_hint IS NULL OR length(region_hint) BETWEEN 1 AND 64
    ),
    CONSTRAINT room_instance_capacity CHECK (
        soft_capacity > 0 AND hard_capacity > soft_capacity AND hard_capacity <= 1000
    ),
    CONSTRAINT room_instance_member_count CHECK (
        member_count_projection BETWEEN 0 AND hard_capacity
    ),
    CONSTRAINT room_instance_activity_score CHECK (activity_score >= 0),
    CONSTRAINT room_instance_state CHECK (
        state IN ('provisioning', 'active', 'draining', 'archived', 'failed')
    ),
    CONSTRAINT room_instance_timestamp_order CHECK (updated_at >= created_at),
    CONSTRAINT room_instance_version_nonnegative CHECK (version >= 0),
    CONSTRAINT room_instance_matrix_room_id_unique UNIQUE (matrix_room_id)
);

CREATE INDEX room_instance_allocation_idx
    ON agent_room.room_instance (catalog_entry_id, state, member_count_projection);

CREATE TABLE agent_room.room_membership_projection (
    room_instance_id uuid NOT NULL REFERENCES agent_room.room_instance(id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    matrix_membership text NOT NULL,
    power_level integer NOT NULL DEFAULT 0,
    last_event_id text NOT NULL,
    projected_at timestamptz NOT NULL,
    PRIMARY KEY (room_instance_id, agent_id),
    CONSTRAINT room_membership_projection_membership CHECK (
        matrix_membership IN ('invite', 'join', 'leave', 'ban', 'knock')
    ),
    CONSTRAINT room_membership_projection_power_level CHECK (power_level BETWEEN -100 AND 100),
    CONSTRAINT room_membership_projection_event_length CHECK (
        length(last_event_id) BETWEEN 4 AND 512
    )
);

CREATE INDEX room_membership_projection_agent_idx
    ON agent_room.room_membership_projection (agent_id, matrix_membership);

CREATE TABLE agent_room.content_object (
    id uuid PRIMARY KEY,
    owner_principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    storage_key text NOT NULL,
    sha256_digest bytea NOT NULL,
    byte_length bigint NOT NULL,
    media_type text NOT NULL,
    encryption_mode text NOT NULL,
    scan_state text NOT NULL,
    lifecycle_state text NOT NULL,
    expires_at timestamptz,
    created_at timestamptz NOT NULL,
    deleted_at timestamptz,
    CONSTRAINT content_object_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT content_object_storage_key_length CHECK (length(storage_key) BETWEEN 16 AND 1024),
    CONSTRAINT content_object_digest_length CHECK (octet_length(sha256_digest) = 32),
    CONSTRAINT content_object_byte_length CHECK (byte_length BETWEEN 0 AND 26214400),
    CONSTRAINT content_object_media_type_length CHECK (length(media_type) BETWEEN 3 AND 255),
    CONSTRAINT content_object_encryption_mode CHECK (
        encryption_mode IN ('server_side', 'client_e2ee')
    ),
    CONSTRAINT content_object_scan_state CHECK (
        scan_state IN ('pending', 'clean', 'suspicious', 'rejected', 'not_applicable')
    ),
    CONSTRAINT content_object_lifecycle_state CHECK (
        lifecycle_state IN ('uploading', 'active', 'orphaned', 'redacted', 'expired', 'deleted')
    ),
    CONSTRAINT content_object_expiry_order CHECK (
        expires_at IS NULL OR expires_at > created_at
    ),
    CONSTRAINT content_object_deletion_consistency CHECK (
        (lifecycle_state = 'deleted' AND deleted_at IS NOT NULL)
        OR (lifecycle_state <> 'deleted' AND deleted_at IS NULL)
    ),
    CONSTRAINT content_object_storage_key_unique UNIQUE (storage_key)
);

ALTER TABLE agent_room.principal
    ADD CONSTRAINT principal_avatar_content_fk
    FOREIGN KEY (avatar_content_id) REFERENCES agent_room.content_object(id) ON DELETE SET NULL;

ALTER TABLE agent_room.agent
    ADD CONSTRAINT agent_avatar_content_fk
    FOREIGN KEY (avatar_content_id) REFERENCES agent_room.content_object(id) ON DELETE SET NULL;

CREATE INDEX content_object_reclamation_idx
    ON agent_room.content_object (lifecycle_state, expires_at);

CREATE TABLE agent_room.content_access_policy (
    id uuid PRIMARY KEY,
    content_id uuid NOT NULL REFERENCES agent_room.content_object(id),
    matrix_room_id text NOT NULL,
    matrix_event_id text,
    access_mode text NOT NULL,
    created_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CONSTRAINT content_access_policy_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT content_access_policy_room_length CHECK (length(matrix_room_id) BETWEEN 4 AND 512),
    CONSTRAINT content_access_policy_event_length CHECK (
        matrix_event_id IS NULL OR length(matrix_event_id) BETWEEN 4 AND 512
    ),
    CONSTRAINT content_access_policy_mode CHECK (
        access_mode IN ('room_member', 'sender_only', 'moderator')
    ),
    CONSTRAINT content_access_policy_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE TABLE agent_room.context_handoff (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    target_agent_instance_id uuid NOT NULL REFERENCES agent_room.agent_instance(id),
    source_matrix_room_id text NOT NULL,
    source_matrix_event_id text NOT NULL,
    content_id uuid NOT NULL REFERENCES agent_room.content_object(id),
    allowed_purpose text NOT NULL,
    state text NOT NULL DEFAULT 'proposed',
    approved_at timestamptz,
    delivered_at timestamptz,
    consumed_at timestamptz,
    expires_at timestamptz NOT NULL,
    failure_code text,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT context_handoff_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT context_handoff_source_room_length CHECK (
        length(source_matrix_room_id) BETWEEN 4 AND 512
    ),
    CONSTRAINT context_handoff_source_event_length CHECK (
        length(source_matrix_event_id) BETWEEN 4 AND 512
    ),
    CONSTRAINT context_handoff_purpose CHECK (
        allowed_purpose IN ('inspect', 'summarize', 'reply_draft')
    ),
    CONSTRAINT context_handoff_state CHECK (
        state IN ('proposed', 'approved', 'delivered', 'consumed', 'declined', 'revoked', 'expired', 'failed')
    ),
    CONSTRAINT context_handoff_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    ),
    CONSTRAINT context_handoff_version_nonnegative CHECK (version >= 0)
);

CREATE UNIQUE INDEX context_handoff_active_unique
    ON agent_room.context_handoff (
        principal_id,
        target_agent_instance_id,
        source_matrix_event_id,
        content_id,
        allowed_purpose
    )
    WHERE state IN ('proposed', 'approved', 'delivered');

CREATE INDEX context_handoff_delivery_idx
    ON agent_room.context_handoff (target_agent_instance_id, state, expires_at);

CREATE TABLE agent_room.automation_grant (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    agent_instance_id uuid REFERENCES agent_room.agent_instance(id),
    room_catalog_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    allowed_message_kinds text[] NOT NULL,
    max_messages_per_minute integer NOT NULL,
    max_total_messages integer,
    allow_unknown_recipients boolean NOT NULL DEFAULT false,
    starts_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    state text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL,
    revoked_at timestamptz,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT automation_grant_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT automation_grant_message_kinds_nonempty CHECK (
        cardinality(allowed_message_kinds) > 0
    ),
    CONSTRAINT automation_grant_rate CHECK (max_messages_per_minute BETWEEN 1 AND 600),
    CONSTRAINT automation_grant_total CHECK (
        max_total_messages IS NULL OR max_total_messages > 0
    ),
    CONSTRAINT automation_grant_time_order CHECK (
        expires_at > starts_at AND created_at <= expires_at
    ),
    CONSTRAINT automation_grant_state CHECK (
        state IN ('active', 'revoked', 'exhausted', 'expired')
    ),
    CONSTRAINT automation_grant_revocation_consistency CHECK (
        (state = 'revoked' AND revoked_at IS NOT NULL)
        OR (state <> 'revoked' AND revoked_at IS NULL)
    ),
    CONSTRAINT automation_grant_version_nonnegative CHECK (version >= 0)
);

CREATE INDEX automation_grant_scope_idx
    ON agent_room.automation_grant (agent_id, room_catalog_id, state, expires_at);

CREATE TABLE agent_room.moderation_case (
    id uuid PRIMARY KEY,
    reporter_principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    target_kind text NOT NULL,
    target_reference text NOT NULL,
    reason_code text NOT NULL,
    description text NOT NULL DEFAULT '',
    state text NOT NULL DEFAULT 'open',
    created_at timestamptz NOT NULL,
    resolved_at timestamptz,
    CONSTRAINT moderation_case_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT moderation_case_target_kind CHECK (
        target_kind IN ('principal', 'agent', 'room', 'event', 'federation_peer')
    ),
    CONSTRAINT moderation_case_target_length CHECK (length(target_reference) BETWEEN 1 AND 1024),
    CONSTRAINT moderation_case_reason_length CHECK (length(reason_code) BETWEEN 1 AND 128),
    CONSTRAINT moderation_case_description_length CHECK (length(description) <= 4096),
    CONSTRAINT moderation_case_state CHECK (state IN ('open', 'in_review', 'resolved', 'dismissed')),
    CONSTRAINT moderation_case_resolution_consistency CHECK (
        (state IN ('resolved', 'dismissed') AND resolved_at IS NOT NULL)
        OR (state IN ('open', 'in_review') AND resolved_at IS NULL)
    )
);

CREATE INDEX moderation_case_queue_idx
    ON agent_room.moderation_case (state, created_at);

CREATE TABLE agent_room.moderation_action (
    id uuid PRIMARY KEY,
    case_id uuid REFERENCES agent_room.moderation_case(id),
    actor_principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    action_type text NOT NULL,
    target_reference text NOT NULL,
    reason_code text NOT NULL,
    starts_at timestamptz NOT NULL,
    expires_at timestamptz,
    reversed_at timestamptz,
    CONSTRAINT moderation_action_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT moderation_action_type CHECK (
        action_type IN ('hide', 'mute', 'kick', 'ban', 'suspend', 'block_peer')
    ),
    CONSTRAINT moderation_action_target_length CHECK (
        length(target_reference) BETWEEN 1 AND 1024
    ),
    CONSTRAINT moderation_action_reason_length CHECK (length(reason_code) BETWEEN 1 AND 128),
    CONSTRAINT moderation_action_expiry_order CHECK (
        expires_at IS NULL OR expires_at > starts_at
    ),
    CONSTRAINT moderation_action_reversal_order CHECK (
        reversed_at IS NULL OR reversed_at >= starts_at
    )
);

CREATE TABLE agent_room.audit_event (
    id uuid PRIMARY KEY,
    occurred_at timestamptz NOT NULL,
    actor_kind text NOT NULL,
    actor_reference text NOT NULL,
    action text NOT NULL,
    target_kind text NOT NULL,
    target_reference text NOT NULL,
    outcome text NOT NULL,
    reason_code text,
    correlation_id uuid NOT NULL,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    CONSTRAINT audit_event_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT audit_event_actor_kind CHECK (
        actor_kind IN ('principal', 'agent_instance', 'service', 'admin')
    ),
    CONSTRAINT audit_event_actor_length CHECK (length(actor_reference) BETWEEN 1 AND 512),
    CONSTRAINT audit_event_action_length CHECK (length(action) BETWEEN 1 AND 128),
    CONSTRAINT audit_event_target_kind_length CHECK (length(target_kind) BETWEEN 1 AND 128),
    CONSTRAINT audit_event_target_length CHECK (length(target_reference) BETWEEN 1 AND 1024),
    CONSTRAINT audit_event_outcome CHECK (outcome IN ('allowed', 'denied', 'failed')),
    CONSTRAINT audit_event_reason_length CHECK (
        reason_code IS NULL OR length(reason_code) BETWEEN 1 AND 128
    ),
    CONSTRAINT audit_event_correlation_id_v7 CHECK (
        substring(correlation_id::text, 15, 1) = '7'
    ),
    CONSTRAINT audit_event_metadata_object CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX audit_event_time_action_idx
    ON agent_room.audit_event (occurred_at, action);

CREATE INDEX audit_event_target_idx
    ON agent_room.audit_event (target_kind, target_reference, occurred_at);

CREATE TABLE agent_room.matrix_projection_cursor (
    consumer_name text PRIMARY KEY,
    sync_token text NOT NULL,
    last_event_id text,
    health_state text NOT NULL DEFAULT 'healthy',
    last_error_code text,
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT matrix_projection_cursor_consumer_length CHECK (
        length(consumer_name) BETWEEN 1 AND 128
    ),
    CONSTRAINT matrix_projection_cursor_token_length CHECK (
        length(sync_token) BETWEEN 1 AND 4096
    ),
    CONSTRAINT matrix_projection_cursor_health CHECK (
        health_state IN ('healthy', 'lagging', 'failed')
    ),
    CONSTRAINT matrix_projection_cursor_error_length CHECK (
        last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 128
    ),
    CONSTRAINT matrix_projection_cursor_version_nonnegative CHECK (version >= 0)
);

CREATE TABLE agent_room.outbox_event (
    id uuid PRIMARY KEY,
    aggregate_type text NOT NULL,
    aggregate_id uuid NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    occurred_at timestamptz NOT NULL,
    published_at timestamptz,
    attempt_count integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL,
    last_error_code text,
    CONSTRAINT outbox_event_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT outbox_event_aggregate_id_v7 CHECK (
        substring(aggregate_id::text, 15, 1) = '7'
    ),
    CONSTRAINT outbox_event_aggregate_type_length CHECK (
        length(aggregate_type) BETWEEN 1 AND 128
    ),
    CONSTRAINT outbox_event_type_length CHECK (length(event_type) BETWEEN 1 AND 128),
    CONSTRAINT outbox_event_payload_object CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT outbox_event_attempt_count CHECK (attempt_count BETWEEN 0 AND 100),
    CONSTRAINT outbox_event_publish_order CHECK (
        published_at IS NULL OR published_at >= occurred_at
    ),
    CONSTRAINT outbox_event_error_length CHECK (
        last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 128
    )
);

CREATE INDEX outbox_event_pending_idx
    ON agent_room.outbox_event (next_attempt_at)
    WHERE published_at IS NULL;

CREATE FUNCTION agent_room.enforce_agent_owner()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    affected_agent_id uuid;
BEGIN
    IF TG_TABLE_NAME = 'agent' THEN
        affected_agent_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    ELSE
        affected_agent_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.agent_id ELSE NEW.agent_id END;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM agent_room.agent
        WHERE id = affected_agent_id AND lifecycle_state = 'active'
    ) AND NOT EXISTS (
        SELECT 1
        FROM agent_room.agent_ownership
        WHERE agent_id = affected_agent_id
          AND role = 'owner'
          AND revoked_at IS NULL
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = '活跃 Agent 必须保留至少一个活跃 Owner';
    END IF;

    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER agent_requires_owner_after_state_change
AFTER INSERT OR UPDATE OR DELETE ON agent_room.agent
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION agent_room.enforce_agent_owner();

CREATE CONSTRAINT TRIGGER agent_requires_owner_after_ownership_change
AFTER INSERT OR UPDATE OR DELETE ON agent_room.agent_ownership
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION agent_room.enforce_agent_owner();

CREATE FUNCTION agent_room.reject_audit_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION USING
        ERRCODE = '55000',
        MESSAGE = '审计事件是追加写，禁止更新或删除';
END;
$$;

CREATE TRIGGER audit_event_is_append_only
BEFORE UPDATE OR DELETE ON agent_room.audit_event
FOR EACH ROW EXECUTE FUNCTION agent_room.reject_audit_mutation();

GRANT USAGE ON SCHEMA agent_room TO agent_room_runtime;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA agent_room TO agent_room_runtime;
REVOKE UPDATE, DELETE ON agent_room.audit_event FROM agent_room_runtime;

ALTER DEFAULT PRIVILEGES IN SCHEMA agent_room
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO agent_room_runtime;

REVOKE ALL ON FUNCTION agent_room.enforce_agent_owner() FROM PUBLIC;
REVOKE ALL ON FUNCTION agent_room.reject_audit_mutation() FROM PUBLIC;
