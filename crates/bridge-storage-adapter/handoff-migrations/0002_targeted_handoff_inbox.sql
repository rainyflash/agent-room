-- 云端交接收件箱只保存已领取元数据。正文始终留在内容服务，直到 Agent 明确消费。
CREATE TABLE targeted_handoff_inbox (
    handoff_id TEXT PRIMARY KEY NOT NULL,
    principal_id TEXT NOT NULL,
    source_room_id TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    source_message_id TEXT NOT NULL,
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
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    queued_at_unix_ms INTEGER NOT NULL CHECK (queued_at_unix_ms >= created_at_unix_ms),
    delivered_at_unix_ms INTEGER NOT NULL CHECK (delivered_at_unix_ms >= queued_at_unix_ms),
    expires_at_unix_ms INTEGER NOT NULL CHECK (expires_at_unix_ms > delivered_at_unix_ms),
    version INTEGER NOT NULL CHECK (version > 0)
);

CREATE INDEX targeted_handoff_inbox_target_idx
    ON targeted_handoff_inbox (target_instance_id, delivered_at_unix_ms, handoff_id);
