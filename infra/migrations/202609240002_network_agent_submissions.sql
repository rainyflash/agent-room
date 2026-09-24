-- 网络 Agent 发言（ADR 0010，2-收发）：每次发言按提交 ID 记一条，重试时回到同一条，Matrix 事务 ID
-- 也跟着固定，所以不会重复发送。状态与本机 Bridge 的提交记录一致。
-- 只加表，旧版控制面不读它，迁移可以在切换服务器之前运行。

CREATE TABLE agent_room.network_agent_submission (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    submission_id uuid NOT NULL,
    kind text NOT NULL,
    fingerprint bytea NOT NULL,
    transaction_id text NOT NULL,
    state text NOT NULL,
    event_id text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, submission_id),
    CONSTRAINT network_agent_submission_transaction_unique UNIQUE (network_agent_id, transaction_id),
    CONSTRAINT network_agent_submission_id_v7 CHECK (substring(submission_id::text, 15, 1) = '7'),
    CONSTRAINT network_agent_submission_kind CHECK (kind IN ('preview', 'replace', 'redact')),
    CONSTRAINT network_agent_submission_state CHECK (
        state IN ('claimed', 'submit_unknown', 'accepted', 'bound')
    ),
    CONSTRAINT network_agent_submission_fingerprint_length CHECK (octet_length(fingerprint) = 32),
    CONSTRAINT network_agent_submission_transaction_length CHECK (
        length(transaction_id) BETWEEN 1 AND 255
    ),
    CONSTRAINT network_agent_submission_event CHECK (
        (state IN ('accepted', 'bound')) = (event_id IS NOT NULL)
    ),
    CONSTRAINT network_agent_submission_event_length CHECK (
        event_id IS NULL OR length(event_id) BETWEEN 2 AND 255
    ),
    CONSTRAINT network_agent_submission_time_order CHECK (updated_at >= created_at)
);

GRANT SELECT, INSERT, UPDATE, DELETE ON agent_room.network_agent_submission TO agent_room_runtime;
