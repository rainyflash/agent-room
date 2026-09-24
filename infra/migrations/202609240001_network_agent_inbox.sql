-- 网络 Agent 收消息（ADR 0010，2-收发）：服务器用 Agent 自己的 Matrix 会话同步房间，验签后把消息预览
-- 放进它的收件箱。只有 Agent 显式确认，收件箱才往前走；断线或控制面重启后，没确认的消息还在。
-- 只加列和表，旧版控制面不读它们，迁移可以在切换服务器之前运行。

ALTER TABLE agent_room.network_agent
    ADD COLUMN sync_token text,
    ADD COLUMN inbox_sequence bigint NOT NULL DEFAULT 0,
    ADD COLUMN acked_sequence bigint NOT NULL DEFAULT 0,
    ADD COLUMN dropped_messages bigint NOT NULL DEFAULT 0,
    ADD CONSTRAINT network_agent_sync_token_length CHECK (
        sync_token IS NULL OR length(sync_token) BETWEEN 1 AND 1024
    ),
    ADD CONSTRAINT network_agent_inbox_order CHECK (
        acked_sequence >= 0 AND acked_sequence <= inbox_sequence
    ),
    ADD CONSTRAINT network_agent_dropped_nonnegative CHECK (dropped_messages >= 0);

-- 网络 Agent 进过的房间：它在哪些房间收消息、往哪儿发言。
CREATE TABLE agent_room.network_agent_room (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    matrix_room_id text NOT NULL,
    catalog_entry_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    joined_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, matrix_room_id),
    CONSTRAINT network_agent_room_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    )
);

-- 验签通过、还没确认的消息预览，按到达顺序编号。确认到哪一条，那一条及之前的就删掉。
CREATE TABLE agent_room.network_agent_inbox (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    sequence bigint NOT NULL,
    matrix_event_id text NOT NULL,
    matrix_room_id text NOT NULL,
    message_id uuid NOT NULL,
    -- 作者：Agent ID 或 human:<Matrix 用户>。编辑和撤回只认同一作者。
    actor_key text NOT NULL,
    preview jsonb NOT NULL,
    received_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, sequence),
    CONSTRAINT network_agent_inbox_event_unique UNIQUE (network_agent_id, matrix_event_id),
    CONSTRAINT network_agent_inbox_sequence_positive CHECK (sequence > 0),
    CONSTRAINT network_agent_inbox_event_id_length CHECK (length(matrix_event_id) BETWEEN 2 AND 255),
    CONSTRAINT network_agent_inbox_actor_key_length CHECK (length(actor_key) BETWEEN 1 AND 600),
    CONSTRAINT network_agent_inbox_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    ),
    CONSTRAINT network_agent_inbox_preview_object CHECK (jsonb_typeof(preview) = 'object'),
    CONSTRAINT network_agent_inbox_preview_size CHECK (octet_length(preview::text) <= 65536)
);

-- 修订（编辑、撤回）按消息 ID 找到还没确认的那一条。
CREATE INDEX network_agent_inbox_message_idx
    ON agent_room.network_agent_inbox (network_agent_id, message_id);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.network_agent_room,
       agent_room.network_agent_inbox
    TO agent_room_runtime;
