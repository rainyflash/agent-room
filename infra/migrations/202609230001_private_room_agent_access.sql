-- 私人房间的 Agent 口令与凭口令进来的 Agent 成员（ADR 0010）。
-- 口令只存摘要，一个房间同一时间只有一个。Agent 成员不是房间成员，它的主人也不因此成为成员。
-- 旧版控制面不读这些表，迁移可以在切换服务器之前运行。

CREATE TABLE agent_room.private_room_join_code (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.private_room_state(catalog_entry_id) ON DELETE CASCADE,
    code_digest bytea NOT NULL,
    permission_bits smallint NOT NULL,
    created_by_principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    created_at timestamptz NOT NULL,
    CONSTRAINT private_room_join_code_digest_length CHECK (octet_length(code_digest) = 32),
    CONSTRAINT private_room_join_code_digest_unique UNIQUE (code_digest),
    CONSTRAINT private_room_join_code_permission_bits CHECK (
        permission_bits BETWEEN 1 AND 31
        AND (permission_bits & 1) = 1
        AND ((permission_bits & 16) = 0 OR (permission_bits & 2) = 2)
    )
);

CREATE TABLE agent_room.private_room_agent_member (
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state(catalog_entry_id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    membership_status text NOT NULL,
    permission_bits smallint NOT NULL,
    joined_via text NOT NULL DEFAULT 'code',
    created_at timestamptz NOT NULL,
    status_changed_at timestamptz NOT NULL,
    PRIMARY KEY (catalog_entry_id, agent_id),
    CONSTRAINT private_room_agent_member_status CHECK (
        membership_status IN ('joined', 'removed')
    ),
    CONSTRAINT private_room_agent_member_joined_via CHECK (joined_via = 'code'),
    CONSTRAINT private_room_agent_member_state_permissions CHECK (
        (membership_status = 'joined' AND permission_bits BETWEEN 1 AND 31 AND (permission_bits & 1) = 1)
        OR (membership_status = 'removed' AND permission_bits = 0)
    ),
    CONSTRAINT private_room_agent_member_timestamp_order CHECK (
        status_changed_at >= created_at
    )
);

CREATE INDEX private_room_agent_member_agent_idx
    ON agent_room.private_room_agent_member (agent_id, membership_status);

-- 猜口令的固定窗口计数。调用方是本机设备（device:<id>），以后也可能是网络来源的摘要。
CREATE TABLE agent_room.join_code_attempt_window (
    caller_key text PRIMARY KEY,
    window_started_at timestamptz NOT NULL,
    failure_count integer NOT NULL,
    CONSTRAINT join_code_attempt_window_caller_length CHECK (length(caller_key) BETWEEN 1 AND 200),
    CONSTRAINT join_code_attempt_window_count_nonnegative CHECK (failure_count >= 0)
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_join_code,
       agent_room.private_room_agent_member,
       agent_room.join_code_attempt_window
    TO agent_room_runtime;
