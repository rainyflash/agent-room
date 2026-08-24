-- 不可变事件日志按本机观察顺序编号，客户端声明时间只保留用于展示和审计。
CREATE TABLE message_projection_event (
    event_id TEXT PRIMARY KEY NOT NULL,
    room_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('preview', 'revision')),
    message_id TEXT NOT NULL,
    revision_id TEXT,
    revision_kind TEXT CHECK (revision_kind IN ('replace', 'redact', 'moderate')),
    created_at_unix_ms INTEGER NOT NULL,
    origin_server_timestamp INTEGER,
    transaction_id TEXT,
    actor_agent_id TEXT NOT NULL,
    actor_json TEXT NOT NULL CHECK (json_valid(actor_json)),
    preview_json TEXT CHECK (preview_json IS NULL OR json_valid(preview_json)),
    content_json TEXT CHECK (content_json IS NULL OR json_valid(content_json)),
    relation_target_message_id TEXT,
    UNIQUE (room_id, sequence),
    CHECK (
        (event_kind = 'preview' AND revision_id IS NULL AND revision_kind IS NULL
            AND preview_json IS NOT NULL AND content_json IS NOT NULL)
        OR
        (event_kind = 'revision' AND revision_id IS NOT NULL AND revision_kind IS NOT NULL
            AND (
                (revision_kind = 'replace' AND preview_json IS NOT NULL AND content_json IS NOT NULL)
                OR (revision_kind IN ('redact', 'moderate') AND preview_json IS NULL AND content_json IS NULL)
            ))
    )
);

CREATE INDEX message_projection_target_idx
    ON message_projection_event (room_id, message_id, sequence);

CREATE TABLE message_current_projection (
    message_id TEXT PRIMARY KEY NOT NULL,
    room_id TEXT NOT NULL,
    base_event_id TEXT NOT NULL UNIQUE,
    first_sequence INTEGER NOT NULL CHECK (first_sequence > 0),
    last_sequence INTEGER NOT NULL CHECK (last_sequence >= first_sequence),
    created_at_unix_ms INTEGER NOT NULL,
    origin_server_timestamp INTEGER,
    actor_agent_id TEXT NOT NULL,
    actor_json TEXT NOT NULL CHECK (json_valid(actor_json)),
    preview_json TEXT NOT NULL CHECK (json_valid(preview_json)),
    content_json TEXT CHECK (content_json IS NULL OR json_valid(content_json)),
    relation_target_message_id TEXT,
    visibility TEXT NOT NULL CHECK (visibility IN ('active', 'redacted')),
    last_revision_event_id TEXT
);

CREATE INDEX message_current_room_order_idx
    ON message_current_projection (room_id, first_sequence);

CREATE TABLE message_sync_issue (
    sync_token TEXT NOT NULL,
    room_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    PRIMARY KEY (sync_token, room_id, event_id, reason)
);

CREATE TABLE message_timeline_gap (
    sync_token TEXT NOT NULL,
    room_id TEXT NOT NULL,
    previous_batch TEXT NOT NULL,
    PRIMARY KEY (sync_token, room_id, previous_batch)
);

CREATE TABLE message_sync_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    next_batch TEXT NOT NULL
);
